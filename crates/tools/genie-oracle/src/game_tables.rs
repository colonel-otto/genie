//! The game's own tables beside the `.dat`: the linked technologies, the unit lines and the
//! drop sites.

/// The game's linked technologies (`resources/_common/dat/linkedTechs.json`): each group's
/// technologies with the group's kind. A technology of a kind the game does not know, or out of
/// the ids' range, is left out.
pub fn linked_techs(
    json: &str,
) -> Result<std::collections::BTreeMap<i16, genie_core::technology::Linked>, serde_json::Error> {
    let file: genie_json::LinkedTechs = serde_json::from_str(json)?;
    Ok(file
        .linked_techs
        .iter()
        .filter_map(|g| genie_core::technology::Linked::of(&g.kind).map(|k| (g, k)))
        .flat_map(|(g, k)| {
            g.techs
                .iter()
                .filter_map(move |&t| i16::try_from(t).ok().map(|t| (t, k)))
        })
        .collect())
}

/// What makes `unitlines.json` out of shape for [`unit_lines`].
#[derive(Debug)]
pub enum UnitLinesError {
    /// The file is not JSON.
    Json(serde_json::Error),
    /// A value of a kind the builder does not read: the entry's index and the key.
    Value(usize, &'static str),
}

impl std::fmt::Display for UnitLinesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(e) => write!(f, "{e}"),
            Self::Value(i, key) => write!(f, "UnitLines[{i}]: {key} is not what the game reads"),
        }
    }
}

impl std::error::Error for UnitLinesError {}

/// The game's unit lines (`resources/_common/dat/unitlines.json`) as `FUN_140fde680` reads each
/// entry of `UnitLines`: `LineID` (an int), `IDChain` (an array of ints), `Building` (a bool,
/// false when absent) and `Identifier` (a string or an array of strings). A key an entry leaves
/// out stays out ([`genie_core::lines::UnitLines::build`] skips the entry); a file without
/// `UnitLines` holds none.
///
/// # Errors
///
/// A file that is not JSON, or a value of another kind than the game reads.
pub fn unit_lines(json: &str) -> Result<Vec<genie_core::lines::UnitLine>, UnitLinesError> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(UnitLinesError::Json)?;
    let Some(entries) = v.get("UnitLines").and_then(serde_json::Value::as_array) else {
        return Ok(Vec::new());
    };
    let int = |i: usize, key: &'static str, v: &serde_json::Value| {
        v.as_i64()
            .and_then(|n| i32::try_from(n).ok())
            .ok_or(UnitLinesError::Value(i, key))
    };
    let text = |i: usize, v: &serde_json::Value| {
        v.as_str()
            .map(str::to_owned)
            .ok_or(UnitLinesError::Value(i, "Identifier"))
    };
    entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let line = e.get("LineID").map(|v| int(i, "LineID", v)).transpose()?;
            let chain = e
                .get("IDChain")
                .map(|v| {
                    v.as_array()
                        .ok_or(UnitLinesError::Value(i, "IDChain"))?
                        .iter()
                        .map(|id| int(i, "IDChain", id))
                        .collect::<Result<Vec<_>, _>>()
                })
                .transpose()?;
            let building = match e.get("Building") {
                None => false,
                Some(b) => b.as_bool().ok_or(UnitLinesError::Value(i, "Building"))?,
            };
            let identifiers = match e.get("Identifier") {
                None => Vec::new(),
                Some(serde_json::Value::Array(names)) => names
                    .iter()
                    .map(|n| text(i, n))
                    .collect::<Result<Vec<_>, _>>()?,
                Some(name) => vec![text(i, name)?],
            };
            Ok(genie_core::lines::UnitLine {
                line,
                chain,
                building,
                identifiers,
            })
        })
        .collect()
}

/// The game's drop-site table (`resources/_common/dat/dropsites.json`): each entry's
/// `building_id`, `worker_object_group` (−1 when absent), `worker_object_list`, `target_civs`,
/// its targets' `attribute_type`s and its `update_ai_resource_types`, in file order.
pub fn drop_sites(json: &str) -> Result<Vec<genie_core::tasks::DropSite>, serde_json::Error> {
    let v: serde_json::Value = serde_json::from_str(json)?;
    let num = |v: &serde_json::Value| v.as_i64().unwrap_or(-1);
    let list = |e: &serde_json::Value, key: &str| -> Vec<i64> {
        e.get(key)
            .and_then(|l| l.as_array())
            .map_or_else(Vec::new, |l| l.iter().map(num).collect())
    };
    Ok(v["drop_site_list"]
        .as_array()
        .map_or_else(Vec::new, |entries| {
            entries
                .iter()
                .map(|e| genie_core::tasks::DropSite {
                    building: num(&e["building_id"]) as i16,
                    worker_class: e.get("worker_object_group").map_or(-1, num) as i16,
                    workers: list(e, "worker_object_list")
                        .into_iter()
                        .map(|w| w as i16)
                        .collect(),
                    civs: list(e, "target_civs")
                        .into_iter()
                        .map(|c| c as u8)
                        .collect(),
                    auto_work: e.get("auto_work").and_then(serde_json::Value::as_bool)
                        != Some(false),
                    targets: e["target_list"].as_array().map_or_else(Vec::new, |ts| {
                        ts.iter()
                            .map(|t| {
                                let field = |key: &str| t.get(key).map_or(-1, num);
                                genie_core::tasks::DropTarget {
                                    class: field("object_group") as i16,
                                    workers: field("worker_num") as i32,
                                    state: field("object_state") as i32,
                                    attribute: field("attribute_type") as i16,
                                }
                            })
                            .collect()
                    }),
                    livestock: e.get("accepts_livestock").is_some(),
                    resource_types: e["update_ai_resource_types"].as_array().map_or_else(
                        Vec::new,
                        |ts| {
                            ts.iter()
                                .map(|t| t.get("resource_type").map_or(-1, num) as i32)
                                .collect()
                        },
                    ),
                })
                .collect()
        }))
}

//! Where two object lists differ, object by object and field by field ([`Difference`]).

use crate::{MoveRow, ObjectRow, Same};

/// One field of one object where the slice's row and the game's save differ
/// ([`object_field_differences`]); its text is [`object_differences`]' line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Difference {
    /// The object's id.
    pub id: i32,
    /// Its master: the game's, or the slice's for an object only the slice holds.
    pub master: i16,
    /// The field's name, or [`ONLY_IN_THE_GAME`], [`ONLY_IN_THE_SLICE`].
    pub field: String,
    /// The slice's value (its whole row for an object only the slice holds).
    pub slice: String,
    /// The game's value (its whole row for an object only the game holds).
    pub game: String,
}

/// [`Difference::field`] of an object the game's save holds and the slice does not.
pub const ONLY_IN_THE_GAME: &str = "only in the game";
/// [`Difference::field`] of an object the slice holds and the game's save does not.
pub const ONLY_IN_THE_SLICE: &str = "only in the slice";

impl std::fmt::Display for Difference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.field.as_str() {
            ONLY_IN_THE_GAME => write!(f, "{}: {} ({})", self.id, self.field, self.game),
            ONLY_IN_THE_SLICE => write!(f, "{}: {} ({})", self.id, self.field, self.slice),
            _ => write!(
                f,
                "{}: {} slice {} game {}",
                self.id, self.field, self.slice, self.game
            ),
        }
    }
}

/// Where two object lists differ, one line per object and field ([`object_field_differences`]).
#[must_use]
pub fn object_differences(slice: &[ObjectRow], game: &[ObjectRow]) -> Vec<String> {
    object_field_differences(slice, game)
        .iter()
        .map(ToString::to_string)
        .collect()
}

/// Where two object lists differ, one [`Difference`] per object and field: `slice` from the
/// slice, `game` from the game's save, matched by id, in the game's order, then the objects only
/// the slice holds. Equal lists give none.
#[must_use]
pub fn object_field_differences(slice: &[ObjectRow], game: &[ObjectRow]) -> Vec<Difference> {
    let mut out = Vec::new();
    // the first slice row of each id, as a search from the front finds it
    let mut by_id = std::collections::HashMap::with_capacity(slice.len());
    for s in slice {
        let _ = by_id.entry(s.id).or_insert(s);
    }
    for g in game {
        let Some(&s) = by_id.get(&g.id) else {
            out.push(Difference {
                id: g.id,
                master: g.master,
                field: ONLY_IN_THE_GAME.to_owned(),
                slice: String::new(),
                game: format!("{g:?}"),
            });
            continue;
        };
        // a row the same to the bit has no field that differs: skip the fields' texts
        if s.same(g) {
            continue;
        }
        let mut field = |name: &str, a: String, b: String| {
            if a != b {
                out.push(Difference {
                    id: g.id,
                    master: g.master,
                    field: name.to_owned(),
                    slice: a,
                    game: b,
                });
            }
        };
        field("master", s.master.to_string(), g.master.to_string());
        field("state", s.state.to_string(), g.state.to_string());
        field("hp", format!("{:?}", s.hp), format!("{:?}", g.hp));
        field(
            "amount",
            format!("{:?}", s.amount),
            format!("{:?}", g.amount),
        );
        field(
            "position",
            format!("{:?}", s.position),
            format!("{:?}", g.position),
        );
        field(
            "action",
            format!("{:?}", s.action),
            format!("{:?}", g.action),
        );
        if s.action_target.is_some() && s.action == g.action {
            field(
                "action target",
                format!("{:?}", s.action_target),
                format!("{:?}", g.action_target),
            );
        }
        field(
            "retarget",
            format!("{:?}", s.retarget),
            format!("{:?}", g.retarget),
        );
        field(
            "construction",
            format!("{:?}", s.construction),
            format!("{:?}", g.construction),
        );
        field(
            "build added",
            format!("{:?}", s.build_added),
            format!("{:?}", g.build_added),
        );
        field(
            "last human order time",
            s.human_order.to_string(),
            g.human_order.to_string(),
        );
        field(
            "gather point",
            format!("{:?}", s.gather),
            format!("{:?}", g.gather),
        );
        field(
            "charge, charge hit, hp powerups",
            format!("{:?}", s.combat),
            format!("{:?}", g.combat),
        );
        field("gate", format!("{:?}", s.gate), format!("{:?}", g.gate));
        field(
            "town bell",
            format!("{:?}", s.town_bell),
            format!("{:?}", g.town_bell),
        );
        field(
            "next volley, reload world time",
            format!("{:?}", s.volley),
            format!("{:?}", g.volley),
        );
        field(
            "unit group",
            format!("{:?}", s.group),
            format!("{:?}", g.group),
        );
        field(
            "production action (state, progress)",
            format!("{:?}", s.making),
            format!("{:?}", g.making),
        );
        match (&s.ai, &g.ai) {
            (Some(a), Some(b)) => {
                for ((name, x), (_, y)) in a.fields.iter().zip(&b.fields) {
                    field(name, x.clone(), y.clone());
                }
            }
            (a, b) => field(
                "unit AI",
                format!("{}", a.is_some()),
                format!("{}", b.is_some()),
            ),
        }
        match (&s.movement, &g.movement) {
            (Some(a), Some(b)) => {
                field(
                    "forward",
                    format!("{:?}", a.forward),
                    format!("{:?}", b.forward),
                );
                field("right", format!("{:?}", a.right), format!("{:?}", b.right));
                field("angle", format!("{:?}", a.angle), format!("{:?}", b.angle));
                field(
                    "last turn",
                    format!("{:?}", a.last_turn),
                    format!("{:?}", b.last_turn),
                );
                field(
                    "velocity",
                    format!("{:?}", a.velocity),
                    format!("{:?}", b.velocity),
                );
                field(
                    "path levels",
                    a.paths.len().to_string(),
                    b.paths.len().to_string(),
                );
                for (i, (pa, pb)) in a.paths.iter().zip(&b.paths).enumerate() {
                    field(
                        &format!("path level {i}"),
                        format!("{pa:?}"),
                        format!("{pb:?}"),
                    );
                }
                let bytes = |m: &MoveRow| {
                    format!(
                        "{:?}",
                        (
                            m.waiting,
                            m.wait_delays,
                            m.attack_moving,
                            m.substitute_count
                        )
                    )
                };
                field("move bytes", bytes(a), bytes(b));
                field(
                    "last move",
                    a.last_move.to_string(),
                    b.last_move.to_string(),
                );
                field(
                    "ignore units",
                    format!("{:?}", a.ignore_units),
                    format!("{:?}", b.ignore_units),
                );
            }
            // a building's movement part without velocity data or paths is not modelled
            (None, Some(b)) if b.velocity.is_none() && b.paths.is_empty() => {}
            (a, b) => field(
                "movement",
                format!("{:?}", a.is_some()),
                format!("{:?}", b.is_some()),
            ),
        }
    }
    for s in slice.iter().filter(|s| !game.iter().any(|g| g.id == s.id)) {
        out.push(Difference {
            id: s.id,
            master: s.master,
            field: ONLY_IN_THE_SLICE.to_owned(),
            slice: format!("{s:?}"),
            game: String::new(),
        });
    }
    out
}

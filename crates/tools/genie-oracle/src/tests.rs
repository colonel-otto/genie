use super::*;
use genie_core::objects::actions::ActionList;

/// Word 9 reads a building's production action's state over its top action's (records
/// MarvinBlue/426048706's town centre 7150 after update 7499: its Attack in state 7 and its
/// make action in state 6 count 9 + 6); its type stays the top action's, −1 without one.
#[test]
fn a_building_s_production_state_comes_before_its_top_action_s() {
    let artifact = genie_core::ownership::artifact::Artifact {
        state: 10,
        timer: 0.0,
        gaia: false,
        move_to: None,
        goal: None,
        target: -1,
        enter: None,
        moving: None,
    };
    let making = genie_core::production::Production {
        action: Some(genie_core::production::Making::Unit(
            genie_core::production::MakeObj {
                state: 6,
                paid: true,
                master: 83,
                progress: 0.5,
                unique_id: -1,
                button: 0,
                train_time: 25,
            },
        )),
        ..genie_core::production::Production::default()
    };
    let kind = ARTIFACT_TYPE;
    let both = Unit {
        actions: ActionList::default().with_artifact(Some(artifact)),
        production: Some(making.clone()),
        ..Unit::default()
    };
    assert_eq!(checkpoint_action(&both), Some(TopAction { kind, state: 6 }));
    let top = Unit {
        actions: ActionList::default().with_artifact(Some(artifact)),
        ..Unit::default()
    };
    assert_eq!(checkpoint_action(&top), Some(TopAction { kind, state: 10 }));
    let training = Unit {
        production: Some(making),
        ..Unit::default()
    };
    assert_eq!(
        checkpoint_action(&training),
        Some(TopAction { kind: -1, state: 6 })
    );
    assert_eq!(checkpoint_action(&Unit::default()), None);
}

/// Each differing field of a matched object is one difference with the game's master; an
/// object in one list alone is one too; the lines are the differences' texts.
#[test]
fn differences_name_the_object_its_master_and_the_field() {
    let unit = |id, master, hp| {
        let mut u = Unit {
            id,
            ..Unit::default()
        };
        u.life.master_id = master;
        u.life.hp = hp;
        u
    };
    let slice = [unit(1, 83, 25.0), unit(2, 83, 25.0), unit(4, 4, 1.0)];
    let game = [unit(1, 83, 25.0), unit(2, 293, 24.0), unit(3, 59, 1.0)];
    let d = object_field_differences(&slice, &game);
    let fields: Vec<(i32, i16, &str)> = d
        .iter()
        .map(|d| (d.id, d.master, d.field.as_str()))
        .collect();
    assert_eq!(
        fields,
        [
            (2, 293, "life.hp"),
            (2, 293, "life.master_id"),
            (3, 59, ONLY_IN_THE_GAME),
            (4, 4, ONLY_IN_THE_SLICE)
        ]
    );
    assert_eq!(d[0].to_string(), "2: life.hp slice 25.0 game 24.0");
    assert_eq!(
        d[2].to_string(),
        "3: only in the game (master 59 state 2 hp 1.0 at [0.0, 0.0, 0.0] action None)"
    );
    assert_eq!(
        object_differences(&slice, &game),
        d.iter().map(ToString::to_string).collect::<Vec<_>>()
    );
}

/// Each error says why the recording could not be compared.
#[test]
fn each_error_says_why() {
    let texts = [
        (Error::NoCheckpoint, "the recording has no checkpoint"),
        (
            Error::NoSuchPlayer(9),
            "a checkpoint names player 9, not in the save",
        ),
        (
            Error::CommandBeforeFirstSync,
            "a command comes before the first sync",
        ),
        (Error::BadMeshDump, "the mesh dump has no mesh line"),
        (Error::BadRegistryDump, "the registry dump is out of shape"),
        (Error::BadTileDump, "the tile list dump is out of shape"),
        (
            Error::BadPoolDump,
            "the recycled pools dump is out of shape",
        ),
        (Error::NoChapter, "the recording has no chapter"),
        (Error::NoPlayer(3), "no player 3 in the save"),
    ];
    for (error, text) in texts {
        assert_eq!(error.to_string(), text);
    }
    let unmodelled = Error::Unmodelled(GenieError::unsupported("a thing"));
    assert!(unmodelled.to_string().starts_with("not modelled: "));
    let truncated = de::truncated(4);
    let text = truncated.to_string();
    let rec = Error::from(truncated);
    assert!(matches!(rec, Error::Rec(_)));
    assert_eq!(rec.to_string(), text);
}

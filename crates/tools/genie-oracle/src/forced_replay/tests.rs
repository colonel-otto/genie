use crate::Dumps;
use crate::fixtures::{gaia, header, human};
use crate::forced_replay::{
    command_phase, forced_run, forced_ticks, ghost_differences, group_owner,
    owned_group_differences,
};
use crate::setup::slice;
use genie_core::groups::{UnitGroup, UnitGroups};
use genie_core::vision::ghost::Ghost;
use genie_core::world::{GameState, World};

/// Gaia and a player, without objects.
fn world() -> World {
    slice(&header(vec![gaia(Vec::new()), human(Vec::new())]), None)
}

/// A ghost of id `id`.
fn ghost(id: i32) -> Ghost {
    Ghost {
        id,
        original: None,
        original_id: id + 100,
        original_master: None,
        original_owner: 0,
        position: [1.0, 2.0, 0.0],
        tile: [1, 2],
        footprint: 0,
        bits: 0,
        cant_see: 0,
        seen: false,
        state: 2,
    }
}

#[test]
fn ghost_lists_differ_by_their_members_or_their_order() {
    let mut turns = world();
    turns.players[1].ghosts = vec![ghost(1), ghost(2)];
    let same = [Vec::new(), vec![ghost(1), ghost(2)]];
    assert_eq!(ghost_differences(&turns, &same), []);
    let order = [Vec::new(), vec![ghost(2), ghost(1)]];
    let d = ghost_differences(&turns, &order);
    assert_eq!(d.len(), 1);
    assert!(d[0].1.starts_with("ghosts in another order"), "{d:?}");
    let other = [Vec::new(), vec![ghost(1), ghost(3)]];
    let d = ghost_differences(&turns, &other);
    assert_eq!(d[0].0, 1);
    assert!(
        d[0].1.starts_with("ghosts the slice's alone [(2, 102"),
        "{d:?}"
    );
    // a player the slice does not hold has no ghosts
    let more = [Vec::new(), vec![ghost(1), ghost(2)], vec![ghost(5)]];
    assert_eq!(ghost_differences(&turns, &more)[0].0, 2);
}

/// Groups `ids`, each owned by player 1.
fn groups(ids: &[i32]) -> UnitGroups {
    UnitGroups {
        groups: ids
            .iter()
            .map(|&id| UnitGroup {
                id,
                owner: 1,
                ..UnitGroup::default()
            })
            .collect(),
        ..UnitGroups::default()
    }
}

/// A group difference goes with the group's owner, in the game or the slice; Gaia takes the
/// world's lines and an owner the world does not hold.
#[test]
fn a_group_difference_goes_with_its_owner() {
    let mut turns = world();
    turns.groups = groups(&[4]);
    let game = GameState {
        groups: Some(groups(&[3])),
        ..GameState::default()
    };
    assert_eq!(
        group_owner(&turns, &game, "group 3: state slice 0 game 1"),
        1
    );
    assert_eq!(
        group_owner(&turns, &game, "group 4: state slice 0 game 1"),
        1
    );
    assert_eq!(
        group_owner(&turns, &game, "group 5: state slice 0 game 1"),
        0
    );
    assert_eq!(
        group_owner(&turns, &game, "unit group counter slice 7 game 8"),
        0
    );
    turns.groups.groups[0].owner = 9;
    assert_eq!(group_owner(&turns, &game, "group 4: x"), 0);
}

/// Without the game's groups there is nothing to compare; with them, the globals dumped beside
/// the save go in first and their command counter is compared too.
#[test]
fn the_groups_compare_with_the_globals_beside_the_save() {
    let mut turns = world();
    turns.groups = groups(&[3]);
    let mut none = GameState::default();
    assert_eq!(
        owned_group_differences(&turns, &mut none, Some("group_commands 4\n")),
        []
    );
    let mut game = GameState {
        groups: Some(groups(&[3])),
        ..GameState::default()
    };
    assert_eq!(owned_group_differences(&turns, &mut game, None), []);
    let d = owned_group_differences(&turns, &mut game, Some("group_commands 4\n"));
    assert_eq!(game.groups.as_ref().unwrap().commands, 4);
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].0, 0);
    let mut other = GameState {
        groups: Some(groups(&[5])),
        ..GameState::default()
    };
    let d = owned_group_differences(&turns, &mut other, None);
    assert!(d.iter().any(|(p, _)| *p == 0), "{d:?}");
}

/// Bytes that are no save are the run's error, before any update.
#[test]
fn a_save_that_does_not_parse_stops_each_run() {
    let dumps = Dumps::default();
    assert!(forced_ticks(&[], &[], &[], 0, 1, None, dumps).is_err());
    assert!(command_phase(&[], (&[], None), &[], 0, None, dumps).is_err());
    let none = |_: usize| None;
    let no_dump = |_: usize, _: &str| None;
    assert!(forced_run(&[], &[], 0, 1, None, dumps, &none, &no_dump, 1).is_err());
}

/// Gaia and a player without objects, as a save file (one chapter, no ops).
fn quiet() -> Vec<u8> {
    crate::fixtures::recording(&header(vec![gaia(Vec::new()), human(Vec::new())]), &[])
}

/// A recording of the same save and `n` syncs of 50 ms.
fn syncs(n: usize) -> Vec<u8> {
    let ops: Vec<_> = (0..n).map(|_| genie_rec::de::Op::Sync { dt: 50 }).collect();
    crate::fixtures::recording(&header(vec![gaia(Vec::new()), human(Vec::new())]), &ops)
}

/// A step and a command phase against the same quiet save have nothing to report.
#[test]
fn a_quiet_step_has_no_differences() {
    let (save, rec) = (quiet(), syncs(3));
    let step = forced_ticks(&save, &save, &rec, 0, 1, None, Dumps::default()).unwrap();
    assert_eq!((step.start, step.ticks), (0, 1));
    assert_eq!(step.differences, []);
    let phase = command_phase(&save, (&save, None), &rec, 0, None, Dumps::default()).unwrap();
    assert_eq!(phase.ticks, 1);
    assert_eq!(phase.differences, []);
}

/// The continuous run reports each update until the saves or the recording end. The slice
/// draws the next frame seed, the fixture's saves keep 0: each update differs by it alone, and
/// the next starts from the game's seed again (forced), so it draws the same.
#[test]
fn the_continuous_run_reports_each_update_and_forces_the_games_state() {
    let (save, rec) = (quiet(), syncs(4));
    let after = |u: usize| (u <= 2).then(quiet);
    let no_dump = |_: usize, _: &str| None;
    let (reports, _) = forced_run(
        &save,
        &rec,
        0,
        3,
        None,
        Dumps::default(),
        &after,
        &no_dump,
        2,
    )
    .unwrap();
    assert_eq!(reports.iter().map(|r| r.start).collect::<Vec<_>>(), [0, 1]);
    for report in &reports {
        assert_eq!(report.differences.len(), 1, "{report:?}");
        assert!(report.differences[0].1.starts_with("frame seed slice "));
        assert!(report.differences[0].1.ends_with(" game 0"));
    }
    assert_eq!(reports[0].differences, reports[1].differences);
}

use crate::task_finder::{FinderObject, FinderWorld};
use genie_core::objects::masters::{Master, Masters};
use genie_core::players::PlayerId;
use genie_core::unit_ai::task_finder::Diplomacy;

/// An object of player 1, master 83, at the main building `annex_of` if it is an annex.
fn object(annex_of: Option<i32>) -> FinderObject {
    FinderObject {
        player: PlayerId::new(1),
        master: 83,
        state: 2,
        held: (0, 5.0),
        hp: 25.0,
        max_hp: 30.0,
        attackable: true,
        pasture: -1,
        annex_of,
        top_action: Some(5),
        position: [1.0, 2.0, 0.0],
    }
}

/// Gaia and two players: player 1's relation to player 2 is `relation`.
fn world(relation: u8) -> FinderWorld {
    let master = Master {
        id: 83,
        ..Master::default()
    };
    FinderWorld {
        masters: vec![Masters::default(), Masters::new([master])],
        objects: [
            (10, object(None)),
            (11, object(Some(10))),
            (12, object(Some(11))),
        ]
        .into_iter()
        .collect(),
        relations: vec![vec![], vec![0, 0, relation], vec![]],
        attribute_95: vec![0.0, 1.5],
        civs: vec![0, 4],
        drop_sites: vec![],
    }
}

/// One's own player counts; Gaia never; relation 0 counts, 4 only with the ally flag, any
/// other or an unknown player not.
#[test]
fn allied_follows_the_relation() {
    let w = world(0);
    assert!(w.allied(PlayerId::new(1), PlayerId::new(1), false));
    assert!(!w.allied(PlayerId::new(1), PlayerId::new(0), true));
    assert!(w.allied(PlayerId::new(1), PlayerId::new(2), false));
    assert!(world(4).allied(PlayerId::new(1), PlayerId::new(2), true));
    assert!(!world(4).allied(PlayerId::new(1), PlayerId::new(2), false));
    assert!(!world(3).allied(PlayerId::new(1), PlayerId::new(2), true));
    assert!(!w.allied(PlayerId::new(5), PlayerId::new(2), true));
}

/// An annex stands for its main building, one step for each of `steps`; an object or a
/// master the world does not hold gives none.
#[test]
fn an_annex_stands_for_its_main_building() {
    let w = world(0);
    assert_eq!(w.target_up(12, 0).map(|t| t.id), Some(12));
    assert_eq!(w.target(12).map(|t| t.id), Some(11));
    assert_eq!(w.target_up(12, 5).map(|t| t.id), Some(10));
    let target = w.target(10).unwrap();
    assert_eq!(
        (target.player, target.state, target.hp),
        (PlayerId::new(1), 2, 25.0)
    );
    assert_eq!((target.pasture, target.top_action), (-1, Some(5)));
    assert!(w.target(99).is_none());
    let mut lost = world(0);
    lost.masters.truncate(1);
    assert!(lost.target(10).is_none());
}

#[test]
fn the_actor_reads_its_player_s_attribute_95_and_civ() {
    let w = world(0);
    let actor = w.actor(10).unwrap();
    assert_eq!(
        (actor.id, actor.player, actor.civ),
        (10, PlayerId::new(1), 4)
    );
    assert_eq!(actor.attribute_95.to_bits(), 1.5_f32.to_bits());
    assert!(w.actor(99).is_none());
}

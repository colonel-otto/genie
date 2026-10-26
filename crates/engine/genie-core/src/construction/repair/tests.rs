use super::*;
use crate::map::spatial::Spatial;
use crate::movement::Me;

const TASK: RepairTask = RepairTask {
    id: 7,
    work_value: 0.5,
};

/// A villager's master work rate 1 with a powerup of 0: half a hit point per second per the
/// task, each repairer past the first half of it.
#[test]
fn the_work_counts_half_for_each_repairer_past_the_first() {
    assert_eq!(work_amount(1.0, 0.5, 0.05, None), 1.0 * 0.5 * 0.05);
    assert_eq!(work_amount(1.0, 0.5, 0.05, Some(1)), 1.0 * 0.5 * 0.05);
    let alone = 1.0f32 * 0.5 * 0.05;
    assert_eq!(
        work_amount(1.0, 0.5, 0.05, Some(3)),
        alone * (((3.0 - 1.0) * 0.5 + 1.0) / 3.0)
    );
}

/// The repairer's player's attributes as a list (food, wood, stone, gold at 0 to 3).
struct Held(Vec<f32>);

impl crate::technology::Attributes for Held {
    fn get(&self, i: i16) -> Result<f32, GenieError> {
        Ok(self.0[i as usize])
    }
    fn set(&mut self, i: i16, v: f32) -> Result<(), GenieError> {
        self.0[i as usize] = v;
        Ok(())
    }
}

/// A house (70, class 3, 550 hit points, 25 wood) and a ram (35, class 13, 175 hit points,
/// 160 wood, 75 gold) of the repairer's player.
fn masters() -> crate::masters::Masters {
    crate::masters::Masters::new([
        crate::masters::Master {
            id: 70,
            kind: 80,
            class: 3,
            hit_points: 550,
            cost: vec![(1, 25, true)],
            ..crate::masters::Master::default()
        },
        crate::masters::Master {
            id: 35,
            kind: 70,
            class: 13,
            hit_points: 175,
            cost: vec![(1, 160, true), (3, 75, true)],
            ..crate::masters::Master::default()
        },
    ])
}

fn house(id: i32, hp: f32) -> Repaired {
    Repaired {
        id,
        kind: 80,
        class: 3,
        master: 70,
        master_hp: 550,
        hp,
        hp_powerups: [0.0; 2],
        main: None,
        annexes: Vec::new(),
        cost_rates: [0.5, 0.15],
    }
}

/// `FUN_1411d37d0` on a house: the amount, at most what its hit points lack, costs 0.15 of
/// its 25 wood per its 550 hit points; the last of it heals it full (1); a player without
/// the wood pays nothing and heals nothing (2); a house below 1 hit point takes nothing.
#[test]
fn a_building_s_repair_pays_its_share_of_the_cost_and_heals() {
    let masters = masters();
    let mut wood = Held(vec![0.0, 100.0, 0.0, 0.0]);
    let mut set = vec![house(1, 500.0)];
    let mut payer = Payer {
        masters: &masters,
        attributes: &mut wood,
    };
    assert_eq!(repair_building(&mut set, 0, 10.0, true, &mut payer), Ok(0));
    assert_eq!(set[0].hp, 510.0);
    let share = 0.15f32 * 10.0 / 550.0;
    assert_eq!(wood.0[1], 100.0 - 25.0 * share);
    let mut payer = Payer {
        masters: &masters,
        attributes: &mut wood,
    };
    assert_eq!(repair_building(&mut set, 0, 50.0, true, &mut payer), Ok(1));
    assert_eq!(set[0].hp, 550.0);
    let mut broke = Held(vec![0.0; 4]);
    let mut set = vec![house(1, 500.0)];
    let mut payer = Payer {
        masters: &masters,
        attributes: &mut broke,
    };
    assert_eq!(repair_building(&mut set, 0, 10.0, true, &mut payer), Ok(2));
    assert_eq!(set[0].hp, 500.0);
    let mut set = vec![house(1, 0.5)];
    assert_eq!(repair_building(&mut set, 0, 10.0, true, &mut payer), Ok(2));
}

/// From its repairer an annex's repair is its main building's, which repairs its annexes
/// (here the annex itself) by the whole amount first, then itself.
#[test]
fn an_annex_s_repair_goes_to_its_main_building_and_its_annexes() {
    let masters = masters();
    let mut wood = Held(vec![0.0, 100.0, 0.0, 0.0]);
    let mut set = vec![
        Repaired {
            main: Some(2),
            ..house(1, 500.0)
        },
        Repaired {
            annexes: vec![1],
            ..house(2, 540.0)
        },
    ];
    let mut payer = Payer {
        masters: &masters,
        attributes: &mut wood,
    };
    assert_eq!(repair_building(&mut set, 0, 20.0, true, &mut payer), Ok(1));
    assert_eq!((set[0].hp, set[1].hp), (520.0, 550.0));
}

/// `FUN_1412143b0` on a ram (class 13): the amount, at most what it lacks of its most, costs
/// its owner's unit repair rate of its costs per its most hit points; then it heals.
#[test]
fn a_siege_unit_s_repair_pays_its_unit_share() {
    let masters = masters();
    let mut held = Held(vec![0.0, 1000.0, 0.0, 1000.0]);
    let mut set = vec![Repaired {
        kind: 70,
        class: 13,
        master: 35,
        master_hp: 175,
        ..house(5, 100.0)
    }];
    let mut payer = Payer {
        masters: &masters,
        attributes: &mut held,
    };
    assert_eq!(repair_unit(&mut set, 0, 10.0, &mut payer), Ok(0));
    let share = 0.5f32 * 10.0 / 175.0;
    assert_eq!(set[0].hp, 110.0);
    assert_eq!(
        (held.0[1], held.0[3]),
        (1000.0 - 160.0 * share, 1000.0 - 75.0 * share)
    );
}

fn me() -> Me {
    Me {
        id: 9,
        position: [10.0, 10.0, 0.0],
        group: -1,
        action_state: 0,
    }
}

fn walk<'a>(world: &'a mut Spatial, mover: Option<&'a mut crate::movement::Mover>) -> Walk<'a> {
    Walk {
        mover,
        me: me(),
        dt: 0.05,
        world,
        plan: None,
    }
}

fn ward(x: f32) -> Repairee {
    Repairee {
        state: 2,
        hp: 300.0,
        owner: 1,
        allied: true,
        position: [x, 10.0, 0.0],
        radius: [1.0, 1.0],
        class: 3,
        moves: false,
    }
}

/// One update of `r` with the target `target`, the repairers `repairers`, a quick path
/// `paths` and the repair's answer `answer` (recording its amounts).
fn update(
    r: &mut RepairAction,
    target: Option<Repairee>,
    (repairers, paths, answer): (Option<u8>, bool, u8),
    amounts: &mut Vec<f32>,
) -> RepairStep {
    let mut world = Spatial::default();
    let mut mover = crate::gathering::hunt::tests::walker();
    let mut can_path = |_: &Spatial| Ok(paths);
    let mut repair = |a: f32| {
        amounts.push(a);
        Ok(answer)
    };
    let view = RepairView {
        target,
        radius: [0.2, 0.2],
        scenario: false,
        rate: 1.0,
        repairers,
        now: 0,
        waits: false,
        stance: 0,
        can_path: &mut can_path,
        repair: &mut repair,
    };
    r.update(walk(&mut world, Some(&mut mover)), view).unwrap()
}

/// A target the world no longer holds, another player's or a dead one ends the Repair
/// (`set_state(1)`: notify 0x1fa, the standing sprite); the next update removes it.
#[test]
fn a_repair_ends_with_its_target_gone_foreign_or_dead() {
    let mut amounts = Vec::new();
    for target in [
        None,
        Some(Repairee {
            allied: false,
            owner: 2,
            ..ward(11.0)
        }),
        Some(Repairee {
            state: 3,
            ..ward(11.0)
        }),
        Some(Repairee {
            hp: 0.25,
            ..ward(11.0)
        }),
    ] {
        let mut r = RepairAction::start(20, [11.0, 10.0, 0.0], TASK, 0.0);
        let step = update(&mut r, target, (None, true, 0), &mut amounts);
        assert_eq!((r.state, r.target), (DONE, -1));
        assert_eq!(step.notifies, vec![crate::unit_ai::ACTION_COMPLETED]);
        assert!(step.stands && !step.done);
        assert_eq!(step.released, target.is_some());
        let step = update(&mut r, None, (None, true, 0), &mut amounts);
        assert!(step.done);
    }
    // Gaia's object is repaired
    let mut r = RepairAction {
        state: FACE,
        move_to: None,
        ..RepairAction::start(20, [11.0, 10.0, 0.0], TASK, 0.0)
    };
    let gaia = Repairee {
        allied: false,
        owner: 0,
        ..ward(11.0)
    };
    let step = update(&mut r, Some(gaia), (Some(0), true, 0), &mut amounts);
    assert_eq!((r.state, step.repairers), (WORK, 1));
}

/// State 4 once the `MoveTo` ended: within reach the unit faces the target (6); out of reach
/// it waits (10) with a quick path to it, else fails (13), whose update posts 0x1f9.
#[test]
fn a_repairer_at_its_target_faces_it_or_waits_or_fails() {
    let mut amounts = Vec::new();
    let arrived = || RepairAction {
        move_to: Some(MoveTo::Arrived),
        ..RepairAction::start(20, [11.0, 10.0, 0.0], TASK, 0.0)
    };
    let mut r = arrived();
    let _ = update(&mut r, Some(ward(11.0)), (Some(0), true, 0), &mut amounts);
    assert_eq!((r.state, r.move_to), (FACE, None));
    let mut r = arrived();
    let step = update(&mut r, Some(ward(20.0)), (Some(0), true, 0), &mut amounts);
    assert_eq!((r.state, r.timer, step.stands), (WAIT, 1.0, true));
    let mut r = arrived();
    let _ = update(&mut r, Some(ward(20.0)), (Some(0), false, 0), &mut amounts);
    assert_eq!(r.state, FAILED);
    let step = update(&mut r, Some(ward(20.0)), (Some(0), false, 0), &mut amounts);
    assert_eq!(
        (r.state, step.notifies, step.done, step.stands),
        (HALTED, vec![crate::unit_ai::ACTION_FAILED], true, true)
    );
    assert_eq!(amounts, Vec::<f32>::new());
}

/// State 7: each update's work goes to the target's repair; full (1) the Repair ends, its
/// counted repairer out of the repairers, the armor back; the player short of resources (2)
/// posts 0x1fb and ends it too; a target that walked out of reach sends the unit walking.
#[test]
fn a_repairer_works_until_the_target_is_full_or_the_player_cannot_pay() {
    let working = || RepairAction {
        state: WORK,
        move_to: None,
        counted: true,
        ..RepairAction::start(20, [11.0, 10.0, 0.0], TASK, 0.0)
    };
    let mut amounts = Vec::new();
    let mut r = working();
    let step = update(&mut r, Some(ward(11.0)), (Some(2), true, 0), &mut amounts);
    assert_eq!((r.state, step.repairers), (WORK, 0));
    assert_eq!(amounts, vec![work_amount(1.0, 0.5, 0.05, Some(2))]);
    let step = update(&mut r, Some(ward(11.0)), (Some(2), true, 1), &mut amounts);
    assert_eq!((r.state, step.repairers, step.restored), (DONE, -1, true));
    assert_eq!(step.notifies, vec![crate::unit_ai::ACTION_COMPLETED]);
    let mut r = working();
    let step = update(&mut r, Some(ward(11.0)), (Some(1), true, 2), &mut amounts);
    assert_eq!(
        step.notifies,
        vec![
            crate::unit_ai::ACTION_INVALIDATED,
            crate::unit_ai::ACTION_COMPLETED
        ]
    );
    let mut r = working();
    let _ = update(&mut r, Some(ward(20.0)), (Some(1), true, 0), &mut amounts);
    assert_eq!(
        (r.state, r.move_to, r.counted),
        (MOVE, Some(MoveTo::Start), false)
    );
}

/// State 10 waits a second (the world's dt off the timer), then faces a target within reach
/// or waits a second again.
#[test]
fn a_waiting_repairer_looks_again_each_second() {
    let mut amounts = Vec::new();
    let mut r = RepairAction {
        state: WAIT,
        move_to: None,
        timer: 0.1,
        ..RepairAction::start(20, [11.0, 10.0, 0.0], TASK, 0.0)
    };
    let _ = update(&mut r, Some(ward(20.0)), (Some(0), true, 0), &mut amounts);
    assert_eq!((r.state, r.timer), (WAIT, 0.1 - 0.05));
    let _ = update(&mut r, Some(ward(20.0)), (Some(0), true, 0), &mut amounts);
    assert_eq!((r.state, r.timer), (WAIT, 1.0));
    r.timer = 0.0;
    let _ = update(&mut r, Some(ward(11.0)), (Some(0), true, 0), &mut amounts);
    assert_eq!(r.state, FACE);
}

/// Slot 8 (`FUN_140ad9700`): the point, the target let go (a counted repairer leaving its
/// repairers), the walk there; its end ends the Repair.
#[test]
fn a_move_sends_the_repairer_to_its_point() {
    let mut mover = crate::gathering::hunt::tests::walker();
    let mut r = RepairAction {
        state: WORK,
        move_to: None,
        counted: true,
        ..RepairAction::start(20, [11.0, 10.0, 0.0], TASK, 0.0)
    };
    assert_eq!(
        r.take_point([30.0, 5.0, 0.0], Some(&mut mover)),
        Ok((20, true))
    );
    assert_eq!(
        (r.state, r.target, r.goal, r.counted),
        (WALK_POINT, -1, Some([30.0, 5.0, 0.0]), false)
    );
    let mut amounts = Vec::new();
    r.move_to = Some(MoveTo::Arrived);
    let step = update(&mut r, None, (None, true, 0), &mut amounts);
    assert_eq!(
        (r.state, step.notifies),
        (DONE, vec![crate::unit_ai::ACTION_COMPLETED])
    );
}

/// Slot 7 (`FUN_140adaa00`, `set_state(2)`): a counted repairer is no longer counted (the
/// caller's to leave the repairers), the `MoveTo` goes, the target stays.
#[test]
fn a_halt_stops_the_repair_and_answers_whether_it_counted() {
    let mut mover = crate::gathering::hunt::tests::walker();
    let mut r = RepairAction {
        state: WORK,
        counted: true,
        ..RepairAction::start(20, [11.0, 10.0, 0.0], TASK, 0.0)
    };
    assert_eq!(r.halt(Some(&mut mover)), Ok(true));
    assert_eq!(
        (r.state, r.counted, r.move_to, r.target),
        (HALTED, false, None, 20)
    );
    assert_eq!(r.halt(None), Ok(false));
}

/// `set_state(4)` from a new target: the old `MoveTo` and point go, a new walk starts.
#[test]
fn walking_again_starts_a_new_move_to_the_target() {
    let mut mover = crate::gathering::hunt::tests::walker();
    let mut r = RepairAction {
        state: WALK_POINT,
        goal: Some([3.0, 4.0, 0.0]),
        move_to: Some(MoveTo::Arrived),
        ..RepairAction::start(20, [11.0, 10.0, 0.0], TASK, 0.0)
    };
    assert_eq!(r.walk_again(Some(&mut mover)), Ok(()));
    assert_eq!(
        (r.state, r.goal, r.move_to),
        (MOVE, None, Some(MoveTo::Start))
    );
}

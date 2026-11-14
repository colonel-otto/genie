//! The idle update of an action the list update does not run (`FUN_140a784a0`), kind by kind,
//! and of a unit's list from its step: below the head, and from the head in the inside update.

use super::*;
use crate::GenieError;
use crate::combat::AttackTargetId;
use crate::combat::attack::AttackState;
use crate::combat::attack::tests::own_attack;
use crate::combat::missile::{
    HitMode, MissileAction, MissileState, MissileTargetId, Origin, ShooterId, StrikeResult,
};
use crate::construction::build::tests::started;
use crate::construction::build::{BuildAction, BuildTask};
use crate::construction::repair::{RepairAction, RepairState, RepairTask};
use crate::construction::wait_solidify::WaitSolidify;
use crate::construction::{FoundationId, RepairTargetId};
use crate::garrison::EnterState;
use crate::gathering::DropSiteId;
use crate::gathering::gather::GatherAction;
use crate::gathering::hunt::tests::hunting;
use crate::gathering::hunt::{HuntAction, HuntState, Lead};
use crate::map::Point;
use crate::map::{Distance, Offset, Position};
use crate::movement::explore::ExploreAction;
use crate::movement::fly::{Fly, FlyState};
use crate::movement::move_to::{MoveTo, MoveToAction};
use crate::movement::{Acceleration, Speed};
use crate::objects::ListIndex;
use crate::objects::Voice;
use crate::objects::WorkRate;
use crate::objects::actions::ActionList;
use crate::objects::masters::ActionKind;
use crate::objects::masters::TaskId;
use crate::objects::masters::TaskSprites;
use crate::objects::masters::{Task, WorkValue};
use crate::objects::teardown::HeadCode;
use crate::ownership::artifact::Artifact;
use crate::players::PlayerId;
use crate::test_ids::object_id;
use crate::test_ids::task_key;
use crate::unit_ai::{Answer, OrderForce};
use crate::world::Seconds;
use crate::world::World;
use crate::world::tests::{enter_town_centre, forage_world, town_workers_and_holding};
use crate::world::tests::{villager_holding, wait_over, walk_to_town_centre};

/// The idle update of `action` with every object it holds in `state`.
fn idled_in(action: &mut Action, state: ObjectState) -> Idled {
    action
        .idle_update(&IdleReads {
            state_of: &|_| Ok(state),
        })
        .unwrap()
}

/// What an idle update that let `released` go in this order and left no building gives.
fn releasing(released: &[i32]) -> Idled {
    Idled {
        clears_count: false,
        left: TornDown {
            releases: released.iter().copied().map(object_id).collect(),
            builder_leaves: Vec::new(),
        },
    }
}

/// A `MoveTo` to (5, 5) that holds `target`, with `voice`.
fn move_to_holding(target: i32, voice: Voice) -> MoveToAction {
    MoveToAction {
        target: Some(object_id(target)),
        voice,
        ..MoveToAction::sub_in(MoveTo::Moving)
    }
}

/// A Build of foundation 10028 (its builder `added` to its count), its drop site 10050.
fn build_on_site(added: bool) -> BuildAction {
    let task = BuildTask {
        range: Distance::ZERO,
        work_value: WorkValue::new(1.0),
        work_rate: WorkRate::new(1.0),
        resource: None,
        id: TaskId::of_task(-1),
        key: task_key(ActionKind::BUILD),
        sprites: TaskSprites::NONE,
    };
    BuildAction {
        added,
        site: Some(DropSiteId::new(object_id(10050))),
        ..started(
            object_id(10028),
            Position::new(30.0, 88.0, 0.0),
            Distance::ZERO,
            task,
        )
    }
}

/// `build_action` after its target went: uncounted, and its quiet walk's `MoveTo` holding the
/// foundation no longer either.
fn dropped_build(build_action: BuildAction) -> BuildAction {
    let mut dropped = BuildAction {
        target: None,
        added: false,
        ..build_action
    };
    if let Some(Some(walk)) = dropped.state.walk_mut() {
        walk.target = None;
    }
    dropped
}

/// A Repair of object 20 (its repairer `counted` among the target's repairers), with `voice`.
fn repair_of(counted: bool, voice: Voice) -> RepairAction {
    RepairAction {
        target: Some(RepairTargetId::new(object_id(20))),
        state: RepairState::Work,
        point: Position::ORIGIN,
        timer: Seconds::ZERO,
        task: RepairTask::of(&Task::default()),
        range: Distance::ZERO,
        counted,
        voice,
    }
}

/// A missile on object 9150, fired from object 9149.
fn missile_on() -> MissileAction {
    MissileAction {
        state: MissileState::Flying,
        target: Some(MissileTargetId::new(object_id(9150))),
        fired_from: Some(ShooterId::new(object_id(9149))),
        point: Position::ORIGIN,
        timer: Distance::ZERO,
        direction: Offset::ZERO,
        ballistic_velocity: Speed::ZERO,
        ballistic_acceleration: Acceleration::ZERO,
        distance: Distance::ZERO,
        hit_mode: HitMode::TARGET_ALONE,
        result: StrikeResult::None,
        origin: Origin::NONE,
        met: Vec::new(),
    }
}

/// `FUN_140a784a0` tests `+0x640` against 7 (`JC`): a target in state 7 or 8 goes, one in state
/// 6 stays, for each kind's slot 0x68 with none; the base's lets it go. The action holds no
/// target after, and nothing else changes.
#[test]
fn target_past_state_6_goes_and_is_let_go() {
    let artifact = Artifact {
        target: Some(object_id(9150)),
        ..Artifact::new(PlayerId::new(1), None)
    };
    let enter = enter_town_centre();
    let actions = [
        (Action::Artifact(artifact), 9150),
        (Action::MoveTo(move_to_holding(9149, Voice::Loud)), 9149),
        (Action::Enter(enter), 9150),
        (Action::Hunt(hunting(9149)), 9149),
        (
            Action::Missile(MissileAction {
                fired_from: None,
                ..missile_on()
            }),
            9150,
        ),
        (Action::Repair(repair_of(false, Voice::Loud)), 20),
    ];
    for (action, target) in actions {
        let mut kept = action.clone();
        assert_eq!(
            idled_in(&mut kept, ObjectState::Undead),
            Idled::default(),
            "{action:?}"
        );
        assert_eq!(kept, action);
        for gone in [ObjectState::Removed, ObjectState::Gone] {
            let mut dropped = action.clone();
            assert_eq!(idled_in(&mut dropped, gone), releasing(&[target]));
            assert_eq!(dropped.target(), None, "{action:?}");
        }
    }
    let mut artifact_held = Action::Artifact(artifact);
    let _ = idled_in(&mut artifact_held, ObjectState::Removed);
    assert_eq!(
        artifact_held,
        Action::Artifact(Artifact {
            target: None,
            ..artifact
        })
    );
}

/// The Gather's slot 0x68 is the base's: its target goes and is let go; then its drop site (the
/// second target) past state 6 goes and is let go too, its place and the rest as they were.
#[test]
fn gather_lets_its_target_then_its_drop_site_go() {
    let mut world = forage_world();
    assert_eq!(
        world.gather_command(
            PlayerId::new(1),
            ListIndex::FIRST,
            object_id(600),
            OrderForce::Forced,
        ),
        Ok(Answer::Taken)
    );
    let gather = *world
        .parts
        .actions_of(object_id(9175))
        .list
        .gather()
        .unwrap();
    let carrying = GatherAction {
        carry_to: Some(DropSiteId::new(object_id(700))),
        ..gather
    };
    let mut action = Action::Gather(carrying);
    assert_eq!(
        idled_in(&mut action, ObjectState::Removed),
        releasing(&[600, 700])
    );
    let mut dropped = GatherAction {
        target: None,
        carry_to: None,
        ..gather
    };
    // its quiet walk's MoveTo holds the bush no longer either, without a release
    if let Some(Some(walk)) = dropped.state.walk_mut() {
        walk.target = None;
    }
    assert_eq!(action, Action::Gather(dropped));
}

/// A quiet `MoveTo` (`+0x60`, as a sub-action is) lets its target go without the release
/// (`FUN_140a7a230`): it holds none after.
#[test]
fn quiet_move_to_holds_its_target_no_longer_without_release() {
    let mut action = Action::MoveTo(move_to_holding(9149, Voice::Quiet));
    assert_eq!(
        idled_in(&mut action, ObjectState::Removed),
        Idled::default()
    );
    assert_eq!(action.target(), None);
}

/// The Attack's slot 0x68 (`FUN_140a82a10`) clears the unit's attack count on a target it held,
/// then the base lets it go unless the Attack is quiet.
#[test]
fn attack_clears_the_count_and_lets_go_unless_quiet() {
    let loud = own_attack(9149, AttackState::Ready, Distance::ZERO);
    for (voice, released) in [(Voice::Loud, vec![9149]), (Voice::Quiet, vec![])] {
        let mut action = Action::Attack(Attack { voice, ..loud });
        assert_eq!(
            idled_in(&mut action, ObjectState::Removed),
            Idled {
                clears_count: true,
                ..releasing(&released)
            }
        );
        assert_eq!(
            action,
            Action::Attack(Attack {
                target: None,
                voice,
                ..loud
            })
        );
    }
}

/// The Hunt's slot 0x68 (`FUN_140ac7980`) clears its lead after the base, its drop site past
/// state 6 goes too.
#[test]
fn hunt_clears_its_lead_with_its_target() {
    let leading = HuntAction {
        carry_to: Some(DropSiteId::new(object_id(700))),
        lead: Lead {
            moved: true,
            still_time: 4,
            start: Point::new(3.0, 4.0),
        },
        ..hunting(9149)
    };
    let mut action = Action::Hunt(leading);
    assert_eq!(
        idled_in(&mut action, ObjectState::Removed),
        releasing(&[9149, 700])
    );
    assert_eq!(
        action,
        Action::Hunt(HuntAction {
            target: None,
            carry_to: None,
            lead: Lead::NEW,
            ..leading
        })
    );
}

/// The Build's (`FUN_140ab0300`) and the Repair's (`FUN_140ada470`) slot 0x68 first take a
/// counted builder or repairer out of its building's count, then the base lets the target go
/// unless the action is quiet (a Repair's `+0x60`); an uncounted one only lets go. The Build's
/// drop site goes after.
#[test]
fn counted_builder_and_repairer_leave_their_building_first() {
    for added in [true, false] {
        let mut action = Action::Build(build_on_site(added));
        let idled = idled_in(&mut action, ObjectState::Removed);
        let leaves = if added {
            vec![object_id(10028)]
        } else {
            vec![]
        };
        assert_eq!(
            (idled.left.builder_leaves, idled.left.releases),
            (leaves, vec![object_id(10028), object_id(10050)])
        );
        assert_eq!(
            action,
            Action::Build(dropped_build(BuildAction {
                site: None,
                ..build_on_site(added)
            }))
        );
    }
    for (counted, voice, released) in [
        (true, Voice::Loud, vec![object_id(20)]),
        (true, Voice::Quiet, vec![]),
        (false, Voice::Loud, vec![object_id(20)]),
    ] {
        let mut action = Action::Repair(repair_of(counted, voice));
        let idled = idled_in(&mut action, ObjectState::Gone);
        let leaves = if counted { vec![object_id(20)] } else { vec![] };
        assert_eq!(
            (idled.left.builder_leaves, idled.left.releases),
            (leaves, released)
        );
        assert_eq!(
            action,
            Action::Repair(RepairAction {
                target: None,
                counted: false,
                ..repair_of(counted, voice)
            })
        );
    }
}

/// A second target past state 6 is let go whatever the action's `+0x60` (a quiet Attack's
/// too) and goes; one in state 6 stays, as does the target in state 6.
#[test]
fn second_target_past_state_6_goes_whatever_the_voice() {
    let quiet = Attack {
        target2: Some(AttackTargetId::new(object_id(9150))),
        voice: Voice::Quiet,
        ..own_attack(9149, AttackState::Ready, Distance::ZERO)
    };
    let states = |id: ObjectId| {
        Ok(if id == object_id(9150) {
            ObjectState::Removed
        } else {
            ObjectState::Undead
        })
    };
    let mut action = Action::Attack(quiet);
    assert_eq!(
        action.idle_update(&IdleReads { state_of: &states }),
        Ok(releasing(&[9150]))
    );
    assert_eq!(
        action,
        Action::Attack(Attack {
            target2: None,
            ..quiet
        })
    );
    let mut missile = Action::Missile(missile_on());
    assert_eq!(
        missile.idle_update(&IdleReads {
            state_of: &|id| Ok(if id == object_id(9149) {
                ObjectState::Gone
            } else {
                ObjectState::Alive
            }),
        }),
        Ok(releasing(&[9149]))
    );
    assert_eq!(
        missile,
        Action::Missile(MissileAction {
            fired_from: None,
            ..missile_on()
        })
    );
}

/// `FUN_140a98bf0` runs `FUN_140a784a0` on the sub-actions after the action's own targets: the
/// Hunt's Attack (quiet: its target goes without a release, the unit's count clears), the
/// Attack's second target, then the Attack's walk's `MoveTo`, in this order.
#[test]
fn sub_actions_follow_the_targets_in_order() {
    let attack = Attack {
        target2: Some(AttackTargetId::new(object_id(4))),
        state: AttackState::Move {
            move_to: Some(move_to_holding(5, Voice::Loud)),
        },
        voice: Voice::Quiet,
        ..own_attack(3, AttackState::Ready, Distance::ZERO)
    };
    let hunt = HuntAction {
        carry_to: Some(DropSiteId::new(object_id(2))),
        state: HuntState::Attack {
            attack: Some(attack),
        },
        ..hunting(1)
    };
    let mut action = Action::Hunt(hunt);
    assert_eq!(
        idled_in(&mut action, ObjectState::Removed),
        Idled {
            clears_count: true,
            ..releasing(&[1, 2, 4, 5])
        }
    );
    let gone = Attack {
        target: None,
        target2: None,
        state: AttackState::Move {
            move_to: Some(MoveToAction {
                target: None,
                ..move_to_holding(5, Voice::Loud)
            }),
        },
        ..attack
    };
    assert_eq!(
        action,
        Action::Hunt(HuntAction {
            target: None,
            carry_to: None,
            state: HuntState::Attack { attack: Some(gone) },
            lead: Lead::NEW,
            ..hunt
        })
    );
}

/// The Artifact's sub-action takes the idle update too: its Enter lets its building go and its
/// walk's `MoveTo` its target; an Enter's and a Fly's walk alike, the Fly holding no target of
/// its own.
#[test]
fn walks_and_the_artifact_enter_let_their_targets_go() {
    let mut walking_enter = enter_town_centre();
    walking_enter.state = EnterState::Walk {
        move_to: Some(move_to_holding(9149, Voice::Loud)),
    };
    let entering = Artifact {
        sub: Some(ArtifactSub::Enter {
            enter: walking_enter,
        }),
        ..Artifact::new(PlayerId::new(1), None)
    };
    let mut action = Action::Artifact(entering);
    assert_eq!(
        idled_in(&mut action, ObjectState::Removed),
        releasing(&[9150, 9149])
    );
    let Action::Artifact(Artifact {
        sub: Some(ArtifactSub::Enter { enter }),
        ..
    }) = action
    else {
        panic!("the Artifact keeps its Enter: {action:?}");
    };
    assert_eq!(
        (enter.target, enter.state.move_to().map(|walk| walk.target)),
        (None, Some(None))
    );
    let walking = Artifact {
        sub: Some(ArtifactSub::Walk {
            move_to: move_to_holding(9149, Voice::Loud),
        }),
        ..Artifact::new(PlayerId::new(1), None)
    };
    let mut action = Action::Artifact(walking);
    assert_eq!(idled_in(&mut action, ObjectState::Gone), releasing(&[9149]));
    let mut action = Action::Enter(walking_enter);
    assert_eq!(
        idled_in(&mut action, ObjectState::Gone),
        releasing(&[9150, 9149])
    );
    let fly = Fly {
        state: FlyState::Flying {
            move_to: Some(move_to_holding(9149, Voice::Loud)),
        },
        timer: Seconds::new(1.0),
        home: Point::new(10.0, 10.0),
        task: Task::default(),
    };
    let mut action = Action::Fly(fly);
    assert_eq!(idled_in(&mut action, ObjectState::Gone), releasing(&[9149]));
}

/// An action without a target (`+0x20` null) reads no object: a wait for a foundation keeps its
/// foundation at `+0x68`, a Make, an Explore and a Transport walk nowhere here; a state read
/// would fail.
#[test]
fn action_without_target_reads_no_object() {
    let fails = |id: ObjectId| Err(GenieError::unreachable("read").with("object", id));
    let actions = [
        Action::WaitSolidify(WaitSolidify::new(
            Task::default(),
            FoundationId::new(object_id(10028)),
        )),
        Action::Explore(ExploreAction::new(Position::ORIGIN)),
    ];
    for action in actions {
        let mut idle = action.clone();
        assert_eq!(
            idle.idle_update(&IdleReads { state_of: &fails }),
            Ok(Idled::default())
        );
        assert_eq!(idle, action);
    }
    let mut held = Action::MoveTo(move_to_holding(9149, Voice::Loud));
    assert_eq!(
        held.idle_update(&IdleReads { state_of: &fails })
            .unwrap_err()
            .reason(),
        "read"
    );
}

/// The villager's step with `from` as the idle update's start, and its code `code` after.
fn end_update(world: &mut World, code: HeadCode, from: IdleFrom) -> GenieResult<()> {
    world.run_on_step(
        PlayerId::new(1),
        ListIndex::new(2),
        |unit, actions, step| unit.end_head_update(actions, step, code, from),
    )
}

/// The workers of town centres 9149 and 9150.
fn town_workers(world: &World) -> (i32, i32) {
    let (first, second, ..) = town_workers_and_holding(world);
    (first, second)
}

/// `world`'s object `id` in `state`.
fn put_in_state(world: &mut World, id: i32, state: ObjectState) {
    world.object_mut(object_id(id)).unwrap().1.life.state = state;
}

/// The list update (`FUN_140a992b0`) runs the idle update (`FUN_140a78470`) of each action below
/// the head, in the list's order: the walk and the Artifact let their removed town centres go at
/// once (one worker less each) and hold them no longer; the head keeps its target. A town centre
/// in state 6 stays held.
#[test]
fn list_update_idles_each_action_below_the_head() {
    let artifact = Artifact {
        target: Some(object_id(9150)),
        ..Artifact::new(PlayerId::new(1), None)
    };
    let below = ActionList::default()
        .with_artifact(Some(artifact))
        .with_move_to(Some(walk_to_town_centre()));
    let mut world = villager_holding(wait_over(below.clone()), ObjectState::Alive);
    for id in [9149, 9150] {
        put_in_state(&mut world, id, ObjectState::Undead);
    }
    end_update(&mut world, HeadCode::Keeps, IdleFrom::BelowHead).unwrap();
    assert_eq!(world.parts.actions_of(object_id(40)).list, wait_over(below));
    assert_eq!(town_workers(&world), (1, 1));
    for id in [9149, 9150] {
        put_in_state(&mut world, id, ObjectState::Removed);
    }
    end_update(&mut world, HeadCode::Keeps, IdleFrom::BelowHead).unwrap();
    assert_eq!(
        world.parts.actions_of(object_id(40)).list,
        wait_over(
            ActionList::default()
                .with_artifact(Some(Artifact {
                    target: None,
                    ..artifact
                }))
                .with_move_to(Some(MoveToAction {
                    target: None,
                    ..walk_to_town_centre()
                }))
        )
    );
    assert_eq!(town_workers(&world), (0, 0));
}

/// The inside update (`FUN_140a98790`) idles the head too (`FUN_140a78480`) before its
/// deletion; the list update leaves the head to its own update.
#[test]
fn inside_update_idles_the_head_too() {
    let head = ActionList::default().with_move_to(Some(walk_to_town_centre()));
    for (from, workers, target) in [
        (IdleFrom::BelowHead, 1, Some(object_id(9149))),
        (IdleFrom::Head, 0, None),
    ] {
        let mut world = villager_holding(head.clone(), ObjectState::Alive);
        put_in_state(&mut world, 9149, ObjectState::Removed);
        end_update(&mut world, HeadCode::Keeps, from).unwrap();
        assert_eq!(town_workers(&world).0, workers, "{from:?}");
        assert_eq!(
            world
                .parts
                .actions_of(object_id(40))
                .list
                .move_to()
                .map(|move_to_action| move_to_action.target),
            Some(target)
        );
    }
}

/// An Attack below the head whose target went clears the unit's attack count (`+0x900`); one on
/// a target in state 6 keeps it.
#[test]
fn idle_attack_on_removed_target_clears_the_attack_count() {
    let attack = own_attack(9149, AttackState::Ready, Distance::ZERO);
    for (state, count) in [
        (ObjectState::Undead, AttackCount::of_data(3)),
        (ObjectState::Removed, AttackCount::ZERO),
    ] {
        let mut world = villager_holding(
            wait_over(ActionList::default().with_attack(Some(attack))),
            ObjectState::Alive,
        );
        world.parts.combat_mut(object_id(40)).unwrap().attack_count = AttackCount::of_data(3);
        put_in_state(&mut world, 9149, state);
        end_update(&mut world, HeadCode::Keeps, IdleFrom::BelowHead).unwrap();
        assert_eq!(
            world.parts.combat_of(object_id(40)).attack_count,
            count,
            "{state:?}"
        );
    }
}

/// A counted builder's Build below the head on a removed foundation leaves its builders and lets
/// it go (`FUN_140ab0300`, then `FUN_140a7a230`).
#[test]
fn counted_build_below_the_head_leaves_the_removed_foundation() {
    let mut build_action = build_on_site(true);
    build_action.site = None;
    let mut world = villager_holding(
        wait_over(ActionList::default().with_build(Some(build_action))),
        ObjectState::Alive,
    );
    put_in_state(&mut world, 10028, ObjectState::Removed);
    end_update(&mut world, HeadCode::Keeps, IdleFrom::BelowHead).unwrap();
    let foundation = &world.players[2].objects[ListIndex::FIRST];
    assert_eq!(
        (
            foundation.workers.get(),
            world
                .parts
                .building_of(foundation.id)
                .construction
                .map(|construction| construction.builders.get())
        ),
        (0, Some(0))
    );
    assert_eq!(
        world.parts.actions_of(object_id(40)).list.build().copied(),
        Some(dropped_build(build_action))
    );
}

/// An action that holds the updating unit itself reads the unit's own state, which the world's
/// table does not find while it updates; one that holds an object the world lacks fails, the
/// actions after it untouched.
#[test]
fn own_target_reads_the_unit_and_missing_one_fails() {
    let own = Artifact {
        target: Some(object_id(40)),
        ..Artifact::new(PlayerId::new(1), None)
    };
    let mut world = villager_holding(
        wait_over(ActionList::default().with_artifact(Some(own))),
        ObjectState::Alive,
    );
    assert_eq!(
        end_update(&mut world, HeadCode::Keeps, IdleFrom::BelowHead),
        Ok(())
    );
    let lost = ActionList::default()
        .with_move_to(Some(walk_to_town_centre()))
        .with_artifact(Some(Artifact {
            target: Some(object_id(99_999)),
            ..own
        }));
    let mut world = villager_holding(wait_over(lost.clone()), ObjectState::Alive);
    put_in_state(&mut world, 9149, ObjectState::Removed);
    assert_eq!(
        end_update(&mut world, HeadCode::Keeps, IdleFrom::BelowHead)
            .unwrap_err()
            .reason(),
        object_gone(object_id(99_999)).reason()
    );
    assert_eq!(world.parts.actions_of(object_id(40)).list, wait_over(lost));
    assert_eq!(town_workers(&world), (1, 1));
}

/// A code of 1 or 3 deletes the head after the idle update of the actions below it: the Enter
/// under the walk has let its removed town centre go before the walk's deletion lets go of its
/// own.
#[test]
fn deletion_follows_the_idle_update() {
    let list = ActionList::default()
        .with_enter(Some(enter_town_centre()))
        .with_move_to(Some(walk_to_town_centre()));
    let mut world = villager_holding(list, ObjectState::Dying);
    put_in_state(&mut world, 9150, ObjectState::Removed);
    end_update(&mut world, HeadCode::Deletes, IdleFrom::BelowHead).unwrap();
    assert_eq!(
        world.parts.actions_of(object_id(40)).list,
        ActionList::default().with_enter(Some(EnterAction {
            target: None,
            ..enter_town_centre()
        }))
    );
    assert_eq!(town_workers(&world), (0, 0));
}

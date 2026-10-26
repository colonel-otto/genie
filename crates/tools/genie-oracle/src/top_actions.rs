//! A saved object's top action as the slice models it ([`saved_actions`]).

use crate::{
    OTHER_SUB_ACTION, combat_master, moving_master, saved_attack, saved_move_wait, saved_reload,
    saved_sprite,
};
use genie_core::GenieError;
use genie_core::artifact::ARTIFACT_TYPE;
use genie_core::turn::Unit;
use genie_save::generated::world::{PlayerObjects, RgeStaticObject};

/// The top action of a saved object as the slice models it: the Artifact action (107), the
/// Build action (101) with its `MoveTo`, a `MoveTo` of its own (type 1), a bird's Fly action (10)
/// with its `MoveTo`, the Hunt (110) with its Attack or `MoveTo` (and in state 8 its second
/// target, the drop site) and a farm's Make (21). Any other saved action stops the object's
/// player.
#[allow(clippy::too_many_arguments)]
pub(super) fn saved_actions(
    object: &genie_save::Object,
    s: &RgeStaticObject,
    objects: &PlayerObjects,
    work: impl Fn(i16) -> Result<genie_core::build::BuildTask, GenieError>,
    fly_task: impl Fn(i16) -> Result<genie_core::masters::Task, GenieError>,
    task_at: impl Fn(i16, i16) -> Result<genie_core::masters::Task, GenieError>,
    graphics: Option<&std::collections::BTreeMap<i16, genie_core::sprite::Graphic>>,
    unit: &mut Unit,
) -> Result<(), GenieError> {
    use genie_core::build::{BUILD, BuildAction, MoveTo};
    use genie_core::fly::{FLY, Fly};
    use genie_core::move_to::{MOVE_TO, MoveToAction};
    let move_to_state = |state: u8| match state {
        1 => Ok(MoveTo::Arrived),
        2 => Ok(MoveTo::Waiting),
        3 => Ok(MoveTo::Start),
        4 => Ok(MoveTo::Moving),
        13 => Ok(MoveTo::Failed),
        _ => Err(GenieError::unsupported(
            "a saved MoveTo in a state the slice does not model",
        )),
    };
    let point = |b: &genie_save::generated::world::RgeAction| {
        (b.target_id == -1).then_some([b.target_x, b.target_y, b.target_z])
    };
    let (kind, action) = object
        .actions()
        .and_then(|a| a.entries().next())
        .expect("saved_actions is called for an object with actions");
    // the master's running sprite, which an Attack's MoveTo may show
    let run_sprite = objects
        .master(s.master_obj)
        .and_then(moving_master)
        .map_or(-1, |m| m.run_sprite_id);
    let base = action
        .base()
        .ok_or(GenieError::unsupported("a saved action without its base"))?;
    // the second target (+0x34): only the Hunt, the Gather and the Build (their drop site), the
    // missile (its shooter) and the Attack hold one in the slice
    if base.target2_id != -1
        && !matches!(
            kind,
            genie_core::gathering::hunt::HUNT
                | genie_core::gathering::gather::GATHER
                | genie_core::combat::missile::MISSILE
                | genie_core::combat::attack::ATTACK
                | BUILD
        )
    {
        return Err(GenieError::unsupported(
            "a saved action with a second target",
        ));
    }
    match kind {
        ARTIFACT_TYPE => {
            let sub = match base.sub_actions.entries().next() {
                None => None,
                Some((MOVE_TO, sub)) => Some(sub),
                Some(_) => {
                    return Err(GenieError::unsupported(
                        "a saved Artifact action with another sub-action",
                    ));
                }
            };
            let move_to = sub
                .map(|sub| move_to_state(sub.base().map_or(0, |b| b.state)))
                .transpose()?;
            // a move to an object (set_state 5): the target, and its quiet MoveTo's goal, a
            // town centre's point or the target, with its range
            let walk_to = match sub {
                Some(sub) if base.target_id != -1 => {
                    let b = sub
                        .base()
                        .ok_or(GenieError::unsupported("a saved MoveTo without its base"))?;
                    let goal =
                        if b.target_id == -1 {
                            genie_core::move_to::Goal::Point(point(b).ok_or(
                                GenieError::unsupported("a saved MoveTo without its point"),
                            )?)
                        } else {
                            genie_core::move_to::Goal::Target(b.target_id)
                        };
                    let range = sub
                        .move_to_range()
                        .ok_or(GenieError::unsupported("a saved MoveTo without its range"))?;
                    Some((goal, range))
                }
                _ => None,
            };
            unit.artifact = Some(genie_core::artifact::Artifact {
                state: base.state,
                timer: base.timer,
                gaia: s.owner == 0,
                move_to,
                goal: if base.target_id == -1 {
                    move_to.and_then(|_| point(base))
                } else {
                    // the target's place, `+0x38`
                    (base.target_x != -1.0).then_some([base.target_x, base.target_y, base.target_z])
                },
                target: base.target_id,
                walk_to,
                enter: None,
            });
            Ok(())
        }
        MOVE_TO => {
            unit.move_to = Some(MoveToAction {
                state: move_to_state(base.state)?,
                goal: point(base),
                target: base.target_id,
                range: action
                    .move_to_range()
                    .ok_or(GenieError::unsupported("a saved MoveTo without its range"))?,
                // its sprite (`+0x48`'s id) is the master's running one
                run: objects
                    .master(s.master_obj)
                    .and_then(moving_master)
                    .is_some_and(|m| {
                        0 <= m.run_sprite_id
                            && m.run_sprite_id != m.move_sprite_id
                            && base.short_val_2 == m.run_sprite_id
                    }),
                // its wait (`+0x6c`, `+0x70`, `+0x74`)
                wait: action
                    .rge_action_by_type
                    .as_deref()
                    .and_then(|b| b.rge_action_move_to.as_deref())
                    .map_or_else(genie_core::move_to::MoveWait::default, |m| {
                        genie_core::move_to::MoveWait {
                            state: m.movement_wait_state,
                            timer: m.timer,
                            delay: f32::from_bits(m.k_default_wait_time),
                        }
                    }),
            });
            Ok(())
        }
        genie_core::combat::missile::MISSILE => {
            let ma = action
                .rge_action_by_type
                .as_deref()
                .and_then(|t| t.rge_action_missile.as_deref())
                .ok_or(GenieError::unsupported(
                    "a saved missile action without its part",
                ))?;
            let mo = object
                .rge_object
                .as_deref()
                .and_then(|r| r.rge_missile_object.as_deref())
                .ok_or(GenieError::unsupported(
                    "a missile action of an object that is not a missile",
                ))?;
            let moving = object
                .moving_part()
                .ok_or(GenieError::unsupported("a missile without its moving part"))?;
            // the hit mode copies the master's (`+0x80`, a miss's 1 is not saved)
            let hit_mode = objects
                .master(s.master_obj)
                .and_then(|m| m.rge_master.as_deref())
                .and_then(|r| r.rge_master_missile_object.as_deref())
                .map_or(0, |m| m.missile_hit_info);
            unit.missile = Some(genie_core::combat::missile::Missile {
                max_range: mo.max_range,
                fired_from: mo.fired_from_id,
                charge: mo.is_charge_projectile != 0,
                flying: genie_core::combat::missile::LANDING < ma.distance,
                step: [moving.velocity_x, moving.velocity_y, moving.velocity_z],
                attack_count: mo.rge_combat_object.attack_count,
                action: Some(genie_core::combat::missile::MissileAction {
                    state: base.state,
                    target: base.target_id,
                    fired_from: base.target2_id,
                    point: [base.target_x, base.target_y, base.target_z],
                    timer: base.timer,
                    direction: [ma.velocity_x, ma.velocity_y, ma.velocity_z],
                    ballistic_velocity: ma.ballistic_velocity,
                    ballistic_acceleration: ma.ballistic_acceleration,
                    distance: ma.distance,
                    hit_mode,
                    result: ma.interacted_with_agility,
                    origin: genie_core::combat::missile::Origin::from_bytes(&ma.projectile_data),
                    met: Vec::new(),
                }),
            });
            Ok(())
        }
        genie_core::gathering::make::MAKE => {
            let make = action
                .rge_action_by_type
                .as_deref()
                .and_then(|t| t.rge_action_make.as_deref())
                .ok_or(GenieError::unsupported("a saved Make without its part"))?;
            if base.sub_actions.entries().next().is_some() {
                return Err(GenieError::unsupported("a saved Make with a sub-action"));
            }
            unit.make = Some(genie_core::gathering::make::MakeAction {
                state: base.state,
                remaining: base.timer,
                total: make.work_timer,
                task: task_at(s.master_obj, base.short_val)?,
            });
            Ok(())
        }
        FLY => {
            let sub = match base.sub_actions.entries().next() {
                None => None,
                Some((MOVE_TO, sub)) => sub.base(),
                Some(_) => {
                    return Err(GenieError::unsupported(
                        "a saved Fly action with another sub-action",
                    ));
                }
            };
            let move_to = sub.map(|b| move_to_state(b.state)).transpose()?;
            unit.fly = Some(Fly {
                state: base.state,
                timer: base.timer,
                move_to,
                goal: sub.and_then(point),
                home: [base.target_x, base.target_y],
                task: fly_task(s.master_obj)?,
            });
            Ok(())
        }
        BUILD => {
            let move_to = match base.sub_actions.entries().next() {
                None => None,
                Some((MOVE_TO, sub)) => Some(move_to_state(sub.base().map_or(0, |b| b.state))?),
                Some(_) => return Err(OTHER_SUB_ACTION),
            };
            // a MoveTo to a point has no target; its point is the action's target point
            let goal = base
                .sub_actions
                .entries()
                .next()
                .and_then(|(_, sub)| sub.base())
                .filter(|b| b.target_id == -1)
                .map(|b| [b.target_x, b.target_y, b.target_z]);
            // the MoveTo's target, −1 for one to a point or none
            let walk_target = base
                .sub_actions
                .entries()
                .next()
                .and_then(|(_, sub)| sub.base())
                .map_or(-1, |b| b.target_id);
            let mut task = work(s.master_obj)?;
            task.work_rate += unit.work_rate_powerup;
            // the drop-off's MoveTo keeps its range (+0x68)
            let drop_range = match base.sub_actions.entries().next() {
                Some((MOVE_TO, sub)) if base.state == genie_core::build::DROP => sub
                    .move_to_range()
                    .ok_or(GenieError::unsupported("a saved MoveTo without its range"))?,
                _ => task.range,
            };
            unit.build = Some(BuildAction {
                target: base.target_id,
                state: base.state,
                move_to,
                goal,
                walk_target,
                point: [base.target_x, base.target_y, base.target_z],
                task,
                added: action
                    .tribe_action_build
                    .as_deref()
                    .is_some_and(|b| b.added_to_num_builders != 0),
                timer: base.timer,
                move_wait: base
                    .sub_actions
                    .entries()
                    .next()
                    .map_or_else(genie_core::move_to::MoveWait::default, |(_, sub)| {
                        saved_move_wait(sub)
                    }),
                site: base.target2_id,
                drop_range,
            });
            Ok(())
        }
        genie_core::repair::REPAIR => {
            let part = action
                .tribe_action_repair
                .as_deref()
                .ok_or(GenieError::missing_data("a saved Repair's own part"))?;
            let sub = base.sub_actions.entries().next();
            let move_to = match sub {
                None => None,
                Some((MOVE_TO, sub)) => Some(move_to_state(sub.base().map_or(0, |b| b.state))?),
                Some(_) => return Err(OTHER_SUB_ACTION),
            };
            // a MoveTo to a point has no target; its point is the action's target point
            let goal = sub
                .and_then(|(_, sub)| sub.base())
                .filter(|b| b.target_id == -1)
                .map(|b| [b.target_x, b.target_y, b.target_z]);
            let t = task_at(s.master_obj, base.short_val)?;
            unit.repair = Some(genie_core::repair::RepairAction {
                target: base.target_id,
                state: base.state,
                move_to,
                move_wait: sub.map_or_else(genie_core::move_to::MoveWait::default, |(_, sub)| {
                    saved_move_wait(sub)
                }),
                goal,
                point: [base.target_x, base.target_y, base.target_z],
                timer: base.timer,
                task: genie_core::repair::RepairTask {
                    id: t.id,
                    work_value: t.work_val1,
                },
                range: objects
                    .master(s.master_obj)
                    .and_then(combat_master)
                    .map_or(0.0, |c| c.weapon_range),
                counted: part.added_to_num_builders != 0,
                quiet: base.sub_action_value != 0,
            });
            Ok(())
        }
        genie_core::gathering::hunt::HUNT => {
            use genie_core::gathering::hunt::{
                ATTACK, DROP, FACE, GATHER, HuntAction, HuntTask, IDLE, LEAD, SEARCH, WALK,
                WALK_POINT, WALK_SEARCH,
            };
            let part = action
                .tribe_action_hunt
                .as_deref()
                .ok_or(GenieError::unsupported("a saved Hunt without its own part"))?;
            if !matches!(
                base.state,
                IDLE | SEARCH
                    | WALK
                    | FACE
                    | GATHER
                    | DROP
                    | LEAD
                    | ATTACK
                    | WALK_SEARCH
                    | WALK_POINT
            ) {
                return Err(GenieError::unsupported(
                    "a saved Hunt in a state the slice does not model",
                ));
            }
            let t = task_at(s.master_obj, base.short_val)?;
            let task = HuntTask {
                id: t.id,
                range: t.work_range,
                resource: t.attribute_type,
                sprites: t.sprites,
                work_value: t.work_val1,
                productivity: t.attribute_type2,
                kind4: t.attribute_type4,
            };
            let mut hunt = HuntAction {
                target: base.target_id,
                state: base.state,
                point: [base.target_x, base.target_y, base.target_z],
                target_class: part.target_type as i16,
                task,
                attack: None,
                move_to: None,
                walk_range: task.range,
                drop_site: part.sheep_drop_site_id as i32,
                carry_to: base.target2_id,
                timer: base.timer,
                move_wait: genie_core::move_to::MoveWait::default(),
                lead: genie_core::gathering::hunt::Lead {
                    moved: part.moved_sheep != 0,
                    still_time: part.sheep_still_time as i16,
                    start: [
                        f32::from_bits(part.sheep_start_x),
                        f32::from_bits(part.sheep_start_y),
                    ],
                },
            };
            match base.sub_actions.entries().next() {
                None => {}
                Some((genie_core::combat::attack::ATTACK, sub)) => {
                    let mut attack = saved_attack(sub, graphics, &move_to_state, run_sprite)?;
                    // a swing past its hit: the swing's times (`+0xb0`, `+0xb4`, unsaved) are what
                    // the hit left, the reload less the attack sprite's length
                    if attack.state == genie_core::combat::attack::SWING && !attack.pending {
                        let reload = saved_reload(object, s, objects).ok_or(
                            GenieError::unsupported("a saved swing without its unit's reload"),
                        )?;
                        let g = attack.arms.sprite.ok_or(GenieError::unsupported(
                            "a saved swing without its attack sprite",
                        ))?;
                        let rest = reload - f32::from(g.frames) * g.frame_duration;
                        attack.swing = rest;
                        attack.rest = rest.max(0.0);
                    }
                    hunt.attack = Some(attack);
                    // the attack reads and changes the unit's sprite from here on
                    unit.sprite = graphics.and_then(|g| saved_sprite(s, g));
                }
                Some((MOVE_TO, sub)) => {
                    hunt.move_to = Some(move_to_state(sub.base().map_or(0, |b| b.state))?);
                    hunt.walk_range = sub
                        .move_to_range()
                        .ok_or(GenieError::unsupported("a saved MoveTo without its range"))?;
                    // its wait (`+0x6c`, `+0x70`, `+0x74`)
                    if let Some(m) = sub
                        .rge_action_by_type
                        .as_deref()
                        .and_then(|b| b.rge_action_move_to.as_deref())
                    {
                        hunt.move_wait = genie_core::move_to::MoveWait {
                            state: m.movement_wait_state,
                            timer: m.timer,
                            delay: f32::from_bits(m.k_default_wait_time),
                        };
                    }
                }
                Some(_) => {
                    return Err(GenieError::unsupported(
                        "a saved Hunt with another sub-action",
                    ));
                }
            }
            unit.hunt = Some(hunt);
            Ok(())
        }
        genie_core::gathering::gather::GATHER => {
            use genie_core::gathering::gather::{
                DONE, DROP, FACE, GatherAction, IDLE, MOVED, SEARCH, WALK, WALK_BACK, WORK,
            };
            let part = action
                .rge_action_by_type
                .as_deref()
                .and_then(|b| b.rge_action_gather.as_deref())
                .ok_or(GenieError::unsupported(
                    "a saved Gather without its own part",
                ))?;
            if !matches!(
                base.state,
                WALK | FACE | WORK | DROP | DONE | IDLE | MOVED | SEARCH | WALK_BACK
            ) {
                return Err(GenieError::unsupported(
                    "a saved Gather in a state the slice does not model",
                ));
            }
            let t = task_at(s.master_obj, base.short_val)?;
            let task = genie_core::gathering::hunt::HuntTask {
                id: t.id,
                range: t.work_range,
                resource: t.attribute_type,
                sprites: t.sprites,
                work_value: t.work_val1,
                productivity: t.attribute_type2,
                kind4: t.attribute_type4,
            };
            let mut gather = GatherAction {
                target: base.target_id,
                carry_to: base.target2_id,
                state: base.state,
                point: [base.target_x, base.target_y, base.target_z],
                timer: base.timer,
                task,
                target_type: part.target_type,
                stops: part.number_stops,
                current_stop: part.current_stop,
                time_per_stop: part.time_per_stop,
                time_at_stop: part.time_at_current_stop,
                last_stop: [part.last_stop_x, part.last_stop_y],
                // `+0x80` is not saved: set_state(7) set it
                worked: base.state == WORK,
                farm: [part.farm_x, part.farm_y],
                gather_at: [part.gather_x, part.gather_y],
                move_to: None,
                walk_to: None,
                walk_range: task.range,
                move_wait: genie_core::move_to::MoveWait::default(),
            };
            match base.sub_actions.entries().next() {
                None => {}
                Some((MOVE_TO, sub)) => {
                    gather.move_to = Some(move_to_state(sub.base().map_or(0, |b| b.state))?);
                    // a MoveTo without a target walks to its point (a farm's stop)
                    gather.walk_to = sub
                        .base()
                        .filter(|b| b.target_id == -1)
                        .map(|b| [b.target_x, b.target_y, b.target_z]);
                    gather.walk_range = sub
                        .move_to_range()
                        .ok_or(GenieError::unsupported("a saved MoveTo without its range"))?;
                    if let Some(m) = sub
                        .rge_action_by_type
                        .as_deref()
                        .and_then(|b| b.rge_action_move_to.as_deref())
                    {
                        gather.move_wait = genie_core::move_to::MoveWait {
                            state: m.movement_wait_state,
                            timer: m.timer,
                            delay: f32::from_bits(m.k_default_wait_time),
                        };
                    }
                }
                Some(_) => {
                    return Err(GenieError::unsupported(
                        "a saved Gather with another sub-action",
                    ));
                }
            }
            unit.gather = Some(gather);
            Ok(())
        }
        genie_core::garrison::ENTER => {
            use genie_core::garrison::{ENTERED, EnterAction};
            let first_time = action
                .rge_action_enter
                .as_deref()
                .ok_or(GenieError::unsupported(
                    "a saved Enter without its own part",
                ))?
                .first_time_enter;
            let move_to = match base.sub_actions.entries().next() {
                None => None,
                Some((MOVE_TO, sub)) => Some(move_to_state(sub.base().map_or(0, |b| b.state))?),
                Some(_) => {
                    return Err(GenieError::unsupported(
                        "a saved Enter with another sub-action",
                    ));
                }
            };
            if !matches!(base.state, ENTERED | 2 | 4 | genie_core::garrison::FAILED) {
                return Err(GenieError::unsupported(
                    "a saved Enter in a state the slice does not model",
                ));
            }
            let task = task_at(s.master_obj, base.short_val)?;
            unit.enter = Some(EnterAction {
                target: base.target_id,
                state: base.state,
                move_to,
                point: [base.target_x, base.target_y, base.target_z],
                range: task.work_range,
                timer: base.timer,
                retry: first_time != 0,
                task_class: task.object_group,
                move_wait: base
                    .sub_actions
                    .entries()
                    .next()
                    .map_or_else(genie_core::move_to::MoveWait::default, |(_, sub)| {
                        saved_move_wait(sub)
                    }),
            });
            Ok(())
        }
        genie_core::transport::TRANSPORT => {
            // the base alone is saved (FUN_140aa3b20): +0x34 is its second target's word, and
            // +0x68 starts clear; set_state makes no sub-action but a MoveTo
            let move_to = match base.sub_actions.entries().next() {
                None => None,
                Some((MOVE_TO, sub)) => Some(move_to_state(sub.base().map_or(0, |b| b.state))?),
                Some(_) => {
                    return Err(GenieError::unreachable(
                        "a saved Transport with another sub-action",
                    ));
                }
            };
            unit.transport = Some(genie_core::transport::TransportAction {
                state: base.state,
                mode: base.target2_id,
                point: [base.target_x, base.target_y, base.target_z],
                move_to,
                retarget: false,
            });
            Ok(())
        }
        genie_core::explore::EXPLORE => {
            let move_to = match base.sub_actions.entries().next() {
                None => None,
                Some((MOVE_TO, sub)) => Some(move_to_state(sub.base().map_or(0, |b| b.state))?),
                Some(_) => {
                    return Err(GenieError::unsupported(
                        "a saved Explore with another sub-action",
                    ));
                }
            };
            if !matches!(
                base.state,
                genie_core::explore::WAIT | genie_core::explore::WALK
            ) {
                return Err(GenieError::unsupported(
                    "a saved Explore in a state the slice does not model",
                ));
            }
            let move_wait = base
                .sub_actions
                .entries()
                .next()
                .and_then(|(_, sub)| sub.rge_action_by_type.as_deref())
                .and_then(|b| b.rge_action_move_to.as_deref())
                .map(|m| genie_core::move_to::MoveWait {
                    state: m.movement_wait_state,
                    timer: m.timer,
                    delay: f32::from_bits(m.k_default_wait_time),
                })
                .unwrap_or_default();
            unit.explore = Some(genie_core::explore::ExploreAction {
                state: base.state,
                point: [base.target_x, base.target_y, base.target_z],
                move_to,
                move_wait,
            });
            Ok(())
        }
        genie_core::combat::attack::ATTACK => {
            unit.attack = Some(saved_attack(action, graphics, &move_to_state, run_sprite)?);
            // the attack reads and changes the unit's sprite from here on
            unit.sprite = graphics.and_then(|g| saved_sprite(s, g));
            Ok(())
        }
        genie_core::wait_solidify::WAIT_SOLIDIFY => {
            // TRIBE_Action_Wait_Solidify's load (FUN_140ae8220): the base action's state and
            // task, and the foundation it waits for (`mTargetBuildingId`, +0x68); it makes no
            // sub-action
            if base.sub_actions.entries().next().is_some() {
                return Err(GenieError::unsupported(
                    "a saved Wait_Solidify with a sub-action",
                ));
            }
            let target = action
                .tribe_action_wait_solidify
                .as_deref()
                .ok_or(GenieError::unsupported(
                    "a saved Wait_Solidify without its own part",
                ))?
                .target_building_id;
            let mut wait = genie_core::wait_solidify::WaitSolidify::new(
                task_at(s.master_obj, base.short_val)?,
                target,
            );
            wait.set_state(base.state);
            unit.wait_solidify = Some(wait);
            Ok(())
        }
        _ => Err(GenieError::unsupported(
            "a saved action the slice does not import",
        )),
    }
}

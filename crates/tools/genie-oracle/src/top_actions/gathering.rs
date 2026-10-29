//! The gathering actions of a saved object's top action: the Hunt, the Gather and a farm's
//! Make ([`crate::top_actions::saved_actions`]).

use crate::top_actions::{Saved, TaskAt, move_to_state, sub_range, sub_state, walk_point};
use crate::{saved_attack, saved_move_wait, saved_reload, saved_sprite};
use genie_core::GenieError;
use genie_core::movement::move_to::MOVE_TO;
use genie_core::objects::masters::Task;
use genie_core::objects::unit::Unit;
use genie_save::generated::world::Action;

/// A farm's Make (21): its timers and task; it makes no sub-action.
pub(super) fn import_make(
    saved: &Saved<'_>,
    task_at: TaskAt<'_>,
    unit: &mut Unit,
) -> Result<(), GenieError> {
    let base = saved.base;
    let make = saved
        .action
        .rge_action_by_type
        .as_deref()
        .and_then(|t| t.rge_action_make.as_deref())
        .ok_or(GenieError::unsupported("a saved Make without its part"))?;
    if base.sub_actions.entries().next().is_some() {
        return Err(GenieError::unsupported("a saved Make with a sub-action"));
    }
    unit.actions
        .set_make(Some(genie_core::gathering::make::MakeAction {
            state: base.state,
            remaining: base.timer,
            total: make.work_timer,
            task: saved.task(task_at)?,
        }));
    Ok(())
}

/// A Hunt's or a Gather's task.
const fn hunt_task(t: &Task) -> genie_core::gathering::hunt::HuntTask {
    genie_core::gathering::hunt::HuntTask {
        id: t.id,
        range: t.work_range,
        resource: t.attribute_type,
        sprites: t.sprites,
        work_value: t.work_val1,
        productivity: t.attribute_type2,
        kind4: t.attribute_type4,
    }
}

/// The Hunt (110): its task, drop site and lead, with its Attack or `MoveTo`. Its Attack reads
/// and changes the unit's sprite from here on.
pub(super) fn import_hunt(
    saved: &Saved<'_>,
    task_at: TaskAt<'_>,
    unit: &mut Unit,
) -> Result<(), GenieError> {
    use genie_core::gathering::hunt::{
        ATTACK, DROP, FACE, GATHER, HuntAction, IDLE, LEAD, SEARCH, WALK, WALK_POINT, WALK_SEARCH,
    };
    let base = saved.base;
    let part = saved
        .action
        .tribe_action_hunt
        .as_deref()
        .ok_or(GenieError::unsupported("a saved Hunt without its own part"))?;
    if !matches!(
        base.state,
        IDLE | SEARCH | WALK | FACE | GATHER | DROP | LEAD | ATTACK | WALK_SEARCH | WALK_POINT
    ) {
        return Err(GenieError::unsupported(
            "a saved Hunt in a state the slice does not model",
        ));
    }
    let task = hunt_task(&saved.task(task_at)?);
    let mut hunt = HuntAction {
        target: base.target_id,
        state: base.state,
        point: saved.point(),
        target_class: part.target_type as i16,
        task,
        attack: None,
        move_to: None,
        walk_range: task.range,
        drop_site: part.sheep_drop_site_id as i32,
        carry_to: base.target2_id,
        timer: base.timer,
        move_wait: genie_core::movement::move_to::MoveWait::default(),
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
            hunt.attack = Some(hunt_attack(saved, sub)?);
            unit.sprite = saved.graphics.and_then(|g| saved_sprite(saved.s, g));
        }
        Some((MOVE_TO, sub)) => {
            hunt.move_to = Some(sub_state(sub)?);
            hunt.walk_range = sub_range(sub)?;
            // its wait (`+0x6c`, `+0x70`, `+0x74`)
            hunt.move_wait = saved_move_wait(sub);
        }
        Some(_) => {
            return Err(GenieError::unsupported(
                "a saved Hunt with another sub-action",
            ));
        }
    }
    unit.actions.set_hunt(Some(hunt));
    Ok(())
}

/// A Hunt's Attack. A swing past its hit: the swing's times (`+0xb0`, `+0xb4`, unsaved) are
/// what the hit left, the reload less the attack sprite's length.
fn hunt_attack(
    saved: &Saved<'_>,
    sub: &Action,
) -> Result<genie_core::combat::attack::Attack, GenieError> {
    let mut attack = saved_attack(sub, saved.graphics, &move_to_state)?;
    if attack.state == genie_core::combat::attack::SWING && !attack.pending {
        let reload = saved_reload(saved.object, saved.s, saved.objects).ok_or(
            GenieError::unsupported("a saved swing without its unit's reload"),
        )?;
        let g = attack.arms.sprite.ok_or(GenieError::unsupported(
            "a saved swing without its attack sprite",
        ))?;
        let rest = reload - f32::from(g.frames) * g.frame_duration;
        attack.swing = rest;
        attack.rest = rest.max(0.0);
    }
    Ok(attack)
}

/// The Gather (5): its stops and task, with its `MoveTo`.
pub(super) fn import_gather(
    saved: &Saved<'_>,
    task_at: TaskAt<'_>,
    unit: &mut Unit,
) -> Result<(), GenieError> {
    use genie_core::gathering::gather::{
        DONE, DROP, FACE, GatherAction, IDLE, MOVED, SEARCH, WALK, WALK_BACK, WORK,
    };
    let base = saved.base;
    let part = saved
        .action
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
    let task = hunt_task(&saved.task(task_at)?);
    let mut gather = GatherAction {
        target: base.target_id,
        carry_to: base.target2_id,
        state: base.state,
        point: saved.point(),
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
        move_wait: genie_core::movement::move_to::MoveWait::default(),
    };
    if let Some(sub) = saved.move_to_sub(GenieError::unsupported(
        "a saved Gather with another sub-action",
    ))? {
        gather.move_to = Some(sub_state(sub)?);
        // a MoveTo without a target walks to its point (a farm's stop)
        gather.walk_to = walk_point(sub);
        gather.walk_range = sub_range(sub)?;
        gather.move_wait = saved_move_wait(sub);
    }
    unit.actions.set_gather(Some(gather));
    Ok(())
}

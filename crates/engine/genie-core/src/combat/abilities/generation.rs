//! The generation tasks (action type 151): their test, what they make and the player's store.

use crate::combat::abilities::{
    BUILDING_KIND, COMBAT_KIND, ONE, Own, Pending, WORKING, WORKING_ACTIONS,
};
use crate::objects::masters::Task;
use crate::turn::Turns;
use crate::{GenieError, math::fmath};

/// What a generation task's test reads of the object's top action (`FUN_140a986f0`): its type
/// (`+0x8`), its state (`+0x18`) and its target as the world holds it (`FUN_140a78450`), None
/// when the world holds no object of its id.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TopWork {
    /// The action's type.
    pub kind: i16,
    /// The action's state.
    pub state: u8,
    /// Its target.
    pub target: Option<WorkTarget>,
}

/// What a generation task's test reads of the top action's target.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorkTarget {
    /// Its hit points (`+0x278`).
    pub hp: f32,
    /// Its master's first storage amount (`+0x610`).
    pub storage: f32,
    /// Its master's class (`+0x410`).
    pub class: i16,
}

/// Whether a generation task generates this update (`0x141207d4d`..`0x141207fbe`): an object
/// of type 70 (`+0xf`) whose task has `combat_level_flag` bit 1, or of type 80 with bit 2, does;
/// otherwise one whose master is of type 70 does while its top action is one of
/// [`WORKING_ACTIONS`] in state 7 with a target the world holds that has hit points above 0 or
/// whose master stores some (`COMISS`/`JA`, `JBE`: a NaN does not) and whose master's class is
/// the task's `object_group`, and one whose master is of type 80 does while it is built
/// (`+0xca1` 1).
#[must_use]
pub fn generates(
    task: &Task,
    kind: u8,
    master_kind: u8,
    top: Option<TopWork>,
    built: bool,
) -> bool {
    match kind {
        COMBAT_KIND if task.combat_level_flag & 2 != 0 => return true,
        BUILDING_KIND if task.combat_level_flag & 4 != 0 => return true,
        _ => {}
    }
    if master_kind == COMBAT_KIND
        && let Some(top) = top
        && WORKING_ACTIONS.contains(&top.kind)
        && top.state == WORKING
        && let Some(t) = top.target
        && (0.0 < t.hp || 0.0 < t.storage)
        && t.class == task.object_group
    {
        return true;
    }
    master_kind == BUILDING_KIND && built
}

/// What a generation task makes in an update (`0x141207fc4`): the world's dt (`world+0x164`)
/// times the task's `work_val1` times the player's attribute the task's `attribute_type2` names,
/// in that order (`MULSS`, `MULSS`).
#[must_use]
pub fn generated(dt: f32, task: &Task, multiplier: f32) -> f32 {
    dt * task.work_val1 * multiplier
}

/// The player's store of what its generation tasks made (`player+0x658`): `amount` above 0 (not
/// a NaN) joins the store of attribute `out` when that is not negative (`ADDSS`: the amount plus
/// the store); the store, or the amount alone when it did not join, once at least 1
/// (`COMISS`/`JC`: a NaN is below) goes to the player (returned, the caller's slot 0xf8) and the
/// store starts again at 0. A store the player does not have (an attribute past its count, which
/// no `.dat` task names) takes nothing.
pub fn store(amount: f32, out: i16, stores: &mut [f32]) -> Option<f32> {
    let slot = usize::try_from(out).ok();
    let mut amount = amount;
    if 0.0 < amount
        && let Some(s) = slot.and_then(|i| stores.get_mut(i))
    {
        *s += amount;
        amount = *s;
    }
    if fmath::below_or_unordered(amount, ONE) {
        return None;
    }
    let s = slot.and_then(|i| stores.get_mut(i))?;
    *s = 0.0;
    Some(amount)
}

impl Turns {
    /// A generation task of the object at `(p, i)` as `owner`'s ([`generates`], [`generated`],
    /// [`store`]): what reaches the store's threshold goes to the player (slot 0xf8 with 1 and 1,
    /// as a load's, [`crate::turn::give_load`]); the statistics (`FUN_14097daf0`, the local
    /// player's, and `FUN_140a4c020`'s telemetry) are not held. The top action's target is read
    /// as the world holds it after the update: a pending one as the update left it.
    ///
    /// # Errors
    ///
    /// The top action of a unit whose action the slice does not model, and a give to a player
    /// who shares with allies ([`crate::turn::give_load`]).
    pub(super) fn generate(
        &mut self,
        (p, i): (usize, usize),
        owner: usize,
        master: Own,
        task: &Task,
        pending: &mut Pending<'_>,
    ) -> Result<(), GenieError> {
        let unit = &self.players[p].objects[i];
        let flags = task.combat_level_flag;
        let short_cut = match master.kind {
            COMBAT_KIND => flags & 2 != 0,
            _ => flags & 4 != 0,
        };
        let top = if short_cut || master.kind != COMBAT_KIND {
            None
        } else {
            if let Some(e) = unit.unmodelled_action.clone() {
                return Err(e);
            }
            unit.top_action().map(|a| {
                // FUN_140a78450: an id below 0 names no object; the target only matters to an
                // action of the test's at work
                let works = WORKING_ACTIONS.contains(&a.kind) && a.state == WORKING;
                let target = unit
                    .top_target()
                    .filter(|&t| works && 0 <= t)
                    .and_then(|t| self.work_target(t, pending));
                TopWork {
                    kind: a.kind,
                    state: a.state,
                    target,
                }
            })
        };
        let built = master.kind == BUILDING_KIND
            && self
                .spatial
                .sight
                .as_ref()
                .and_then(|v| v.seer(unit.id))
                .and_then(|s| s.building)
                .is_some_and(|b| b.built);
        if !generates(task, master.kind, master.kind, top, built) {
            return Ok(());
        }
        let player = &mut self.players[owner];
        let multiplier = usize::try_from(task.attribute_type2)
            .ok()
            .and_then(|a| player.attribute(a))
            .unwrap_or(0.0);
        let amount = generated(self.dt_seconds, task, multiplier);
        let Some(give) = store(amount, task.attribute_type3, &mut player.generated) else {
            return Ok(());
        };
        let sharing = player.sharing;
        let mut adds = Vec::new();
        crate::turn::give_load(
            (&self.spatial.relations, sharing),
            owner,
            &mut adds,
            task.attribute_type3,
            give,
        )?;
        for (kind, amount) in adds {
            self.players[owner].add_attribute(kind, amount)?;
        }
        Ok(())
    }

    /// What a generation task's test reads of the object `id` its top action works on: a pending
    /// one as its update left it, else as the world holds it; None for an object the world does
    /// not hold.
    pub(super) fn work_target(&self, id: i32, pending: &mut Pending<'_>) -> Option<WorkTarget> {
        if let Some(v) = pending.get(id) {
            return Some(WorkTarget {
                hp: v.hp,
                storage: v.storage,
                class: v.class,
            });
        }
        let (q, u) = self.object(id)?;
        let m = u
            .own_master
            .as_deref()
            .or_else(|| self.masters.get(q).and_then(|m| m.get(u.life.master_id)))?;
        Some(WorkTarget {
            hp: u.life.hp,
            storage: m.storage.first().map_or(0.0, |s| s.1),
            class: m.class,
        })
    }
}

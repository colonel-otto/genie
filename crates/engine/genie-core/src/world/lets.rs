//! What an object's update does to the objects its actions take and let go (slots 0x178 and
//! 0x180) and to the building a counted builder leaves: each a call through the update's view
//! where the game makes it (FD-045-D07), as [`World::take`], [`World::release`] and
//! [`World::leave_builders`] do outside an update.
//!
//! The updating object itself is outside the view (its slot holds its stand-in): a call on it
//! goes to the world after the step (the Context's `takes`, `releases`, `builder_leaves`).
//!
//! [`World::take`]: crate::world::World::take
//! [`World::release`]: crate::world::World::release
//! [`World::leave_builders`]: crate::world::World::leave_builders

use crate::GenieError;
use crate::objects::masters::Master;
use crate::objects::release::{Releaser, released};
use crate::world::context::Context;
use crate::world::others::{Others, OthersRef};

/// Slot 0x178 on object `id`, which an action of `by` takes ([`World::take`] through the
/// view): a live one counts its worker ([`crate::objects::unit::Unit::taken`]; a building
/// first lists an unallied taker of a foundation with a stack unit); a sleeping one counts its
/// worker (at most 550) and wakes, the end of its player's live list and last in its tracker
/// ([`Others::wake`]). An object the world does not hold is not taken.
///
/// [`World::take`]: crate::world::World::take
pub(crate) fn take(c: &mut Context<'_>, others: &mut Others<'_>, id: i32, by: Releaser) {
    if others.is_updating(id) {
        c.takes.push((id, by));
        return;
    }
    let Some((p, _, sleeps)) = others.locate(id) else {
        return;
    };
    if sleeps {
        let (_, u) = others.get_mut(id).expect("located");
        if u.workers < 550 {
            u.workers += 1;
        }
        let _ = others.wake(id, c.tracker, (c.every_masters, c.lines));
        return;
    }
    let allied = allied(c, others.view(), by.player, p);
    let (_, u) = others.get_mut(id).expect("located");
    let listed = master(c, p, u.life.master_id).is_ok_and(|m| u.lists_taker(m, allied));
    u.taken(listed, by.id);
}

/// Slot 0x180 on object `id`, which an action of `by` lets go ([`World::release`] through the
/// view, [`released`]); a sleeping one only counts a worker less, and an object the world does
/// not hold is not let go.
///
/// # Errors
///
/// A live object of a master its owner lacks, and a notify its unit AI's queue does not take.
///
/// [`World::release`]: crate::world::World::release
pub(crate) fn release(
    c: &mut Context<'_>,
    others: &mut Others<'_>,
    id: i32,
    by: Releaser,
) -> Result<(), GenieError> {
    if others.is_updating(id) {
        c.releases.push((id, by));
        return Ok(());
    }
    let Some((p, _, sleeps)) = others.locate(id) else {
        return Ok(());
    };
    if sleeps {
        // a sleeping object (static: only those sleep) counts a worker less (FUN_141197d00);
        // asleep already (`+0x641`), it is not flagged again
        let (_, u) = others.get_mut(id).expect("located");
        if 0 < u.workers {
            u.workers -= 1;
        }
        return Ok(());
    }
    let allied = allied(c, others.view(), by.player, p);
    let (_, u) = others.get_mut(id).expect("located");
    let m = master(c, p, u.life.master_id)?;
    let (kind, listed) = (m.kind, u.lists_taker(m, allied));
    released(u, kind, listed, by)
}

/// The builder count of the building a Build on `id` counted its builder in
/// ([`World::leave_builders`] through the view, `FUN_140aaddb0`): the object itself, or for a
/// building that is an annex its main building, counts one less (`+0x9e8`); none once the world
/// no longer holds `id`.
///
/// # Errors
///
/// An object of a master its owner lacks.
///
/// [`World::leave_builders`]: crate::world::World::leave_builders
pub(crate) fn leave_builders(
    c: &mut Context<'_>,
    others: &mut Others<'_>,
    id: i32,
) -> Result<(), GenieError> {
    if others.is_updating(id) {
        c.builder_leaves.push(id);
        return Ok(());
    }
    let Some((p, f, _)) = others.locate(id) else {
        return Ok(());
    };
    let site = if master(c, p, f.life.master_id)?.kind == crate::orders::BUILDING {
        f.annex_of.unwrap_or(id)
    } else {
        id
    };
    if others.is_updating(site) {
        c.builder_leaves.push(id);
        return Ok(());
    }
    if let Some((_, b)) = others.get_mut(site) {
        let n = b.repairers_mut();
        *n = n.wrapping_sub(1);
    }
    Ok(())
}

/// What an action list's teardown left the world, applied at once: the buildings its counted
/// builders leave ([`leave_builders`]), then the targets it lets go ([`release`]), as
/// [`World::clear_actions`] applies them.
///
/// # Errors
///
/// [`leave_builders`]' and [`release`]'s.
///
/// [`World::clear_actions`]: crate::world::World::clear_actions
pub(crate) fn torn_down(
    c: &mut Context<'_>,
    others: &mut Others<'_>,
    (builder_leaves, releases): (Vec<i32>, Vec<(i32, Releaser)>),
) -> Result<(), GenieError> {
    for id in builder_leaves {
        leave_builders(c, others, id)?;
    }
    for (id, by) in releases {
        release(c, others, id, by)?;
    }
    Ok(())
}

/// Player `from` counts player `to` as its own or allied, with its allies (`FUN_141663dc0`
/// with 1): the updating player's relations from the Context, another's from the view.
fn allied(c: &Context<'_>, view: OthersRef<'_>, from: usize, to: usize) -> bool {
    let relations = if from == c.player {
        c.relations
    } else {
        let other = view.players().player(from);
        other.map_or(&[][..], |p| &p.relations[..])
    };
    crate::unit_ai::task_finder::allied(relations, from, to, true)
}

/// Player `p`'s master `id`.
fn master<'c>(c: &Context<'c>, p: usize, id: i16) -> Result<&'c Master, GenieError> {
    c.every_masters
        .get(p)
        .and_then(|m| m.get(id))
        .ok_or(crate::world::OBJECT_MASTER)
}

//! What an object's update does to the objects its actions take and let go (slots 0x178 and
//! 0x180) and to the building a counted builder leaves: each the world's call where the game
//! makes it (FD-045-D07, D28; [`World::take`], [`World::release`], [`World::leave_builders`]),
//! which runs [`Lets`] on a view of the whole world. The unit AI's side runs [`Lets`] on its
//! own view.
//!
//! The updating object itself is out of its slot (its stand-in holds it): a call on it goes to
//! the world after the step (the `Step`'s `takes`, `releases`, `builder_leaves`).

use crate::GenieError;
use crate::objects::lines::UnitLines;
use crate::objects::masters::Masters;
use crate::objects::release::{Releaser, released};
use crate::objects::unit::TAKEN_OUT;
use crate::players::tracker::Tracker;
use crate::world::others::{Others, OthersRef};
use crate::world::step::Step;
use crate::world::{Player, World};

/// What the calls on the objects let go and taken reach besides the view: the updating player
/// and its relations (whether a taker or releaser is allied), every player's masters and the
/// unit lines (a wake's tracker key), the updating player's tracker, and the calls on the
/// updating object itself, which wait for the world after the step.
pub(crate) struct Lets<'l> {
    pub(crate) player: usize,
    pub(crate) relations: &'l [u8],
    pub(crate) masters: &'l [Masters],
    pub(crate) lines: &'l UnitLines,
    pub(crate) tracker: &'l mut Tracker,
    pub(crate) takes: &'l mut Vec<(i32, Releaser)>,
    pub(crate) releases: &'l mut Vec<(i32, Releaser)>,
    pub(crate) builder_leaves: &'l mut Vec<i32>,
}

impl World {
    /// `f` with the world's [`Lets`] for player `player`'s calls and a view of every player's
    /// objects, none of them updating: [`World::take`], [`World::release`] and
    /// [`World::leave_builders`] outside an update run the update's calls on it.
    pub(crate) fn with_lets<R>(
        &mut self,
        player: usize,
        f: impl FnOnce(&mut Lets<'_>, &mut Others<'_>) -> R,
    ) -> R {
        let (before, rest) = self.players.split_at_mut(player);
        let (own, after) = rest.split_first_mut().expect("a player of the world");
        let Player {
            objects,
            sleeping,
            relations,
            tracker,
            ..
        } = own;
        let mut others = Others::new((player, TAKEN_OUT), (objects, sleeping), (before, after));
        let (mut takes, mut releases, mut builder_leaves) = (Vec::new(), Vec::new(), Vec::new());
        let mut lets = Lets {
            player,
            relations,
            masters: &self.masters,
            lines: &self.lines,
            tracker,
            takes: &mut takes,
            releases: &mut releases,
            builder_leaves: &mut builder_leaves,
        };
        f(&mut lets, &mut others)
    }
}

/// Slot 0x178 on object `id`, which an action of `by` takes, from an update: the world's take
/// ([`World::take`]); the updating object's own, which the world does not hold while it
/// updates, waits for the world after the step.
pub(crate) fn take(c: &mut Step<'_>, id: i32, by: Releaser) {
    if id == c.me {
        c.takes.push((id, by));
    } else {
        c.w.take(id, by);
    }
}

/// Slot 0x180 on object `id`, which an action of `by` lets go, from an update: the world's
/// release ([`World::release`]); the updating object's own waits for the world after the step.
///
/// # Errors
///
/// [`World::release`]'s.
pub(crate) fn release(c: &mut Step<'_>, id: i32, by: Releaser) -> Result<(), GenieError> {
    if id == c.me {
        c.releases.push((id, by));
        return Ok(());
    }
    c.w.release(id, by)
}

/// A counted builder's leave of the building a Build on `id` counted it in, from an update:
/// the world's ([`World::leave_builders`]); one on the updating object itself, or on an annex
/// whose main building it is, waits for the world after the step.
///
/// # Errors
///
/// An object of a master its owner lacks.
pub(crate) fn leave_builders(c: &mut Step<'_>, id: i32) -> Result<(), GenieError> {
    if id == c.me || c.w.counted_building(id)? == Some(c.me) {
        c.builder_leaves.push(id);
        return Ok(());
    }
    c.w.leave_builders(id)
}

/// What an action list's teardown left the world, applied at once from an update: the
/// buildings its counted builders leave ([`leave_builders`]), then the targets it lets go
/// ([`release`]), as [`World::clear_actions`] applies them.
///
/// # Errors
///
/// [`leave_builders`]' and [`release`]'s.
pub(crate) fn torn_down(
    c: &mut Step<'_>,
    (builder_leaves, releases): (Vec<i32>, Vec<(i32, Releaser)>),
) -> Result<(), GenieError> {
    for id in builder_leaves {
        leave_builders(c, id)?;
    }
    for (id, by) in releases {
        release(c, id, by)?;
    }
    Ok(())
}

impl Lets<'_> {
    /// The tracker's promotion (`FUN_14104fb20`) of the owner's unit `id`, whose master changes
    /// from `old` to `new` where the change runs ([`crate::players::tracker::Tracker::promote`]).
    pub(crate) fn promote(&mut self, id: i32, old: i16, new: i16) {
        let masters = self.masters.get(self.player);
        let key = |m| crate::players::tracker::key(masters, self.lines, m);
        self.tracker.promote(id, key(old), key(new));
    }

    /// Slot 0x178 on object `id`, which an action of `by` takes ([`World::take`] through the
    /// view): a live one counts its worker ([`crate::objects::unit::Unit::taken`]; a building
    /// first lists an unallied taker of a foundation with a stack unit); a sleeping one counts
    /// its worker (at most 550) and wakes, the end of its player's live list and last in its
    /// tracker ([`Others::wake`]). An object the world does not hold is not taken.
    ///
    /// [`World::take`]: crate::world::World::take
    pub(crate) fn take(&mut self, others: &mut Others<'_>, id: i32, by: Releaser) {
        if others.is_updating(id) {
            self.takes.push((id, by));
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
            let _ = others.wake(id, self.tracker, (self.masters, self.lines));
            return;
        }
        let allied = self.allied(others.view(), by.player, p);
        let (_, u) = others.get_mut(id).expect("located");
        let listed = u
            .master(self.masters, p)
            .is_ok_and(|m| u.lists_taker(m, allied));
        u.taken(listed, by.id);
    }

    /// Slot 0x180 on object `id`, which an action of `by` lets go ([`World::release`] through
    /// the view, [`released`]); a sleeping one only counts a worker less, and an object the
    /// world does not hold is not let go.
    ///
    /// # Errors
    ///
    /// A live object of a master its owner lacks, and a notify its unit AI's queue does not
    /// take.
    ///
    /// [`World::release`]: crate::world::World::release
    pub(crate) fn release(
        &mut self,
        others: &mut Others<'_>,
        id: i32,
        by: Releaser,
    ) -> Result<(), GenieError> {
        if others.is_updating(id) {
            self.releases.push((id, by));
            return Ok(());
        }
        let Some((p, _, sleeps)) = others.locate(id) else {
            return Ok(());
        };
        if sleeps {
            // a sleeping object (static: only those sleep) counts a worker less
            // (FUN_141197d00); asleep already (`+0x641`), it is not flagged again
            let (_, u) = others.get_mut(id).expect("located");
            if 0 < u.workers {
                u.workers -= 1;
            }
            return Ok(());
        }
        let allied = self.allied(others.view(), by.player, p);
        let (_, u) = others.get_mut(id).expect("located");
        let m = u.master(self.masters, p)?;
        let (kind, listed) = (m.kind, u.lists_taker(m, allied));
        released(u, kind, listed, by)
    }

    /// The builder count of the building a Build on `id` counted its builder in
    /// ([`World::leave_builders`] on a view, `FUN_140aaddb0`): the object itself, or for
    /// a building that is an annex its main building, counts one less (`+0x9e8`); none once the
    /// world no longer holds `id`.
    ///
    /// # Errors
    ///
    /// An object of a master its owner lacks.
    ///
    /// [`World::leave_builders`]: crate::world::World::leave_builders
    pub(crate) fn leave_builders(
        &mut self,
        others: &mut Others<'_>,
        id: i32,
    ) -> Result<(), GenieError> {
        if others.is_updating(id) {
            self.builder_leaves.push(id);
            return Ok(());
        }
        let Some((p, f, _)) = others.locate(id) else {
            return Ok(());
        };
        let site = if f.master(self.masters, p)?.kind == crate::orders::BUILDING {
            f.annex_of.unwrap_or(id)
        } else {
            id
        };
        if others.is_updating(site) {
            self.builder_leaves.push(id);
            return Ok(());
        }
        if let Some((_, b)) = others.get_mut(site) {
            let n = b.repairers_mut();
            *n = n.wrapping_sub(1);
        }
        Ok(())
    }

    /// What an action list's teardown left the world, applied at once: the buildings its
    /// counted builders leave ([`Lets::leave_builders`]), then the targets it lets go
    /// ([`Lets::release`]), as [`World::clear_actions`] applies them.
    ///
    /// # Errors
    ///
    /// [`Lets::leave_builders`]' and [`Lets::release`]'s.
    ///
    /// [`World::clear_actions`]: crate::world::World::clear_actions
    pub(crate) fn torn_down(
        &mut self,
        others: &mut Others<'_>,
        (builder_leaves, releases): (Vec<i32>, Vec<(i32, Releaser)>),
    ) -> Result<(), GenieError> {
        for id in builder_leaves {
            self.leave_builders(others, id)?;
        }
        for (id, by) in releases {
            self.release(others, id, by)?;
        }
        Ok(())
    }

    /// Player `from` counts player `to` as its own or allied, with its allies (`FUN_141663dc0`
    /// with 1): the updating player's relations, another's from the view.
    fn allied(&self, view: OthersRef<'_>, from: usize, to: usize) -> bool {
        let relations = if from == self.player {
            self.relations
        } else {
            let other = view.players().player(from);
            other.map_or(&[][..], |p| &p.relations[..])
        };
        crate::unit_ai::task_finder::allied(relations, from, to, true)
    }
}

//! A building's queue cancelled: the whole queue, its units and researches refunded, and
//! `UNIT_ORDER`'s cancel of one, five or some.

use crate::game_status::{
    CURRENT_AGE, ONE_UNIT_AGE, ONE_UNIT_MODE, VILLAGER_FEMALE, VILLAGER_MALE,
};
use crate::production::{Making, refund, refund_research};
use crate::turn::Turns;
use crate::unit_ai::GenieError;

impl Turns {
    /// `FUN_1411cff00`, the cancel of the whole queue of the building at `idx` of player `p`,
    /// entry by entry. A unit entry: each unit gets its refund ([`refund`]) and the tracker its
    /// place back (the counter one less, [`crate::tracker::Tracker::unplace`]); attribute 80
    /// lowers by the population times the count (slot 0xf8), and the tracker's counter by the
    /// count once more. A research entry, for a player with a tech tree: its count's refund
    /// ([`Turns::refund_researches`]). Then the interface's `+0xc02` (the local player's event
    /// 0x7d) is left; the entries go, the production list's action goes
    /// ([`Turns::end_production_action`]), the change counter grows and the total is 0.
    pub(crate) fn cancel_queue(&mut self, p: usize, idx: usize) -> Result<(), GenieError> {
        let Some(production) = self.players[p].objects[idx].production.as_ref() else {
            return Ok(());
        };
        let entries = production.queue.entries().to_vec();
        for e in &entries {
            if e.tech == -1 {
                self.cancel_units(p, e, e.count)?;
                self.players[p].tracker.count_queued(-e.count);
            } else if !self.players[p].techs.is_empty() {
                self.refund_researches(p, idx, e, e.count)?;
            }
        }
        if let Some(production) = self.players[p].objects[idx].production.as_mut() {
            production.queue.count = 0;
        }
        self.end_production_action(p, idx)?;
        if let Some(production) = self.players[p].objects[idx].production.as_mut() {
            production.queue.changes = production.queue.changes.wrapping_add(1);
            production.queue.total = 0;
        }
        Ok(())
    }

    /// `count` units of a unit entry `e` of player `p` cancelled (`FUN_1411cff00`,
    /// `FUN_1411d61e0`): each its refund ([`refund`]) and its tracker place (the counter one
    /// less, [`crate::tracker::Tracker::unplace`]); then attribute 80 lowers by the population
    /// times the count (slot 0xf8 with −population × count).
    pub(super) fn cancel_units(
        &mut self,
        p: usize,
        e: &crate::production::Entry,
        count: i16,
    ) -> Result<(), GenieError> {
        use crate::production::{QUEUED, population};
        let class = self.tracker_key(p, e.master).class;
        let master = self.masters.get(p).and_then(|m| m.get(e.master)).cloned();
        let owner = &mut self.players[p];
        for _ in 0..count.max(0) {
            if let Some(m) = &master {
                refund(m, &e.costs, owner)?;
            }
            owner.tracker.count_queued(-1);
            owner.tracker.unplace(e.master, class);
        }
        let pop = master.as_ref().map_or(0.0, |m| population(&m.storage));
        owner.add_attribute(QUEUED, -pop * f32::from(count))
    }

    /// `count` researches of a research entry `e` at the building at `idx` of player `p`
    /// cancelled (`FUN_1411cff00`, `FUN_1411d61e0`): the refund of what the entry paid times
    /// the count (`FUN_1413d15e0`, [`refund_research`]; the technology in state 1 with no work
    /// done), the costs the building paid back to its store (`FUN_1411c7a00`: each of the
    /// `.dat`'s cost entries paid by the building, flag 2, its amount once), the technology out of
    /// the building's disabled list (`FUN_1411c5250`) and its researches queued (`+0x58`) lower
    /// by the count.
    pub(super) fn refund_researches(
        &mut self,
        p: usize,
        idx: usize,
        e: &crate::production::Entry,
        count: i16,
    ) -> Result<(), GenieError> {
        // the entry's technology is one of the tree: RESEARCH queued it, the load read it
        let Some(t) = usize::try_from(e.tech)
            .ok()
            .filter(|&t| t < self.players[p].techs.len())
        else {
            return Ok(());
        };
        let owner = &mut self.players[p];
        refund_research(&e.costs, count, owner)?;
        let state = &mut owner.techs[t];
        state.state = 1;
        state.research_done = 0.0;
        let building = &mut owner.objects[idx];
        if let Some(def) = self.techs.get(t) {
            for c in def.costs.iter().filter(|c| c.paid == 2) {
                match building.store.iter_mut().find(|s| s.0 == c.attribute) {
                    Some(s) => s.1 += f32::from(c.amount),
                    None => building
                        .store
                        .push((c.attribute, f32::from(c.amount) + 0.0)),
                }
            }
        }
        if let Some(production) = building.production.as_mut() {
            production.enable_tech(e.tech);
        }
        let pending = &mut owner.techs[t].pending;
        *pending = pending.wrapping_sub(count);
        Ok(())
    }

    /// `UNIT_ORDER`'s action 4 (`FUN_14161e370` case 4 -> `FUN_1411d66b0`) at the building at
    /// `idx` of player `p`: the entry of the `index`-th unit
    /// ([`crate::production::Queue::entry_of_unit`]; none past the last unit) is cancelled.
    /// A unit entry with `all` (the command's `+0x1d`) gives up to five of its kind back
    /// ([`Turns::cancel_five`]); without, one unit ([`Turns::give_back_one`]). A research entry,
    /// for a player with a tech tree: for the first unit the list's first action (the research)
    /// loses its progress (`+0x70`) and the technology its work done; then one research, five
    /// with `all`, goes ([`Turns::cancel_some`] with the entry and its technology). Otherwise
    /// (one unit, or a research entry of a player without a tech tree, which gives nothing
    /// back) the entry's count, the total and the change counter lower, an emptied entry going
    /// ([`crate::production::Queue::close`]), and for the first unit (index 0) the production
    /// list's action goes ([`Turns::end_production_action`]; the interface's `+0xc02` and the
    /// local player's event 0x7d are left).
    pub(crate) fn cancel_queued(
        &mut self,
        p: usize,
        idx: usize,
        index: u32,
        all: bool,
    ) -> Result<(), GenieError> {
        let Some(production) = self.players[p].objects[idx].production.as_ref() else {
            return Ok(());
        };
        let Some(e) = production.queue.entry_of_unit(index) else {
            return Ok(());
        };
        let entry = production.queue.slots[e];
        if entry.tech == -1 {
            if all {
                return self.cancel_five(p, idx, index, e);
            }
            self.give_back_one(p, &entry)?;
        } else if !self.players[p].techs.is_empty() {
            if index == 0 {
                let owner = &mut self.players[p];
                if let Some(Some(action)) = owner.objects[idx]
                    .production
                    .as_mut()
                    .map(|q| q.action.as_mut())
                {
                    match action {
                        Making::Research(r) => r.progress = 0.0,
                        // the game writes the make action's +0x70, its unique id
                        Making::Unit(m) => m.unique_id = 0,
                    }
                    if let Some(state) = usize::try_from(entry.tech)
                        .ok()
                        .and_then(|t| owner.techs.get_mut(t))
                    {
                        state.research_done = 0.0;
                    }
                }
            }
            let count = i16::from(all) * 4 + 1;
            return self.cancel_some(p, idx, (-1, count), e as i32, entry.tech);
        }
        let owner = &mut self.players[p];
        let queue = &mut owner.objects[idx].production.as_mut().expect("above").queue;
        queue.slots[e].count -= 1;
        queue.total -= 1;
        queue.changes = queue.changes.wrapping_add(1);
        if queue.slots[e].count < 1 {
            queue.close(e, true);
        }
        if index == 0 {
            self.end_production_action(p, idx)?;
        }
        Ok(())
    }

    /// `FUN_1411d66b0`'s one unit of entry `entry` given back to player `p`: its refund
    /// ([`refund`]), the tracker's counter one less and its place back, attribute 80 lower by
    /// its population (slot 0xf8).
    pub(super) fn give_back_one(
        &mut self,
        p: usize,
        entry: &crate::production::Entry,
    ) -> Result<(), GenieError> {
        let class = self.tracker_key(p, entry.master).class;
        let master = self
            .masters
            .get(p)
            .and_then(|m| m.get(entry.master))
            .cloned();
        let owner = &mut self.players[p];
        if let Some(m) = &master {
            refund(m, &entry.costs, owner)?;
        }
        owner.tracker.count_queued(-1);
        owner.tracker.unplace(entry.master, class);
        let pop = master
            .as_ref()
            .map_or(0.0, |m| crate::production::population(&m.storage));
        owner.add_attribute(crate::production::QUEUED, -pop)
    }

    /// `FUN_1411d66b0`'s cancel of up to five of a kind, from the unit entry `e` of the
    /// `index`-th unit of the queue of the building at `idx` of player `p`: while the entry at
    /// `e` is a unit entry with units of the first entry's master or of a villager's (83, 293:
    /// the two sexes count as one kind), at most what is left of five go
    /// ([`Turns::cancel_some`] with its master, from `index`). When the entry at `e` is then a
    /// research's, or `e` is past the last entry, `e` steps back one; the loop ends once five
    /// went, the queue is empty, or the entry at `e` is a research's, empty or of another
    /// kind. A step back from the first entry reads before the buffer in the game (heap memory
    /// the simulation does not hold): taken as no entry of the kind, which ends the loop.
    pub(super) fn cancel_five(
        &mut self,
        p: usize,
        idx: usize,
        index: u32,
        e: usize,
    ) -> Result<(), GenieError> {
        fn queue(t: &Turns, p: usize, idx: usize) -> &crate::production::Queue {
            &t.players[p].objects[idx]
                .production
                .as_ref()
                .expect("the caller's production")
                .queue
        }
        let first = queue(self, p, idx).slots[e];
        let kind = first.master;
        let mut e = e as i32;
        let mut left = first.count.min(5);
        let mut master = kind;
        let mut gone: u16 = 0;
        let mut n = queue(self, p, idx).count;
        loop {
            if n == 0 {
                return Ok(());
            }
            let Some(slot) = usize::try_from(e)
                .ok()
                .map(|e| queue(self, p, idx).slots[e])
            else {
                return Ok(());
            };
            if slot.tech != -1 || slot.count == 0 {
                return Ok(());
            }
            if master != kind && master != VILLAGER_MALE && master != VILLAGER_FEMALE {
                return Ok(());
            }
            let take = if left <= slot.count { left } else { slot.count };
            self.cancel_some(p, idx, (master, take), index as i32, -1)?;
            gone = gone.wrapping_add(take as u16);
            let q = queue(self, p, idx);
            let at = q.slots[e as usize];
            n = q.count;
            if at.tech != -1 || i32::from(n) - 1 < e {
                e -= 1;
            }
            if 0 < n {
                left = 5 - gone as i16;
                // before the buffer: no master of the kind
                master = usize::try_from(e).map_or(-1, |e| q.slots[e].master);
            } else {
                left = take;
            }
            if 4 < gone as i16 {
                return Ok(());
            }
        }
    }

    /// `FUN_1411d61e0`, the cancel of `count` of master `master` (or of technology `tech`, not
    /// −1) in the queue of the building at `idx` of player `p`, from entry `from` (held to the
    /// last). A research: the entry at `from` gives back what its researches paid
    /// ([`Turns::refund_researches`], its count at most the entry's). A unit: the first entry
    /// at `from` or before it of that master (none: nothing) gives back its units, at most its
    /// count ([`Turns::cancel_units`]), and the tracker's counter lowers by the count once more.
    /// The entry's count, the total and the change counter lower; an emptied entry goes
    /// ([`crate::production::Queue::close`]; a research's leaves the train times in their
    /// slots). For the first entry the production list's action goes
    /// ([`Turns::end_production_action`]; the interface's `+0xc02` and the local player's event
    /// 0x7d are left) unless a villager's entry went, the cancel did not start at the first
    /// unit, and a villager's entry with units followed it (the make action goes on for the
    /// other sex).
    pub(crate) fn cancel_some(
        &mut self,
        p: usize,
        idx: usize,
        (master, count): (i16, i16),
        from: i32,
        tech: i16,
    ) -> Result<(), GenieError> {
        let Some(production) = self.players[p].objects[idx].production.as_ref() else {
            return Ok(());
        };
        let q = &production.queue;
        if q.count == 0 {
            return Ok(());
        }
        let start = from.min(i32::from(q.count) - 1);
        if start < 0 {
            return Ok(());
        }
        let found = if tech == -1 {
            (0..=start as usize)
                .rev()
                .find(|&i| q.slots[i].master == master)
        } else {
            Some(start as usize)
        };
        let Some(i) = found else {
            return Ok(());
        };
        let entry = q.slots[i];
        let take = count.min(entry.count);
        if tech == -1 {
            self.cancel_units(p, &entry, take)?;
            self.players[p].tracker.count_queued(-take);
        } else {
            let e = crate::production::Entry { tech, ..entry };
            self.refund_researches(p, idx, &e, take)?;
        }
        let queue = &mut self.players[p].objects[idx]
            .production
            .as_mut()
            .expect("above")
            .queue;
        queue.slots[i].count = queue.slots[i].count.wrapping_sub(take);
        queue.total = queue.total.wrapping_sub(take);
        queue.changes = queue.changes.wrapping_add(1);
        if 0 < queue.slots[i].count {
            return Ok(());
        }
        let villager = |m: i16| m == VILLAGER_MALE || m == VILLAGER_FEMALE;
        // the game reads the next slot before the move; it matters for the first entry alone
        let pair = tech == -1
            && i == 0
            && villager(queue.slots[0].master)
            && queue.slots.get(1).is_some_and(|s| villager(s.master));
        queue.close(i, tech == -1);
        if i != 0 {
            return Ok(());
        }
        if tech == -1 && start != 0 && pair && 0 < queue.slots[0].count {
            return Ok(());
        }
        self.end_production_action(p, idx)
    }

    /// `FUN_14161e370` case 4's tail after the cancel at the building at `idx` of player `p`:
    /// in game mode 16, with the player's attribute 6 at most 1 (`COMISS`, `JC`: a NaN is not),
    /// the master of the queue's slot at the unit index `index` (read as a slot's index, after
    /// the cancel) keys a military class ([`crate::tracker::Key::military`]): the player's flag
    /// of a military unit queued in that mode (`+0x1378`) clears. An index past the buffer
    /// reads heap memory the simulation does not hold: taken as no master.
    pub(crate) fn cancel_tail(
        &mut self,
        p: usize,
        idx: usize,
        index: u32,
    ) -> Result<(), GenieError> {
        if self.settings.game_mode != ONE_UNIT_MODE {
            return Ok(());
        }
        let attribute = crate::tech::Attributes::get(&self.players[p], CURRENT_AGE)?;
        if crate::math::fmath::below_or_unordered(ONE_UNIT_AGE, attribute) {
            return Ok(());
        }
        let master = self.players[p].objects[idx]
            .production
            .as_ref()
            .and_then(|q| q.queue.slots.get(index as i32 as usize))
            .map(|s| s.master);
        if let Some(master) = master
            && self.tracker_key(p, master).military
        {
            self.players[p].one_military_queued = false;
        }
        Ok(())
    }
}

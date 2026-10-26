//! A building's production list and its make action (`TRIBE_Action_Make_Obj`).

use crate::production::spawn::MAKE_ACTION;
use crate::production::{IN_PRODUCTION, MakeObj, Making, QUEUED, housed, population};
use crate::unit_ai::{ACTION_COMPLETED, GenieError, Notify};
use crate::world::Turns;

/// The make action's type (`+0x8`, 0x66, `TRIBE_Action_Make_Obj`).
pub(super) const MAKE_OBJ: i16 = 0x66;
/// The research action's type (0x67, `TRIBE_Action_Make_Tech`).
pub(super) const MAKE_TECH: i16 = 0x67;

/// The attribute that scales research times above 0 (`+0x2b0`: 86).
pub(super) const RESEARCH_TIME_MODIFIER: i16 = 86;
/// The attribute naming the world's effect a player gets after each research (`+0x868`: 269).
pub(super) const WORLD_EFFECT: i16 = 269;

impl Turns {
    /// The building update's production step (`FUN_1411e7dc0`, after its combat update) for the
    /// building at `i` of player `p`. A disabled queue (`+0xbf6`) or an empty one does nothing.
    /// For a player not of type 3 a top action of the building's own (`FUN_141227a00`) that is a
    /// make, a research or of types 0x7b to 0x7d sets `+0xc01` and nothing more happens; else
    /// `+0xc01` clears. Without an action in its production list the first entry's comes: for a
    /// unit the population test first ([`housed`]): a housed building waits (its reason
    /// `+0xc02`, 0x20 or 4, and the events 0x7c and 0x7d are the interface's), a computer
    /// player's cancels its queue ([`Turns::cancel_queue`]); then the make action
    /// ([`MakeObj::for_entry`]; its constructor `FUN_140acb140` gives the building's unit AI the
    /// action 619, `FUN_140c64b50`; the headroom's sound and event 0xa9 are the interface's), or
    /// the research's ([`Turns::make_research`]). The list's add (`FUN_140a977e0`) starts it
    /// for a building in a state below 3 (slot 0x28): a make action of a master the player has
    /// (`FUN_140acf0a0`) adds its population to attribute 81 and takes state 6, one of a master
    /// it lacks stays in state 0 ("Canceling Train #1a"); a building in state 3 or more deletes
    /// it at once ([`Turns::end_production_action`]). Then the list's update (`FUN_140a992b0`,
    /// the first action's slot 0x30; an empty list answers done): the make action's
    /// ([`Turns::make_step`]) or the research's ([`Turns::research_step`]). Done (1) or failed
    /// (2, 3): after a research of the shared kind the technology joins the building's lists
    /// ([`crate::production::Production::note_shared_research`]); the action goes ([`Turns::end_production_action`])
    /// and the first entry lets one go: a unit's population leaves attribute 80
    /// ([`crate::production::Queue::release_first`]).
    pub(crate) fn produce(
        &mut self,
        p: usize,
        i: usize,
        dt_seconds: f32,
    ) -> Result<(), GenieError> {
        let unit = &self.players[p].objects[i];
        let Some(production) = unit.production.as_ref() else {
            return Ok(());
        };
        if !production.queue.enabled || production.queue.count == 0 {
            return Ok(());
        }
        let busy = !self.players[p].computer
            && unit
                .top_action_kind()
                .is_some_and(|k| matches!(k, MAKE_OBJ | MAKE_TECH | 0x7b..=0x7d));
        if let Some(production) = self.players[p].objects[i].production.as_mut() {
            production.queue.waiting = busy;
        }
        if busy {
            return Ok(());
        }
        let production = self.players[p].objects[i]
            .production
            .as_ref()
            .expect("above");
        if production.action.is_none() {
            let first = *production.queue.first().expect("an entry");
            let making = if first.tech == -1 {
                let pop = self
                    .masters
                    .get(p)
                    .and_then(|m| m.get(first.master))
                    .map_or(0.0, |m| population(&m.storage));
                if housed(&self.players[p].attributes, pop)? {
                    if self.players[p].computer {
                        return self.cancel_queue(p, i);
                    }
                    return Ok(());
                }
                if let Some(ai) = self.players[p].objects[i].ai.as_mut() {
                    ai.set_action(MAKE_ACTION);
                }
                Making::Unit(MakeObj::for_entry(&first))
            } else {
                Self::make_research(&first)
            };
            self.start_production_action(p, i, making)?;
        }
        let code = self.update_production_action(p, i, dt_seconds)?;
        if !matches!(code, 1..=3) {
            return Ok(());
        }
        if code == 1
            && let Some(tech) = self.players[p].objects[i]
                .production
                .as_ref()
                .and_then(|q| q.queue.first())
                .map(|e| e.tech)
                .filter(|&t| 0 <= t)
            && self
                .techs
                .get(tech as usize)
                .is_some_and(|d| d.kind == crate::technology::SHARED_KIND)
            && (tech as usize) < self.players[p].techs.len()
            && let Some(production) = self.players[p].objects[i].production.as_mut()
        {
            production.note_shared_research(tech);
        }
        self.end_production_action(p, i)?;
        let Some(first) = self.players[p].objects[i]
            .production
            .as_ref()
            .and_then(|q| q.queue.first().copied())
        else {
            return Ok(());
        };
        if first.tech == -1 {
            let pop = self
                .masters
                .get(p)
                .and_then(|m| m.get(first.master))
                .map_or(0.0, |m| population(&m.storage));
            self.players[p].add_attribute(QUEUED, -pop)?;
        }
        if let Some(production) = self.players[p].objects[i].production.as_mut() {
            production.queue.release_first();
        }
        Ok(())
    }

    /// The list's add (`FUN_140a977e0`) of `making` to the production list of the building at
    /// `i` of player `p`: for a building in a state below 3 it joins the list and starts
    /// (slot 0x28 with 1). A make action (`FUN_140acf0a0`) of a master the player has adds its
    /// population to attribute 81 and takes state 6; one of a master it lacks stays in state 0
    /// ("Canceling Train #1a"). (The building's update makes it paid, so the payment test of an
    /// unpaid one never runs here.) A research (`0x140ad1d80`): its start test
    /// ([`crate::technology::TechState::start_test`] against what the first entry paid) puts the
    /// technology in state 2 at the building's master and the action in state 6, with no work
    /// done; a failed test (its refund, `FUN_1413d15e0`) is not modelled. A building in state 3
    /// or more deletes the action unstarted (its destructor runs, [`Turns::destroy_action`]).
    pub(super) fn start_production_action(
        &mut self,
        p: usize,
        i: usize,
        making: Making,
    ) -> Result<(), GenieError> {
        use crate::technology::Attributes;
        if 3 <= self.players[p].objects[i].life.state {
            return self.destroy_action(p, i, making);
        }
        let mut making = making;
        match &mut making {
            Making::Unit(make) => {
                if let Some(m) = self.masters.get(p).and_then(|m| m.get(make.master)) {
                    let pop = population(&m.storage);
                    self.players[p].add_attribute(IN_PRODUCTION, pop)?;
                    make.state = MakeObj::MAKING;
                }
            }
            Making::Research(r) => {
                let (t, def) = self.research_tech(p, r.tech)?;
                let owner = &self.players[p];
                let get = |k: i16| owner.get(k);
                let factors = crate::technology::factors(r.tech, &get)?;
                let paid = owner.objects[i]
                    .production
                    .as_ref()
                    .and_then(|q| q.queue.first())
                    .map_or(crate::production::NO_COSTS, |e| e.costs);
                if !owner.techs[t].start_test(def, factors, &paid, &get)? {
                    return Err(GenieError::unsupported(
                        "a research whose start test fails (its refund)",
                    ));
                }
                let master = owner.objects[i].life.master_id;
                let state = &mut self.players[p].techs[t];
                state.state = 2;
                state.building_master = master;
                r.state = MakeObj::MAKING;
                r.progress = 0.0;
            }
        }
        if let Some(production) = self.players[p].objects[i].production.as_mut() {
            production.action = Some(making);
        }
        Ok(())
    }

    /// The production list's update (`FUN_140a992b0`) of the building at `i` of player `p`:
    /// the first action's slot 0x30, or 1 for an empty list.
    pub(super) fn update_production_action(
        &mut self,
        p: usize,
        i: usize,
        dt_seconds: f32,
    ) -> Result<u8, GenieError> {
        let action = self.players[p].objects[i]
            .production
            .as_ref()
            .and_then(|q| q.action);
        match action {
            None => Ok(1),
            Some(Making::Unit(_)) => self.make_step(p, i, dt_seconds),
            Some(Making::Research(_)) => self.research_step(p, i, dt_seconds),
        }
    }

    /// The make action's update (`0x140ad0490`) at the building at `i` of player `p`: in state
    /// 1 it is done; in state 6 the building's work rate (its powerup, slot 0x6e8, plus its
    /// master's `work_rate`) times dt adds to its progress, and once its train time
    /// ([`MakeObj::time`]) is reached (`COMISS`, `JC`: a NaN waits) its unit spawns
    /// ([`Turns::spawn`], which a computer player's unit does not reach) and it is done; any
    /// other state waits. Not modelled: a computer player's handicaps (`FUN_141659cf0`, the
    /// difficulty's factors on the rate), the game's instant work (`+0xc14`,
    /// `FUN_140c884c0`, 0 as the tick oracle holds it), the spot search's failure (state 10's
    /// timer, `FUN_140ace630`).
    pub(super) fn make_step(
        &mut self,
        p: usize,
        i: usize,
        dt_seconds: f32,
    ) -> Result<u8, GenieError> {
        let building = &self.players[p].objects[i];
        let Some(Some(Making::Unit(make))) = building.production.as_ref().map(|q| q.action) else {
            unreachable!("the caller's make action");
        };
        match make.state {
            MakeObj::DONE => return Ok(1),
            MakeObj::MAKING => {}
            _ => return Ok(0),
        }
        let site = self.master_of(p, building.life.master_id)?;
        let rate = building.work_rate_powerup + site.work_rate;
        let made = self.masters.get(p).and_then(|m| m.get(make.master));
        let time = f32::from(make.time(made, building.life.master_id));
        let progress = rate * dt_seconds + make.progress;
        if let Some(Some(Making::Unit(m))) = self.players[p].objects[i]
            .production
            .as_mut()
            .map(|q| q.action.as_mut())
        {
            m.progress = progress;
        }
        if crate::math::fmath::below_or_unordered(progress, time) {
            return Ok(0);
        }
        let pop = made.map_or(0.0, |m| population(&m.storage));
        self.spawn(p, i, make.master, pop)?;
        if let Some(Some(Making::Unit(m))) = self.players[p].objects[i]
            .production
            .as_mut()
            .map(|q| q.action.as_mut())
        {
            m.state = MakeObj::DONE;
        }
        Ok(1)
    }

    /// The production list's first action goes (`FUN_140a98e20`, and `FUN_140a98620` for the
    /// list of one the building's update makes) at the building at `i` of player `p`: its
    /// destructor ([`Turns::destroy_action`]).
    pub(crate) fn end_production_action(&mut self, p: usize, i: usize) -> Result<(), GenieError> {
        let Some(making) = self.players[p].objects[i]
            .production
            .as_mut()
            .and_then(|q| q.action.take())
        else {
            return Ok(());
        };
        self.destroy_action(p, i, making)
    }

    /// A production action's destructor at the building at `i` of player `p`. The make
    /// action's (`FUN_140ace3f0`): in state 6 or 2, while the game is not over, its unit's
    /// population leaves attribute 81 and, made without the queue's payment (`+0x60` clear),
    /// the master's paid costs come back ([`crate::production::refund_master`]); the building's
    /// unit AI is told the make action completed (0x1fa with 0x26b). The research's: its
    /// destructor ([`Turns::research_destroyed`]).
    pub(super) fn destroy_action(
        &mut self,
        p: usize,
        i: usize,
        making: Making,
    ) -> Result<(), GenieError> {
        let make = match making {
            Making::Unit(make) => make,
            Making::Research(r) => return self.research_destroyed(p, i, &r),
        };
        if matches!(make.state, MakeObj::MAKING | 2) && self.game_state == 0 {
            let made = self.masters.get(p).and_then(|m| m.get(make.master));
            let pop = made.map_or(0.0, |m| population(&m.storage));
            let refund = (!make.paid).then(|| made.cloned()).flatten();
            let owner = &mut self.players[p];
            owner.add_attribute(IN_PRODUCTION, -pop)?;
            if let Some(m) = refund {
                crate::production::refund_master(&m, 1, owner)?;
            }
        }
        let building = &mut self.players[p].objects[i];
        let (id, state) = (building.id, building.life.state);
        if let Some(ai) = building.ai.as_mut() {
            ai.post(
                Notify::own(id, ACTION_COMPLETED, [MAKE_ACTION, 0, 0]),
                state,
            )?;
        }
        Ok(())
    }
}

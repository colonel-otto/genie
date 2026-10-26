//! A research's completion (`FUN_1413cdcd0`): its effect, the technologies that depend on
//! it, a built building's technologies, the cost modifier and the disable and enable commands.

use crate::GenieError;
use crate::masters::master_type;
use crate::technology::EffectCommand;
use crate::technology::effect::{
    ATTRIBUTE_39, MODE_TESTED_MASTER, MODE_WITHOUT_0X26D, REENABLED_KIND, RESEARCH_SPENT,
    RESEARCHED, TECH_DISABLE, TECHS_RESEARCHED, kind,
};
use crate::turn::Turns;

impl Turns {
    /// `FUN_1413cdcd0` for player `p`'s technology `t` (`param_3` 0) researched at a building of
    /// master `building`: a technology not in state 0 to 2 is not done (the game's overrides
    /// are not held). Else it takes state 3, gives its effect (its override's, else its own;
    /// [`Turns::do_effect`]) and updates the technologies that depend on it
    /// ([`Turns::update_dependents`]); researched at a location with a button, it counts in
    /// attribute 0x15 and its listed costs (`FUN_1413d4a00`, the amounts summed) in attribute
    /// 0x63. Last, an age (101 to 103) adds its event to the player's history (`FUN_141560ae0`
    /// with 0, 1 or 2, [`crate::history::History::add_event`]), which the AI's age time reads.
    /// Not held: the local player's statistics and achievements (`FUN_1409644a0` among them).
    /// Not modelled: a research paid in relics (`FUN_1413d0e70`), an effect the slice cannot run without its
    /// building ([`Turns::research_effect_known`]) and the civ's disabled technologies
    /// (`FUN_140ea93a0`; such a technology is never available to be researched).
    pub(crate) fn complete_research(
        &mut self,
        p: usize,
        t: usize,
        building: Option<i16>,
    ) -> Result<bool, GenieError> {
        use crate::technology::Attributes;
        let def = self.techs[t].clone();
        let state = &self.players[p].techs[t];
        if !(0..=2).contains(&state.state) {
            return Ok(false);
        }
        if def.costs[0].attribute == 7 {
            return Err(GenieError::unsupported(
                "a research paid in relics (FUN_1413d0e70)",
            ));
        }
        self.research_effect_known(p, t)?;
        self.players[p].techs[t].state = 3;
        self.do_effect(p, &def.effect, None)?;
        self.update_dependents(p, t)?;
        let owner = &self.players[p];
        let state = &owner.techs[t];
        let location = building.map_or_else(|| state.locations.first(), |b| state.location(b));
        if location.is_some_and(|l| 0 <= l.building && l.button != 0) {
            let get = |k: i16| owner.get(k);
            let factors = crate::technology::factors(t as i16, &get)?;
            let listed = state.listed_costs(&def, factors);
            let spent = listed.iter().map(|&(_, a)| i32::from(a)).sum::<i32>() as f32;
            let owner = &mut self.players[p];
            owner.add_attribute(TECHS_RESEARCHED, 1.0)?;
            owner.add_attribute(RESEARCH_SPENT, spent)?;
        }
        let age = match t {
            101 => Some(crate::history::FEUDAL_AGE),
            102 => Some(crate::history::CASTLE_AGE),
            103 => Some(crate::history::IMPERIAL_AGE),
            _ => None,
        };
        if let Some(kind) = age {
            let now = self.world_time;
            self.players[p].history.add_event(kind, 0.0, 0.0, 0, now);
        }
        Ok(true)
    }

    /// Whether player `p`'s technology `t` gives an effect this file runs (`FUN_1413ce9b0`): its
    /// override's when it has one (the world's effect table `+0x128`, not held: only its own
    /// effect is known), with the research's building as the effect's object (`FUN_1413cdcd0`'s
    /// `param_4`, which the callers do not pass yet): an effect that reaches its object (an
    /// enable for one building, an upgrade of the object, the object's kinds 203 to 206, a
    /// disable of a building's own technology) would differ without it.
    pub(super) fn research_effect_known(&self, p: usize, t: usize) -> Result<(), GenieError> {
        let def = &self.techs[t];
        let state = &self.players[p].techs[t];
        let shared = |c: &EffectCommand| {
            // FUN_1413d1020 tests the whole id
            usize::try_from(crate::checkpoint::cvttss2si(c.d))
                .ok()
                .and_then(|d| self.techs.get(d))
                .is_some_and(|d| d.kind == crate::technology::SHARED_KIND)
        };
        let reaches_object = def.effect.iter().any(|c| match c.kind {
            kind::ENABLE => c.c == 1,
            kind::UPGRADE => c.c == 2,
            TECH_DISABLE => shared(c),
            k => (kind::OBJECT_SET..=kind::OBJECT_UPGRADE).contains(&k),
        });
        if state.effect_override != -1 && state.effect_override != def.effect_id {
            return Err(GenieError::unsupported("a technology's effect override"));
        }
        if reaches_object {
            return Err(GenieError::unsupported(
                "a research's effect on its building (FUN_1413cdcd0's param_4, not passed)",
            ));
        }
        Ok(())
    }

    /// A completed building's technology `t` for player `p` (the end of `FUN_1411dbb70`'s
    /// `set_state(2)`): [`Turns::complete_research`] with `param_3` 0 and no building, so the
    /// technology's first location decides its statistics; a technology beyond the player's tree
    /// is not researched.
    pub(crate) fn research_built(&mut self, p: usize, t: i16) -> Result<(), GenieError> {
        let tree = self.players[p].techs.len();
        if tree == 0 || self.techs.len() != tree {
            return Err(GenieError::unsupported(
                "a completion's technology without the player's tech tree",
            ));
        }
        if let Some(t) = usize::try_from(t).ok().filter(|&t| t < tree) {
            let _ = self.complete_research(p, t, None)?;
        }
        Ok(())
    }

    /// After a built building of `master` died (`FUN_1411dbb70`): unless another of player `p`'s
    /// live objects of the master stands (state 2, `FUN_141166ab0`), the master's technology
    /// `tech` in a state above 0 turns to 0 and its dependents are updated (`FUN_1413d4ec0`, then
    /// `FUN_1413cce70`, [`Turns::update_dependents`]).
    pub(crate) fn withdraw_tech(
        &mut self,
        p: usize,
        master: i16,
        tech: i16,
    ) -> Result<(), GenieError> {
        let owner = &self.players[p];
        if owner
            .objects
            .iter()
            .any(|u| u.life.master_id == master && u.life.state == 2)
        {
            return Ok(());
        }
        let Some(t) = usize::try_from(tech)
            .ok()
            .filter(|&t| t < owner.techs.len())
        else {
            return Ok(());
        };
        if owner.techs[t].state <= 0 {
            return Ok(());
        }
        self.players[p].techs[t].state = 0;
        self.update_dependents(p, t)
    }

    /// `FUN_1413cce70` for player `p`'s technology `t`: each technology that requires it, by id
    /// (`+0x38`'s lists), not of the shared kind: one in state 0 or 1 and not held by
    /// `+0x60` turns to −1 when it is another civ's (or its civ's under the full tech tree,
    /// [`crate::settings::Settings::full_tech_tree`], while not of mode 1) or not for this game
    /// mode; else with fewer of
    /// its requirements researched than it needs it turns to 0 (from above 0), with them all to
    /// 1 (from below 1) unless its research is done already (its work done reaching a
    /// location's time), which completes it ([`Turns::complete_research`]). Each such technology, and each in
    /// state 2, counts; none counted sets attribute 39 to 1 where it is 0. The game's lists hold
    /// the dependents by id, as the `.dat`'s requirements give them.
    pub(crate) fn update_dependents(&mut self, p: usize, t: usize) -> Result<(), GenieError> {
        let mut counted = 0;
        let civ = i16::from(self.players[p].civ);
        for d in 0..self.techs.len() {
            let def = &self.techs[d];
            if !def.required.contains(&(t as i16)) || def.kind == crate::technology::SHARED_KIND {
                continue;
            }
            let Some(state) = self.players[p].techs.get(d) else {
                continue;
            };
            match state.state {
                0 | 1 => {}
                2 => {
                    counted += 1;
                    continue;
                }
                _ => continue,
            }
            if state.enabled {
                continue;
            }
            // another civ's: −1; the civ's own too under the full tech tree (the option +0x63)
            // unless of mode 1
            let disabled = if def.civ < 1 {
                false
            } else if def.civ == civ {
                self.settings.full_tech_tree && def.mode != 1
            } else {
                true
            };
            if disabled {
                self.players[p].techs[d].state = -1;
                continue;
            }
            // FUN_1413cf2f0: a technology for other games turns to −1 too
            if !self.settings.tech_mode_allows(def.mode)? {
                self.players[p].techs[d].state = -1;
                continue;
            }
            counted += 1;
            let techs = &self.players[p].techs;
            let met = def
                .required
                .iter()
                .filter(|&&r| 0 <= r && techs.get(r as usize).is_some_and(|s| s.state == 3))
                .count() as i16;
            let state = &mut self.players[p].techs[d];
            if met < def.required_count {
                if 0 < state.state {
                    state.state = 0;
                }
                continue;
            }
            let reached = |l: &crate::technology::Location| {
                let at = state.location(l.building).copied().unwrap_or(*l);
                f32::from(l.time as i16) + at.adjustment <= state.research_done
            };
            // its research done already: it completes (`FUN_1413cdcd0` with `param_3` 0 and no
            // building), once per location reached without a building of its own (each
            // uncounting it; past the first the technology is done and the call does nothing),
            // else once for its building's
            if state.building_master == -1 {
                let reached = state.locations.iter().filter(|l| reached(l)).count();
                if 0 < reached {
                    for _ in 0..reached {
                        let _ = self.complete_research(p, d, None)?;
                        counted -= 1;
                    }
                    continue;
                }
            } else if state.location(state.building_master).is_some_and(reached) {
                let _ = self.complete_research(p, d, None)?;
                continue;
            }
            let state = &mut self.players[p].techs[d];
            if state.state < 1 {
                state.state = 1;
            }
        }
        if counted == 0 {
            let owner = &mut self.players[p];
            if let Some(v) = owner
                .attributes
                .get_mut(ATTRIBUTE_39)
                .filter(|v| **v == 0.0)
            {
                *v = 1.0;
            }
        }
        Ok(())
    }

    /// The cost modifier (`'e'`, `FUN_1413d10f0`) of player `p`'s technology `tech`: each of its
    /// `.dat` cost entries that is paid and takes `resource` (any for −1) changes its adjustment
    /// (`+0x6`): mode 2 adds `(amount − 1) × the .dat's amount`, rounded half away from 1; mode
    /// 0 sets it so the cost is `amount`; any other adds `amount`. A named resource changes only
    /// its first entry. A technology out of the tree is let be.
    pub(super) fn tech_cost(&mut self, p: usize, tech: i16, resource: i16, amount: f32, mode: u8) {
        let Some(t) = usize::try_from(tech)
            .ok()
            .filter(|&t| t < self.players[p].techs.len() && t < self.techs.len())
        else {
            return;
        };
        for i in 0..3 {
            let cost = self.techs[t].costs[i];
            if cost.paid == 0 || (cost.attribute != resource && resource != -1) {
                continue;
            }
            let adj = &mut self.players[p].techs[t].cost_adj[i];
            *adj = match mode {
                2 => {
                    let half = if amount < 1.0 { -0.5 } else { 0.5 };
                    let delta = (amount - 1.0) * f32::from(cost.amount) + half;
                    adj.wrapping_add(crate::checkpoint::cvttss2si(delta) as i16)
                }
                // CVTTSS2SI's 32 bits, the low word added (0x1413d11a6, 0x1413d11be)
                0 => (crate::checkpoint::cvttss2si(amount) as i16).wrapping_sub(cost.amount),
                _ => adj.wrapping_add(crate::checkpoint::cvttss2si(amount) as i16),
            };
            if resource != -1 {
                return;
            }
        }
    }

    /// The disable command (`'f'`, `FUN_14163a500` with `param_3` 0) of player `p`'s technology
    /// `tech`: one of the shared kind (`FUN_1413d1020`) is a building's own, which a command
    /// without a building as its object lets be (with one, `FUN_1411c49a0` files it in the
    /// building's list `+0xd00`, which the slice does not hold); any other goes through
    /// `FUN_1413cdb00`: a technology of type 0x21 turns to 1 (it may be researched), any other
    /// but a researched one (state 3) to −1; a negative id does so to every technology.
    pub(super) fn disable_tech(
        &mut self,
        p: usize,
        tech: i32,
        object: Option<i32>,
    ) -> Result<(), GenieError> {
        let count = self.players[p].techs.len().min(self.techs.len());
        let held = usize::try_from(tech).ok().filter(|&t| t < count);
        if held.is_some_and(|t| self.techs[t].kind == crate::technology::SHARED_KIND) {
            let building = object.and_then(|id| self.object(id)).is_some_and(|(q, u)| {
                self.master_of(q, u.life.master_id)
                    .is_ok_and(|m| m.kind == master_type::BUILDING)
            });
            if building {
                return Err(GenieError::unsupported(
                    "a research of the shared kind (FUN_1411c49a0)",
                ));
            }
            return Ok(());
        }
        let disable = |defs: &[crate::technology::TechDef],
                       states: &mut [crate::technology::TechState],
                       t: usize| {
            if defs[t].kind == REENABLED_KIND {
                states[t].state = 1;
            } else if states[t].state != RESEARCHED {
                states[t].state = -1;
            }
        };
        // FUN_1413cdb00 takes the id's low 16 bits (MOVZX EDX, BX in FUN_14163a500)
        let short = tech as i16;
        let states = &mut self.players[p].techs;
        match usize::try_from(short).ok().filter(|&t| t < count) {
            Some(t) => disable(&self.techs, states, t),
            None if short < 0 => {
                for t in 0..count {
                    disable(&self.techs, states, t);
                }
            }
            None => {}
        }
        Ok(())
    }

    /// The enable command (`FUN_1416665b0`) of player `p` on its master `master` with `enable`
    /// (the command's low byte): a master the player does not have is let be (its id check
    /// against `+0x260` is taken to hold). Without `one` building (a third argument of 1) and
    /// its object, the master's slot 0x58 (`FUN_141126280`) sets `available` unless it is
    /// `disabled` (the byte, held as its test against 0). The sprites' loading on an enable
    /// (slot 0xa8) is not simulation. The enabling of master `0x26d` does nothing in game mode
    /// 11; not modelled: its enabling in another mode (it asks a settings flag) and a command for
    /// one building (`FUN_1411c69f0`, `FUN_1411c6a50`: the building's list, not held).
    pub(super) fn enable_master(
        &mut self,
        p: usize,
        master: i16,
        enable: u8,
        one: bool,
        object: Option<i32>,
    ) -> Result<(), GenieError> {
        if self.masters.get(p).and_then(|m| m.get(master)).is_none() {
            return Ok(());
        }
        if enable == 1 && master == MODE_TESTED_MASTER {
            // game mode 11 never enables it; any other asks a settings flag (their `+0x18`'s
            // `+0x7c`, bit 1, FUN_141324b60), which is not held
            if self.settings.game_mode == MODE_WITHOUT_0X26D {
                return Ok(());
            }
            return Err(GenieError::unsupported(
                "the enabling of master 0x26d (the game's mode)",
            ));
        }
        if let (true, Some(id)) = (one, object) {
            let building = self.object(id).is_some_and(|(q, u)| {
                self.master_of(q, u.life.master_id)
                    .is_ok_and(|m| m.kind == master_type::BUILDING)
            });
            if building {
                return Err(GenieError::unsupported(
                    "an enable command for one building",
                ));
            }
            return Ok(());
        }
        let m = self.masters[p].get_mut(master).expect("tested above");
        if !m.disabled {
            m.available = enable != 0;
        }
        Ok(())
    }
}

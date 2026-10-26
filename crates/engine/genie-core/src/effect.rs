//! A research's completion and the effects it gives: `FUN_1413cdcd0` (the technology done, its
//! effect, the technologies that depend on it, the player's statistics), the effects'
//! `TRIBE_Effects::do_effect` (`FUN_141462440`, then the base `FUN_140ec2ac0`) with every kind
//! for every party, and what they reach: the player's attribute modifier (slots 0x150, 0x158,
//! 0x160 -> slot 0x148, `FUN_1416121b0`: the hooks `FUN_141614f40` around `FUN_14165a000`, the
//! objects' slots 0x78, 0x80, 0x88 and the masters' 0x18, 0x20, 0x28 of
//! [`crate::masters`]), the resource commands (slots 0x1b0, 0x1b8, the player's attribute set
//! and add), the technology changes (`'e'`, `'f'`, `'g'`, kind 8), the enables, upgrades,
//! spawns and cost entries. The objects' parts an upgrade, a spawn and the objects' own masters
//! need are the objects' routines, which this file leaves to them.

use crate::GenieError;
use crate::masters::{AttributeWorld, Master, Masters, master_type};
use crate::rng::Stream;
use crate::tech::EffectCommand;
use crate::turn::{Turns, Unit};

/// The attribute counting the technologies researched (`0x15`) and the one summing their costs
/// (`0x63`), which a research done at a location with a button adds to.
const TECHS_RESEARCHED: i16 = 0x15;
const RESEARCH_SPENT: i16 = 0x63;
/// The attribute the dependents' update sets to 1 when none of them is left (`+0x138`).
const ATTRIBUTE_39: usize = 39;
/// The master whose enabling first asks the game's mode (`0x26d`, `FUN_1416665b0`), and the
/// mode that never enables it (11).
const MODE_TESTED_MASTER: i16 = 0x26d;
const MODE_WITHOUT_0X26D: i32 = 11;
/// The `TRIBE_Effects` command kinds that change technologies (`'e'`, `'f'`, `'g'`): a cost, the
/// disabling and a research time.
const TECH_COST: u8 = b'e';
const TECH_DISABLE: u8 = b'f';
const TECH_TIME: u8 = b'g';
/// The technology type that a disable command makes available instead (`FUN_1413cdb00`: 0x21).
const REENABLED_KIND: i16 = 0x21;
/// A researched technology's state, which a disable command keeps (`FUN_1413cdb00`).
const RESEARCHED: i16 = 3;

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
        use crate::tech::Attributes;
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
            let factors = crate::tech::factors(t as i16, &get)?;
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
    fn research_effect_known(&self, p: usize, t: usize) -> Result<(), GenieError> {
        let def = &self.techs[t];
        let state = &self.players[p].techs[t];
        let shared = |c: &EffectCommand| {
            // FUN_1413d1020 tests the whole id
            usize::try_from(crate::checkpoint::cvttss2si(c.d))
                .ok()
                .and_then(|d| self.techs.get(d))
                .is_some_and(|d| d.kind == crate::tech::SHARED_KIND)
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
            if !def.required.contains(&(t as i16)) || def.kind == crate::tech::SHARED_KIND {
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
            let reached = |l: &crate::tech::Location| {
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
    fn tech_cost(&mut self, p: usize, tech: i16, resource: i16, amount: f32, mode: u8) {
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
    fn disable_tech(&mut self, p: usize, tech: i32, object: Option<i32>) -> Result<(), GenieError> {
        let count = self.players[p].techs.len().min(self.techs.len());
        let held = usize::try_from(tech).ok().filter(|&t| t < count);
        if held.is_some_and(|t| self.techs[t].kind == crate::tech::SHARED_KIND) {
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
        let disable =
            |defs: &[crate::tech::TechDef], states: &mut [crate::tech::TechState], t: usize| {
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
    fn enable_master(
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

/// The `RGE_Effects` command kinds the base (`FUN_140ec2ac0`) switches on: the player's own (0 to
/// 8), the same for its mutual allies (10 to 18), enemies (20 to 28), neutrals (30 to 38) and Gaia
/// (40 to 48), and the objects' (200 to 206).
mod kind {
    pub const SET: u8 = 0;
    pub const RESOURCE: u8 = 1;
    pub const ENABLE: u8 = 2;
    pub const UPGRADE: u8 = 3;
    pub const ADD: u8 = 4;
    pub const MULTIPLY: u8 = 5;
    pub const RESOURCE_MULTIPLY: u8 = 6;
    pub const SPAWN: u8 = 7;
    pub const TECH: u8 = 8;
    /// A set, add and multiply on the objects with masters of their own of a master or class
    /// (200 to 202), on the effect's object (203 to 205), and its master's change (206).
    pub const OBJECTS_SET: u8 = 200;
    pub const OBJECT_SET: u8 = 203;
    pub const OBJECT_UPGRADE: u8 = 206;
}

/// The cost attributes a multiply by a negative amount turns into a cost entry's removal or
/// addition (`FUN_1415c1f70`): 103 to 106.
const COST_ATTRIBUTES: std::ops::RangeInclusive<i16> = 103..=106;
/// The resource whose change (kind 1) grows the food the player's farms hold after the effect
/// (`0x24`, `FUN_141615270`).
const FARM_FOOD: i16 = 0x24;

/// Which players a block of the base's kinds reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Party {
    /// The player itself (0 to 8).
    Own,
    /// Its mutual allies (10 to 18, both relations 0).
    Allies,
    /// Its enemies (20 to 28, both relations 3 or both 4).
    Enemies,
    /// Its neutrals (30 to 38, both relations 1).
    Neutrals,
    /// Gaia (40 to 48).
    Gaia,
}

impl Party {
    /// The party of a command kind's block (its tens: 0 the player, 1 its allies, 2 its enemies,
    /// 3 its neutrals, 4 Gaia).
    const fn of_block(block: u8) -> Self {
        match block {
            0 => Self::Own,
            1 => Self::Allies,
            2 => Self::Enemies,
            3 => Self::Neutrals,
            _ => Self::Gaia,
        }
    }
}

impl Turns {
    /// The players party `party` reaches from player `p`: for a relation, each player past Gaia
    /// (`FUN_14152fdd0`, from 1, the player itself when it counts itself so) whose and to whom
    /// the relations (`+0x260`, `FUN_141667f50`) are the party's, in order.
    fn party(&self, p: usize, party: Party) -> Vec<usize> {
        let relation = |a: usize, b: usize| self.players[a].relations.get(b).copied();
        let mutual = |q: usize, wanted: &[u8]| {
            wanted
                .iter()
                .any(|&r| relation(p, q) == Some(r) && relation(q, p) == Some(r))
        };
        let wanted: &[u8] = match party {
            Party::Own => return vec![p],
            Party::Gaia => return vec![0],
            Party::Allies => &[0],
            Party::Enemies => &[3, 4],
            Party::Neutrals => &[1],
        };
        (1..self.players.len())
            .filter(|&q| mutual(q, wanted))
            .collect()
    }

    /// `TRIBE_Effects::do_effect` (`FUN_141462440`) of an effect's commands `effect` (the
    /// effect's id names them in the game's table; its 0x318 makes the spawns count in the local
    /// player's statistics, which are not simulation) for player `p` with the effect's object `object` (none for a research's): first its
    /// technology commands in order, the cost modifier (`'e'`, [`Turns::tech_cost`]), the disable
    /// command (`'f'`, an id other than −1, [`Turns::disable_tech`]) and the research time
    /// modifier (`'g'`, [`Turns::research_time`]); then the base's ([`Turns::base_effect`]).
    pub(crate) fn do_effect(
        &mut self,
        p: usize,
        effect: &[EffectCommand],
        object: Option<i32>,
    ) -> Result<(), GenieError> {
        for c in effect {
            // the technology commands read the third argument's low byte as the mode
            let mode = (c.c & 0xff) as u8;
            match c.kind {
                TECH_COST => self.tech_cost(p, c.a, c.b, c.d, mode),
                TECH_DISABLE => {
                    let tech = crate::checkpoint::cvttss2si(c.d);
                    if tech != -1 {
                        self.disable_tech(p, tech, object)?;
                    }
                }
                TECH_TIME => self.research_time(p, c.a, c.d, mode, -1)?,
                _ => {}
            }
        }
        self.base_effect(p, effect, object)
    }

    /// `RGE_Effects::do_effect` (`FUN_140ec2ac0`): each command by its kind ([`kind`]), for the
    /// players of its block ([`Party`]); a kind it does not know (the technology commands among
    /// them) does nothing. The command's third argument's low byte is the attribute of a change
    /// (slot 0x148, [`Turns::modify_attribute`]). After the commands, the player's change of the
    /// farms' food (resource 0x24) grows the food its farms hold ([`Turns::farm_food`]), by the
    /// last such command's value before and amount.
    fn base_effect(
        &mut self,
        p: usize,
        effect: &[EffectCommand],
        object: Option<i32>,
    ) -> Result<(), GenieError> {
        let mut farms: Option<(f32, f32)> = None;
        for c in effect {
            if kind::OBJECTS_SET <= c.kind && c.kind <= kind::OBJECT_UPGRADE {
                Self::object_effect(c, object)?;
                continue;
            }
            if 4 < c.kind / 10 || 8 < c.kind % 10 {
                continue;
            }
            let party = Party::of_block(c.kind / 10);
            match c.kind % 10 {
                kind::SET | kind::ADD | kind::MULTIPLY => self.attribute_command(p, party, c)?,
                kind::RESOURCE | kind::RESOURCE_MULTIPLY => {
                    self.resource_command(p, party, c, &mut farms)?;
                }
                kind::ENABLE | kind::UPGRADE | kind::SPAWN | kind::TECH => {
                    self.master_command(p, party, c, object)?;
                }
                // 9, 19, 29, 39 are no kind
                _ => {}
            }
        }
        if let Some((before, amount)) = farms {
            self.farm_food(p, before, amount)?;
        }
        Ok(())
    }

    /// [`Turns::base_effect`]'s attribute changes (kinds 0, 4 and 5 of a block): set and add by
    /// slot 0x148 ([`Turns::modify_attribute`]) for each player of the block; multiply too, but
    /// a cost attribute by a negative amount (`COMISS` 0, d; `JBE`: not a NaN) changes the cost
    /// entries instead, except in Gaia's block (`FUN_141666ee0`).
    fn attribute_command(
        &mut self,
        p: usize,
        party: Party,
        c: &EffectCommand,
    ) -> Result<(), GenieError> {
        let attribute = (c.c & 0xff) as u8;
        let change = match c.kind % 10 {
            kind::SET => Change::Set,
            kind::ADD => Change::Add,
            _ => Change::Multiply,
        };
        let cost = change == Change::Multiply
            && COST_ATTRIBUTES.contains(&c.c)
            && 0.0 > c.d
            && party != Party::Gaia;
        for q in self.party(p, party) {
            if cost {
                let unit = if c.b < 0 { c.a } else { c.b };
                self.cost_entries(q, unit, c.c, crate::checkpoint::cvttss2si(c.d));
            } else {
                self.modify_attribute(q, change, c.a, c.b, c.d, attribute)?;
            }
        }
        Ok(())
    }

    /// [`Turns::base_effect`]'s resource changes: kind 1 by the amount the effect's player's
    /// attribute `c` scales ([`Turns::scaled_amount`]; the own block's farm food is noted in
    /// `farms`, its value before and the amount, else a resource below 0 is skipped), kind 6 a
    /// multiply of a resource 0 or above; each for the block's players.
    fn resource_command(
        &mut self,
        p: usize,
        party: Party,
        c: &EffectCommand,
        farms: &mut Option<(f32, f32)>,
    ) -> Result<(), GenieError> {
        if c.kind % 10 == kind::RESOURCE_MULTIPLY {
            if 0 <= c.a {
                for q in self.party(p, party) {
                    self.multiply_resource(q, c.a, c.d)?;
                }
            }
            return Ok(());
        }
        // the effect's player's attribute `c` scales the amount
        let amount = self.scaled_amount(p, c);
        if party == Party::Own && c.a == FARM_FOOD {
            *farms = Some((self.attribute_of(p, FARM_FOOD), amount));
        } else if c.a < 0 {
            return Ok(());
        }
        let mode = (c.b & 0xff) as u8;
        for q in self.party(p, party) {
            self.modify_resource(q, c.a, amount, mode)?;
        }
        Ok(())
    }

    /// [`Turns::base_effect`]'s master commands for the block's players: the enable (a master 0
    /// or above; the own block's with its one-object flag and the effect's object), the upgrade
    /// (the own block's with the effect's object), the spawn and the technology change.
    fn master_command(
        &mut self,
        p: usize,
        party: Party,
        c: &EffectCommand,
        object: Option<i32>,
    ) -> Result<(), GenieError> {
        let own = party == Party::Own;
        match c.kind % 10 {
            kind::ENABLE => {
                if 0 <= c.a {
                    let enable = (c.b & 0xff) as u8;
                    for q in self.party(p, party) {
                        let (one, object) = if own {
                            (c.c == 1, object)
                        } else {
                            (false, None)
                        };
                        self.enable_master(q, c.a, enable, one, object)?;
                    }
                }
            }
            kind::UPGRADE => {
                for q in self.party(p, party) {
                    let object = object.filter(|_| own);
                    self.upgrade_master(q, c.a, c.b, c.c, object)?;
                }
            }
            kind::SPAWN => {
                for q in self.party(p, party) {
                    self.spawn_at(q, c.a, c.b, c.c)?;
                }
            }
            _ => {
                for q in self.party(p, party) {
                    self.modify_tech(q, c.a, c.b, c.d, c.c)?;
                }
            }
        }
        Ok(())
    }

    /// A resource command's amount (`FUN_140ec2ac0`'s kinds 1, 11, ...): its value times the
    /// effect's player's attribute named by its third argument, when the player has it.
    fn scaled_amount(&self, p: usize, c: &EffectCommand) -> f32 {
        usize::try_from(c.c)
            .ok()
            .and_then(|i| self.attribute_value(p, i))
            .map_or(c.d, |scale| c.d * scale)
    }

    /// Player `p`'s attribute `i` (the resources are [`crate::turn::Player::resources`]), None
    /// past its count (`+0x90`, the saved attributes').
    fn attribute_value(&self, p: usize, i: usize) -> Option<f32> {
        let player = &self.players[p];
        let value = player.attributes.get(i).copied()?;
        Some(if i < 4 { player.resources[i] } else { value })
    }

    /// Player `p`'s attribute `i`, 0 past its count.
    fn attribute_of(&self, p: usize, i: i16) -> f32 {
        usize::try_from(i)
            .ok()
            .and_then(|i| self.attribute_value(p, i))
            .unwrap_or(0.0)
    }
}

/// How slot 0x148 changes an attribute (its second argument): set (0, slot 0x150), add (1, slot
/// 0x158) or multiply (2, slot 0x160); the masters' slots 0x18, 0x20, 0x28 and the objects' slots
/// 0x78, 0x80, 0x88 follow it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// The attribute takes the amount.
    Set,
    /// The amount adds to it.
    Add,
    /// It is multiplied by the amount.
    Multiply,
}

/// The resources the population attribute (110) moves an object's storage of (`FUN_1412163c0`):
/// headroom (4), and current population (11) and 19, which hold its magnitude.
const HEADROOM: i16 = 4;
const POPULATION: i16 = 11;
const RESOURCE_19: i16 = 19;
/// A storage flag of `'@'` (64): the object's storage stays with it (`FUN_141195a50`).
const KEPT_STORAGE: u8 = 64;
/// A hit point total a capped multiply holds an object's below (`FUN_1411962e0`: `0x46fffe00`).
const OBJECT_HP_CAP: f32 = 32767.0;
/// A multiply's rounding (`DAT_142e7f384`, 0.5).
const ROUND: f32 = crate::image::effect::ROUND;

impl Turns {
    /// What the masters' setters read of the world for player `p` ([`AttributeWorld`]): the
    /// sprite table (the `.dat`'s graphics), no handicap (`player+0x668`, not held: none in the
    /// games probed) and the player's `+0x648` unset (not held: 0 for every player probed).
    fn attribute_world(
        graphics: &std::collections::BTreeMap<i16, crate::sprite::Graphic>,
    ) -> impl Fn(i32) -> bool + '_ {
        move |id| i16::try_from(id).is_ok_and(|id| graphics.contains_key(&id))
    }

    /// The ids player `p`'s tracker files under class `class` (`FUN_14104f8f0`, key class + 900)
    /// and those pending under it (`FUN_14104f9e0`, key class + 90900), each in the tracker's
    /// order (a queued unit's place holds −1, which no object has).
    fn tracked_class(&self, p: usize, class: i16) -> (Vec<i32>, Vec<i32>) {
        let t = &self.players[p].tracker;
        let key = i32::from(class) + crate::tracker::CLASS_KEY;
        (
            t.list(key).to_vec(),
            t.list(key + crate::tracker::PENDING).to_vec(),
        )
    }

    /// The ids player `p`'s tracker files as done under key master `key` (`FUN_14104f560`'s list
    /// of one key, whatever the objects' masters), in the tracker's order.
    fn tracked_key(&self, p: usize, key: i16) -> Vec<i32> {
        self.players[p].tracker.list(i32::from(key)).to_vec()
    }

    /// The ids player `p`'s tracker files under master `master` (`FUN_14104f960`), of which
    /// `FUN_14165a860` keeps those of that very master; and for a building master those pending
    /// under it, or under the stack unit it heads (`FUN_14104fa50`, key + 90000), whatever their
    /// masters. Each in the tracker's order.
    fn tracked_master(&self, p: usize, master: &Master) -> (Vec<i32>, Vec<i32>) {
        let player = &self.players[p];
        let of_master = |id: &i32| {
            player
                .objects
                .iter()
                .chain(&player.sleeping)
                .any(|u| u.id == *id && u.life.master_id == master.id)
        };
        let done: Vec<i32> = player
            .tracker
            .list(i32::from(master.id))
            .iter()
            .copied()
            .filter(of_master)
            .collect();
        let mut pending = Vec::new();
        if master_type::BUILDING <= master.kind {
            let stack = master.building.as_ref().map_or(-1, |b| b.stack_unit);
            let key = self
                .masters
                .get(p)
                .and_then(|m| m.get(stack))
                .filter(|s| {
                    s.kind == master_type::BUILDING
                        && s.building
                            .as_ref()
                            .is_some_and(|b| b.head_unit == master.id)
                })
                .map_or(master.id, |s| s.id);
            pending = player
                .tracker
                .list(i32::from(key) + crate::tracker::PENDING)
                .to_vec();
        }
        (done, pending)
    }
}

impl Turns {
    /// `FUN_14165bae0` and the objects' slots 0x78, 0x80, 0x88 (by `change`) on object `id` of
    /// player `p`: nothing for an object the world lacks (or one with a master of its own,
    /// `FUN_141188bc0`, which the slice does not hold: none taken to have one); else its type's
    /// slot, which reads its master's values before the master changes. Returns whether it ran.
    pub(crate) fn change_object(
        &mut self,
        id: i32,
        change: Change,
        v: f32,
        attribute: u8,
    ) -> Result<bool, GenieError> {
        let Some((owner, u)) = self.object(id) else {
            return Ok(false);
        };
        let master = self.master_of(owner, u.life.master_id)?.clone();
        match master.kind {
            master_type::BUILDING => {
                self.change_building(owner, id, &master, change, v, attribute)?;
            }
            master_type::COMBAT => self.change_combat(owner, id, &master, change, v, attribute)?,
            _ => self.change_static(owner, id, &master, change, v, attribute)?,
        }
        Ok(true)
    }

    /// `FUN_141195a50`, `FUN_141196060`, `FUN_1411962e0`: the static object's slots. The hit
    /// points take the master's share (a set: `v × hp / master's`, an add: `(master's + v) × hp /
    /// master's`, truncated to a short; a multiply: `v × hp + 0.5`, truncated, held at 32767 when
    /// that is not below it under the player's `+0x648` ([`CAPPED`])); the line of sight lets the old circle go and takes
    /// the new one (an object on the map in state 2: slot 0x1d8, then 0x1d0 with the new value,
    /// truncated); a set of a resource storage (21, 26, 27) moves its player's attribute from the
    /// old amount to the new one (slot 0xf8 twice) unless the master keeps it (flag `'@'`).
    fn change_static(
        &mut self,
        p: usize,
        id: i32,
        master: &Master,
        change: Change,
        v: f32,
        attribute: u8,
    ) -> Result<(), GenieError> {
        use crate::masters::attribute as a;
        match attribute {
            a::HIT_POINTS => {
                let (_, u) = self.object_mut(id).expect("found by the caller");
                let full = f32::from(master.hit_points);
                let hp = u.life.hp;
                let short = |x: f32| f32::from(crate::checkpoint::cvttss2si(x) as i16);
                u.life.hp = match change {
                    Change::Set => short(v * hp / full),
                    Change::Add => short((full + v) * hp / full),
                    // COMISS 32767, x; JBE: a NaN holds the cap too
                    Change::Multiply
                        if CAPPED
                            && crate::fmath::at_most_or_unordered(
                                OBJECT_HP_CAP,
                                v * hp + ROUND,
                            ) =>
                    {
                        OBJECT_HP_CAP
                    }
                    Change::Multiply => short(v * hp + ROUND),
                };
            }
            a::LINE_OF_SIGHT => {
                let los = match change {
                    Change::Set => v,
                    Change::Add => v + master.line_of_sight,
                    Change::Multiply => v * master.line_of_sight,
                };
                self.sight_again(id, crate::checkpoint::cvttss2si(los));
            }
            a::STORAGE_1 | a::STORAGE_2 | a::STORAGE_3 if change == Change::Set => {
                let j = match attribute {
                    a::STORAGE_1 => 0,
                    a::STORAGE_2 => 1,
                    _ => 2,
                };
                if let Some(&(kind, amount, flag)) = master.storage.get(j)
                    && flag != KEPT_STORAGE
                    && 0 <= kind
                {
                    let owner = &mut self.players[p];
                    owner.add_attribute(kind, -amount)?;
                    owner.add_attribute(kind, v)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// The line of sight's change on object `id` (`FUN_141195a50`'s attribute 1): an object on
    /// the map (its tile `+0x2e8`, which one inside another has not) in state 2 lets its circle go
    /// (slot 0x1d8, its master's line of sight) and takes one of `los` (slot 0x1d0).
    fn sight_again(&mut self, id: i32, los: i32) {
        let Some((_, u)) = self.object(id) else {
            return;
        };
        if u.life.state != 2 || u.life.inside.is_some() {
            return;
        }
        let Some(placed) = self.spatial.get(id).copied() else {
            return;
        };
        if let Some(sight) = self.spatial.sight.as_mut() {
            sight.sight_off(id, placed.position, placed.radius, false, false);
            let _ = sight.sight_on_los(id, placed.position, placed.radius, false, los, false);
        }
    }

    /// `FUN_1412163c0`, `FUN_1412167d0`, `FUN_141216970`: a combat unit's slots. Its charge
    /// (`+0x9e0`) keeps its share of the maximum (none: a set takes the value, an add adds it);
    /// its volleys (`+0xa00`) take the change; a set of the population (110) moves its player's
    /// attributes by its master's storage (headroom: plus the old amount's magnitude, then the
    /// new; population and resource 19: less the old, then plus the new's magnitude). The hero
    /// glow's particles (40) are not simulation; the rest is the static object's.
    fn change_combat(
        &mut self,
        p: usize,
        id: i32,
        master: &Master,
        change: Change,
        v: f32,
        attribute: u8,
    ) -> Result<(), GenieError> {
        use crate::masters::attribute as a;
        match (attribute, change) {
            (a::HERO_STATUS, Change::Set | Change::Add) => Ok(()),
            (a::MAXIMUM_CHARGE, _) => {
                let (_, u) = self.object_mut(id).expect("found by the caller");
                u.charge = charge_after(u.charge, master.max_charge, change, v);
                Ok(())
            }
            (a::TOTAL_MISSILES, _) => {
                let (_, u) = self.object_mut(id).expect("found by the caller");
                u.firepower = match change {
                    Change::Set => v,
                    Change::Add => v + u.firepower,
                    Change::Multiply => v * u.firepower,
                };
                Ok(())
            }
            (a::POPULATION, Change::Set) => self.population_storage(p, master, v),
            _ => self.change_static(p, id, master, change, v, attribute),
        }
    }

    /// `FUN_1412163c0`'s population (110): for each of the master's three storage entries (`+0x60a`
    /// to `+0x60e`, their amounts `+0x610` on), one of the headroom (4) adds the old amount's
    /// magnitude then `v`; one of the population (11) or resource 19 its negation then `v`'s
    /// magnitude (slot 0xf8 each).
    fn population_storage(&mut self, p: usize, master: &Master, v: f32) -> Result<(), GenieError> {
        let owner = &mut self.players[p];
        for &(kind, amount, _) in master.storage.iter().take(3) {
            match kind {
                HEADROOM => {
                    owner.add_attribute(HEADROOM, amount.abs())?;
                    owner.add_attribute(HEADROOM, v)?;
                }
                POPULATION | RESOURCE_19 => {
                    owner.add_attribute(kind, -amount)?;
                    owner.add_attribute(kind, v.abs())?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// `FUN_1411d4e70`, `FUN_1411d4f30`, `FUN_1411d4fe0`: a building's slots. Its build points
    /// (`+0xc50`) keep their share of its master's build time (its first train location's,
    /// `t`): a set `v / t × points`, an add `(t + v) / t × points`, a multiply
    /// `points / t × (t × v + 0.5, truncated to a short)`, or `1 × that` for a time of at most 0;
    /// its icon override (17, `+0xe`) is the interface's; the rest is the combat unit's.
    fn change_building(
        &mut self,
        p: usize,
        id: i32,
        master: &Master,
        change: Change,
        v: f32,
        attribute: u8,
    ) -> Result<(), GenieError> {
        use crate::masters::attribute as a;
        match (attribute, change) {
            (a::BUILDING_ICON_OVERRIDE, Change::Set) => Ok(()),
            (a::TRAIN_TIME, _) => {
                let (_, u) = self.object_mut(id).expect("found by the caller");
                if let Some(c) = u.construction.as_mut() {
                    c.progress = build_points_after(c.progress, master.build_time, change, v);
                }
                Ok(())
            }
            _ => self.change_combat(p, id, master, change, v, attribute),
        }
    }
}

/// 1.0 (`DAT_14330eb8c`): a building whose master has no build time takes the new one as its
/// points.
const ONE: f32 = crate::image::effect::ONE;
/// The player's `+0x648`, which caps an add or multiply of hit points (not held: 0 for every
/// player probed).
const CAPPED: bool = false;

/// A combat unit's charge after its master's maximum charge (`max`, the old) changes
/// (`FUN_1412163c0`'s `;`): a set takes `v / max × charge` (`v` without a maximum), an add
/// `(v + max) / max × charge` (`v + charge` without one), a multiply `v × charge`.
fn charge_after(charge: f32, max: f32, change: Change, v: f32) -> f32 {
    match change {
        Change::Set if max != 0.0 => v / max * charge,
        Change::Set => v,
        Change::Add if max != 0.0 => (v + max) / max * charge,
        Change::Add => v + charge,
        Change::Multiply => v * charge,
    }
}

/// A building's build points after its master's build time (`time`, the old) changes
/// (`FUN_1411d4e70`'s `e`).
fn build_points_after(points: f32, time: i16, change: Change, v: f32) -> f32 {
    let t = f32::from(time);
    match change {
        Change::Set => v / t * points,
        Change::Add => (t + v) / t * points,
        Change::Multiply => {
            let new = f32::from(crate::checkpoint::cvttss2si(t * v + ROUND) as i16);
            if 0.0 < t { points / t * new } else { ONE * new }
        }
    }
}

/// `unit` lies within the player's master table (`0 <= unit < player+0x20`): the slice's
/// [`Masters`] hold no count past their last master, which stands for the table's end.
fn within_table(masters: &Masters, unit: i16) -> bool {
    0 <= unit && masters.iter().any(|m| unit <= m.id)
}

/// The sprite attributes whose change shows on the objects after it (`FUN_140fde290`: 70 to 77,
/// 82 to 91 and 131).
const fn shows_sprite(attribute: u8) -> bool {
    matches!(attribute, 70..=77 | 82..=91 | 131)
}

/// The object has an action in its list (`+0x880`'s first node; a building producing has its
/// Make): the slice's top actions.
fn has_actions(u: &Unit) -> bool {
    u.artifact.is_some()
        || u.build.is_some()
        || u.move_to.is_some()
        || u.fly.is_some()
        || u.hunt.is_some()
        || u.gather.is_some()
        || u.attack.is_some()
        || u.explore.is_some()
        || u.enter.is_some()
        || u.transport.is_some()
        || u.missile.is_some()
        || u.make.is_some()
        || u.production.as_ref().is_some_and(|q| q.action.is_some())
}

impl Turns {
    /// Slot 0x148 (`FUN_1416121b0`) of player `p`: the hooks before (`FUN_141614f40` with 0),
    /// the change of attribute `attribute` by `v` on master `unit` or on the masters and
    /// objects of class `class` ([`Turns::change_attribute`]), the hooks after (with 1). A master
    /// it reaches whose loader left the changed value unfilled ([`Master::unloaded_for`]) stops it
    /// first.
    pub(crate) fn modify_attribute(
        &mut self,
        p: usize,
        change: Change,
        unit: i16,
        class: i16,
        v: f32,
        attribute: u8,
    ) -> Result<(), GenieError> {
        if let Some(why) = self
            .reached_masters(p, unit, class)
            .iter()
            .find_map(|&id| self.masters[p].get(id)?.unloaded_for(attribute))
        {
            return Err(GenieError::unsupported(why));
        }
        self.effect_hooks(p, false, unit, class, attribute)?;
        self.change_attribute(p, change, unit, class, v, attribute)?;
        self.effect_hooks(p, true, unit, class, attribute)
    }

    /// `FUN_141614f40`: for each of player `p`'s live objects (its list `+0x30`), a combat unit's
    /// or building's volleys lose the shares of the combat units inside before the change
    /// (`FUN_141217ac0`) and take them again after (`FUN_14120b910`), as the change may move
    /// them. The objects' abilities (`+0x928`, `+0x930`, `FUN_14107da40` and `FUN_14107b830`) are
    /// not held: none taken. After a change of a sprite ([`shows_sprite`]) each object of the
    /// master or class shows it ([`Turns::show_changed_sprite`]).
    fn effect_hooks(
        &mut self,
        p: usize,
        after: bool,
        unit: i16,
        class: i16,
        attribute: u8,
    ) -> Result<(), GenieError> {
        let ids: Vec<i32> = self.players[p].objects.iter().map(|u| u.id).collect();
        for id in ids {
            let Some((_, u)) = self.object(id) else {
                continue;
            };
            let master = self.master_of(p, u.life.master_id)?;
            let (kind, master_id, master_class) = (master.kind, master.id, master.class);
            if matches!(kind, master_type::COMBAT | master_type::BUILDING) {
                self.garrison_volleys(p, id, after)?;
            }
            if after && shows_sprite(attribute) && (master_class == class || master_id == unit) {
                self.show_changed_sprite(p, id, attribute)?;
            }
        }
        Ok(())
    }

    /// `FUN_141217ac0` (`after` false) and `FUN_14120b910` (true) on object `id` of player `p`:
    /// with units inside, a reload above 0 and an attack rate above 0 (`COMISS`, `JBE`: not a
    /// NaN; [`crate::garrison::attack_rate`]), each combat unit inside takes its share
    /// ([`crate::garrison::share`]) off its volleys (`+0xa00`), or adds it.
    fn garrison_volleys(&mut self, p: usize, id: i32, after: bool) -> Result<(), GenieError> {
        use crate::fmath::at_most_or_unordered;
        let Some((_, u)) = self.object(id) else {
            return Ok(());
        };
        let masters = &self.masters[p];
        let master = self.master_of(p, u.life.master_id)?;
        if u.garrison.is_empty() || at_most_or_unordered(master.reload, 0.0) {
            return Ok(());
        }
        let rate = crate::garrison::attack_rate(master, masters, u.reload_powerup);
        if at_most_or_unordered(rate, 0.0) {
            return Ok(());
        }
        let mut shares = Vec::new();
        for &inside in &u.garrison {
            let Some((q, unit)) = self.object(inside) else {
                continue;
            };
            let m = self.master_of(q, unit.life.master_id)?;
            if m.kind == master_type::COMBAT {
                shares.push(crate::garrison::share(
                    m,
                    &self.masters[q],
                    unit.reload_powerup,
                    rate,
                )?);
            }
        }
        let (_, u) = self.object_mut(id).expect("found above");
        for s in shares {
            u.firepower = if after {
                s + u.firepower
            } else {
                u.firepower - s
            };
        }
        Ok(())
    }

    /// The sprite part of `FUN_141614f40` after a sprite's change on object `id` of player `p`:
    /// one below type 40 or without actions shows its master's standing sprite again when the
    /// change was to the one it shows (71 with its idle variant `+0x78c` 0, 72 with 1;
    /// `FUN_141189760`, slot 0x68); one with actions has each take its master's sprites
    /// (`FUN_140a97970` -> `FUN_140a78150`): an Attack's (`FUN_140a7f800`) its moving, attack and
    /// standing sprites from its master unless they are its task's, and from type 70 on its idle
    /// attack and second attack sprites; a `MoveTo` keeps the sprite it holds (it is not one of
    /// the master's new ones) and shows it again (slot 0x60, which the unit shows already); the
    /// other actions' slot 0x50 does nothing.
    fn show_changed_sprite(&mut self, p: usize, id: i32, attribute: u8) -> Result<(), GenieError> {
        use crate::masters::attribute as a;
        let Some((_, u)) = self.object(id) else {
            return Ok(());
        };
        let master = self.master_of(p, u.life.master_id)?.clone();
        let g = |id: i16| self.graphics.get(&id).copied();
        if master.kind < master_type::ACTION || !has_actions(u) {
            let sprite = match (attribute, u.idle_variant) {
                (a::STANDING_GRAPHIC, 0) => Some(master.standing_sprite),
                (a::STANDING_GRAPHIC_2, 1) => master.standing_sprite_2,
                _ => return Ok(()),
            };
            // slot 0x68 with no sprite (a second standing sprite the table lacks) shows none,
            // which the slice's sprites cannot hold; without the graphics the Sprite stream is not
            // modelled
            let (Some(graphic), false) = (sprite.and_then(g), self.graphics.is_empty()) else {
                return Ok(());
            };
            let place = self.place(id).expect("found above");
            let rng = self.streams.get(Stream::Sprite);
            let u = match place {
                crate::order::Place::Live(q, i) => &mut self.players[q].objects[i],
                crate::order::Place::Sleeping(q, i) => &mut self.players[q].sleeping[i],
            };
            if u.sprite.is_some() {
                crate::attack::set_sprite(&mut u.sprite, graphic, rng);
            } else {
                // a sprite the slice does not hold is taken to be another one
                let _ = crate::sprite::ActiveSprite::start(graphic, rng);
            }
            return Ok(());
        }
        let sprites = MasterSprites {
            moving: g(master.move_sprite),
            attack: g(master.attack_sprite),
            standing: g(master.standing_sprite),
            idle_attack: g(master.idle_attack_sprite),
            second: g(master.attack_sprite2),
            combat: master_type::COMBAT <= master.kind,
        };
        let (_, u) = self.object_mut(id).expect("found above");
        // a unit's own Attack's task (the work command's attack task) has no sprites
        if let Some(attack) = u.attack.as_mut() {
            follow_master(&mut attack.arms, None, &sprites);
        }
        if let Some(hunt) = u.hunt.as_mut() {
            let task = hunt.task.sprites;
            if let Some(attack) = hunt.attack.as_mut() {
                follow_master(&mut attack.arms, Some(task), &sprites);
            }
        }
        Ok(())
    }

    /// `FUN_14165a000` of player `p`: a master `unit` within the player's table changes alone
    /// ([`Turns::change_master_attribute`]; a null slot changes nothing); else with a class, the
    /// objects the tracker files under it ([`Turns::tracked_class`]: each that runs its slot,
    /// [`Turns::change_object`], a building also its annexes of the class, but those with
    /// annexes of their own), the pending ones, then each master of the class, by id.
    fn change_attribute(
        &mut self,
        p: usize,
        change: Change,
        unit: i16,
        class: i16,
        v: f32,
        attribute: u8,
    ) -> Result<(), GenieError> {
        let Some(masters) = self.masters.get(p) else {
            return Ok(());
        };
        if within_table(masters, unit) {
            return match masters.get(unit) {
                Some(_) => self.change_master_attribute(p, change, unit, v, attribute),
                None => Ok(()),
            };
        }
        if class < 0 {
            return Ok(());
        }
        let (done, pending) = self.tracked_class(p, class);
        for id in done {
            if !self.change_object(id, change, v, attribute)? {
                continue;
            }
            let annexes = match self.object(id) {
                Some((_, u))
                    if self.master_of(p, u.life.master_id)?.kind == master_type::BUILDING =>
                {
                    crate::turn::annexes_of(id, &self.players[p].objects)
                }
                _ => continue,
            };
            for annex in annexes {
                let Some((_, a)) = self.object(annex) else {
                    continue;
                };
                let am = self.master_of(p, a.life.master_id)?;
                let own_annexes = am.kind == master_type::BUILDING
                    && !crate::turn::annexes_of(annex, &self.players[p].objects).is_empty();
                if !own_annexes && am.class == class {
                    let _ = self.change_object(annex, change, v, attribute)?;
                }
            }
        }
        for id in pending {
            let _ = self.change_object(id, change, v, attribute)?;
        }
        for id in self.reached_masters(p, unit, class) {
            self.change_one_master(p, id, change, v, attribute);
        }
        Ok(())
    }

    /// The masters of player `p` slot 0x148 changes ([`Turns::change_attribute`]): master `unit`
    /// within the table (none for a null slot), else those of class `class`, by id.
    fn reached_masters(&self, p: usize, unit: i16, class: i16) -> Vec<i16> {
        let Some(masters) = self.masters.get(p) else {
            return Vec::new();
        };
        if within_table(masters, unit) {
            return masters.get(unit).map(|m| m.id).into_iter().collect();
        }
        if class < 0 {
            return Vec::new();
        }
        masters
            .iter()
            .filter(|m| m.class == class)
            .map(|m| m.id)
            .collect()
    }

    /// `FUN_14165a400` with no object (`param_6` −1) of player `p`'s master `unit`: its objects
    /// the tracker files under it ([`Turns::tracked_master`]) each run their slot, but an obstruction
    /// type's change to another type and a height's change (32) take the object's obstruction
    /// record off first (`FUN_14165b510` -> `FUN_141189670`), and a hero status (40) on one from
    /// type 70 on runs its slot and is kept too; for a building master its pending objects
    /// follow; then the master's slot; then the kept objects: a building of the new height and not
    /// desolid takes its record again (`FUN_1411c3f70`, `FUN_14118c540`), as every object after
    /// an obstruction type's change; a hero in a group leaves it and joins it again
    /// (`FUN_14102c160`, `FUN_1410083c0`), which the slice's groups do not model.
    fn change_master_attribute(
        &mut self,
        p: usize,
        change: Change,
        unit: i16,
        v: f32,
        attribute: u8,
    ) -> Result<(), GenieError> {
        use crate::masters::attribute as a;
        let master = self.master_of(p, unit)?.clone();
        let (done, pending) = self.tracked_master(p, &master);
        let mut kept = Vec::new();
        for id in done {
            let Some((_, u)) = self.object(id) else {
                continue;
            };
            let kind = master.kind;
            let unrecorded = match attribute {
                // a char compare of the new type with the object's master's
                a::OBSTRUCTION_TYPE => {
                    crate::checkpoint::cvttss2si(v) as u8 != master.obstruction_type
                }
                a::SIZE_Z => true,
                _ => false,
            };
            if unrecorded {
                self.take_record(id, u.life.inside.is_some(), &master);
                kept.push(id);
            } else if attribute == a::HERO_STATUS && master_type::COMBAT <= kind {
                let _ = self.change_object(id, change, v, attribute)?;
                kept.push(id);
            } else {
                let _ = self.change_object(id, change, v, attribute)?;
            }
        }
        if master_type::BUILDING <= master.kind {
            for id in pending {
                let _ = self.change_object(id, change, v, attribute)?;
            }
        }
        self.change_one_master(p, unit, change, v, attribute);
        let master = self.master_of(p, unit)?.clone();
        for id in kept {
            match attribute {
                a::SIZE_Z => {
                    let desolid = self
                        .object(id)
                        .and_then(|(_, u)| u.construction.as_ref())
                        .is_some_and(|c| c.desolid);
                    if master.kind == master_type::BUILDING && !desolid && 0.0 < master.height {
                        self.give_record(id, &master);
                    }
                }
                a::OBSTRUCTION_TYPE => self.give_record(id, &master),
                _ => {
                    if self.object(id).is_some_and(|(_, u)| u.group.id != -1) {
                        return Err(GenieError::unsupported(
                            "a hero status change of a unit in a group (removeUnit, FUN_14102c160)",
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// `FUN_141189670` on object `id` (`inside`: it is inside another, `+0x686`'s case; the slice
    /// takes the flag as that): a master with height above 0 (the record's `+0x658`, which the
    /// slice holds with the place's record, otherwise) has the obstruction manager drop the
    /// object's record (`FUN_141055b40`) and the mesh its shape (`+0x660`, `FUN_141053840`).
    fn take_record(&mut self, id: i32, inside: bool, master: &Master) {
        if inside {
            return;
        }
        let recorded = self
            .spatial
            .get(id)
            .is_some_and(|o| o.obstruction.is_some());
        if 0.0 < master.height || recorded {
            self.spatial.drop_record(id);
            self.pathing.removed(id);
        }
    }

    /// `FUN_14118c540` on object `id`: below state 7, not inside another (`+0x686`) and of a
    /// master with height above 0, it takes its record again (`FUN_141056460`: by the master's
    /// obstruction kind, not 0 or 12, and class, not 0) and its mesh shape.
    fn give_record(&mut self, id: i32, master: &Master) {
        let Some((_, u)) = self.object(id) else {
            return;
        };
        if 7 <= u.life.state
            || u.life.inside.is_some()
            || crate::fmath::at_most_or_unordered(master.height, 0.0)
        {
            return;
        }
        let record = crate::spatial::Obstruction {
            kind: master.obstruction_type,
            class: master.obstruction_class,
        };
        if record.recorded() {
            self.spatial.add_record(id, record);
            if let Some(placed) = self.spatial.get(id).copied() {
                self.pathing.place_shape(id, &placed);
            }
        }
    }

    /// Player `p`'s master `id` takes the change (its slot 0x18, 0x20 or 0x28), then the values
    /// the slice derives from it follow ([`Master::follow`], [`Turns::master_followers`]).
    fn change_one_master(&mut self, p: usize, id: i16, change: Change, v: f32, attribute: u8) {
        {
            let sprite = Self::attribute_world(&self.graphics);
            let world = AttributeWorld {
                sprite: &sprite,
                handicap: None,
                capped: CAPPED,
            };
            let terrain = self.spatial.terrain.as_ref();
            let Some(m) = self.masters.get_mut(p).and_then(|m| m.get_mut(id)) else {
                return;
            };
            match change {
                Change::Set => m.set_attribute(attribute, v, &world),
                Change::Add => m.add_attribute(attribute, v, &world),
                Change::Multiply => m.multiply_attribute(attribute, v, &world),
            }
            m.follow(attribute, &self.graphics, terrain);
        }
        self.master_followers(p, id, attribute);
    }

    /// What player `p`'s objects of master `id` hold of it and the slice keeps beside the master
    /// (the game reads the master): their movement's values ([`Master::follow_moving`]), their
    /// dying sprite and death spawns, their construction's hit points, build time and record,
    /// their sight's line of sight and their tile's height test.
    fn master_followers(&mut self, p: usize, id: i16, attribute: u8) {
        use crate::masters::attribute as a;
        let Some(master) = self.masters.get(p).and_then(|m| m.get(id)).cloned() else {
            return;
        };
        let terrain = self.spatial.terrain.as_ref();
        let player = &mut self.players[p];
        let mut ids = Vec::new();
        for u in player
            .objects
            .iter_mut()
            .chain(player.sleeping.iter_mut())
            .filter(|u| u.life.master_id == id)
        {
            ids.push(u.id);
            if let Some(m) = u.mover.as_mut() {
                master.follow_moving(&mut m.stats, attribute, &self.graphics, terrain);
            }
            match attribute {
                a::DYING_GRAPHIC => {
                    u.life.dying_sprite = (0 <= master.death_sprite).then_some(master.death_sprite);
                }
                a::DEAD_UNIT_ID => u.life.death_spawn = 0 <= master.death_spawns[0],
                a::BLOOD_UNIT => u.life.remains = master.death_spawns[1],
                _ => {}
            }
            if let Some(c) = u.construction.as_mut() {
                match attribute {
                    a::HIT_POINTS => c.hit_points = master.hit_points,
                    a::TRAIN_TIME
                    | a::TRAIN_LOCATION
                    | a::TRAIN_LOCATIONS_ENTRY_MOD
                    | a::TRAIN_LOCATIONS_TOTAL_NUM => c.build_time = master.build_time,
                    a::OBSTRUCTION_TYPE | a::BLOCKAGE_CLASS => {
                        c.record = crate::spatial::Obstruction {
                            kind: master.obstruction_type,
                            class: master.obstruction_class,
                        };
                    }
                    _ => {}
                }
            }
        }
        for id in ids {
            match attribute {
                a::LINE_OF_SIGHT => {
                    if let Some(s) = self.spatial.sight.as_mut().and_then(|v| v.seer_mut(id)) {
                        s.los = master.line_of_sight;
                    }
                }
                a::SIZE_Z => self.spatial.set_tall(id, master.tall),
                _ => {}
            }
        }
    }
}

/// A master's sprites an Attack's slot 0x50 (`FUN_140a7f800`) takes ([`follow_master`]), as
/// graphics: the walking, attack, standing, idle attack and second attack sprites, and whether
/// the master is of a combat type (70 and up).
#[derive(Clone, Copy, Debug)]
pub(crate) struct MasterSprites {
    pub(crate) moving: Option<crate::sprite::Graphic>,
    pub(crate) attack: Option<crate::sprite::Graphic>,
    pub(crate) standing: Option<crate::sprite::Graphic>,
    pub(crate) idle_attack: Option<crate::sprite::Graphic>,
    pub(crate) second: Option<crate::sprite::Graphic>,
    pub(crate) combat: bool,
}

/// An Attack's slot 0x50 (`FUN_140a7f800`): its walking, attack and standing sprites take the
/// master's (`sprites`) unless they are its task's (`task`, the moving, proceeding, working and
/// carrying sprites; none for a unit's own Attack), and from type 70 on its idle attack and
/// second attack sprites.
pub(crate) fn follow_master(
    arms: &mut crate::attack::Arms,
    task: Option<[i16; 4]>,
    sprites: &MasterSprites,
) {
    let of_task = |s: Option<crate::sprite::Graphic>, i: usize| {
        task.zip(s).is_some_and(|(t, s)| t[i] == s.id)
    };
    if arms.move_sprite.is_some() && !of_task(arms.move_sprite, 0) {
        arms.move_sprite = sprites.moving;
    }
    if arms.sprite.is_some() && !of_task(arms.sprite, 1) && !of_task(arms.sprite, 2) {
        arms.sprite = sprites.attack;
    }
    if arms.idle_sprite.is_some() && !of_task(arms.idle_sprite, 1) && !of_task(arms.idle_sprite, 2)
    {
        arms.idle_sprite = sprites.standing;
    }
    if sprites.combat {
        arms.idle_attack_sprite = sprites.idle_attack;
        arms.second_sprite = sprites.second;
    }
}

/// The attribute that turns the allied vision on (`FUN_141616820`'s and `FUN_14162d610`'s 0x32):
/// the player's `allied_LOS_Enable` (`+0x2cc`) takes 1, the world's masks are rebuilt (its slot
/// 0x78, [`Turns::rebuild_vision`]) and the player's map files every tile its masks reach as new
/// ([`Turns::refill_tiles`], `FUN_141285cb0`).
const ALLIED_VISION: i16 = 0x32;
/// The attribute that shares every player's sight (0xb7): the masks are rebuilt.
const SHARED_VISION: i16 = 0xb7;
/// The attribute of the town centres' flare reveal across the players (0xc4, Spies' "Treason"):
/// a player with it above 0 changes nothing; else each other player with it at most 0 takes the
/// amount too.
const SPREAD_ATTRIBUTE: i16 = 0xc4;
/// The attribute whose set above 0 calls the scripts' `EffectFunction<n>` (0x21, `FUN_14162d610`).
const EFFECT_FUNCTION: i16 = 0x21;
/// The attribute giving the exponent the farms' food grows by (31, `FUN_141615270`).
const FARM_EXPONENT: i16 = 31;
/// The masters whose objects' food a farm-food change grows (`DAT_142f925d0`'s keys: 50, 1187,
/// 1897, 1893), in that order.
const FARMS: [i16; 4] = [50, 1187, 1897, 1893];

impl Turns {
    /// Slot 0x1b0 (`FUN_141620c70`) of player `p` on resource `attribute` with `amount`: every
    /// live combat unit's and building's volleys lose the shares of the units inside, the
    /// attribute changes (`FUN_141661160`: `mode` 0 sets it, slot 0xf0
    /// [`Turns::set_player_attribute`]; any other adds to it, slot 0xf8 with no statistics,
    /// [`Turns::add_player_attribute`]; the local player's statistics are not simulation), and
    /// the volleys take the shares again. A change of resource 0x20 would not run in the screen's
    /// mode 4 (a playback's is not) or with the game's `+0x668` set (not held: taken unset).
    fn modify_resource(
        &mut self,
        p: usize,
        attribute: i16,
        amount: f32,
        mode: u8,
    ) -> Result<(), GenieError> {
        self.all_garrison_volleys(p, false)?;
        if mode == 0 {
            self.set_player_attribute(p, attribute, amount)?;
        } else {
            self.add_player_attribute(p, attribute, amount)?;
        }
        self.all_garrison_volleys(p, true)
    }

    /// Slot 0x1b8 (`FUN_141620e30`) of player `p` on resource `attribute` by `v`: as
    /// [`Turns::modify_resource`], the attribute set to its value times `v` (`FUN_1416614c0`).
    fn multiply_resource(&mut self, p: usize, attribute: i16, v: f32) -> Result<(), GenieError> {
        self.all_garrison_volleys(p, false)?;
        let value = self.attribute_of(p, attribute) * v;
        self.set_player_attribute(p, attribute, value)?;
        self.all_garrison_volleys(p, true)
    }

    /// `FUN_141217ac0` (`after` false) or `FUN_14120b910` on each of player `p`'s live combat
    /// units and buildings ([`Turns::garrison_volleys`]).
    fn all_garrison_volleys(&mut self, p: usize, after: bool) -> Result<(), GenieError> {
        let ids: Vec<i32> = self.players[p].objects.iter().map(|u| u.id).collect();
        for id in ids {
            let kind = self
                .object(id)
                .map(|(_, u)| u.life.master_id)
                .and_then(|m| self.masters.get(p).and_then(|t| t.get(m)))
                .map(|m| m.kind);
            if matches!(kind, Some(master_type::COMBAT | master_type::BUILDING)) {
                self.garrison_volleys(p, id, after)?;
            }
        }
        Ok(())
    }

    /// `FUN_1416674b0`, the attribute's store: within the player's attributes' count it takes `v`
    /// (the resources are [`crate::turn::Player::resources`]). A computer player's resource
    /// peaks (`+0x6dc`) and attribute 262's fields (`+0x628`..`+0x630`) are not held.
    fn store_attribute(&mut self, p: usize, attribute: i16, v: f32) {
        let player = &mut self.players[p];
        let Some(i) = usize::try_from(attribute)
            .ok()
            .filter(|&i| i < player.attributes.len())
        else {
            return;
        };
        if i < 4 {
            player.resources[i] = v;
        } else {
            player.attributes[i] = v;
        }
    }

    /// Slot 0xf0 (`FUN_14162d610`) of player `p`: attribute `attribute` takes `v`
    /// ([`Turns::store_attribute`]), with its side effects: 0x32 turns the allied vision on
    /// first ([`Turns::allied_vision_on`]); 0xc4 changes nothing for a player above 0 and spreads
    /// to the others at most 0 ([`SPREAD_ATTRIBUTE`]); 0xb7 rebuilds the masks after; 0x21 above 0
    /// calls the scripts' `EffectFunction<v>` (the XS runtime is not modelled).
    pub(crate) fn set_player_attribute(
        &mut self,
        p: usize,
        attribute: i16,
        v: f32,
    ) -> Result<(), GenieError> {
        match attribute {
            ALLIED_VISION => self.allied_vision_on(p)?,
            SPREAD_ATTRIBUTE => {
                if 0.0 < self.attribute_of(p, SPREAD_ATTRIBUTE) {
                    return Ok(());
                }
                // the message to the players is the interface's
                for q in 1..self.players.len() {
                    if q != p && self.attribute_of(q, SPREAD_ATTRIBUTE) <= 0.0 {
                        self.store_attribute(q, SPREAD_ATTRIBUTE, v);
                    }
                }
            }
            EFFECT_FUNCTION if 0.0 < v => {
                return Err(GenieError::unsupported(
                    "an effect's script function (attribute 0x21, EffectFunction in Effects.xs)",
                ));
            }
            _ => {}
        }
        self.store_attribute(p, attribute, v);
        if attribute == SHARED_VISION {
            // the world's slot 0x78; the message is the interface's
            self.rebuild_vision()?;
        }
        Ok(())
    }

    /// Slot 0xf8 (`FUN_141616820`) of player `p` with no statistics (`param_4` 0): `v` adds to
    /// attribute `attribute` ([`crate::turn::Player::add_attribute`]), with the side effects of
    /// 0x32 (the allied vision on first), 0xb7 (the masks rebuilt first, with the old value) and
    /// 0xc4 (nothing for a player above 0; the others at most 0 add it too).
    pub(crate) fn add_player_attribute(
        &mut self,
        p: usize,
        attribute: i16,
        v: f32,
    ) -> Result<(), GenieError> {
        match attribute {
            ALLIED_VISION => self.allied_vision_on(p)?,
            SHARED_VISION => self.rebuild_vision()?,
            SPREAD_ATTRIBUTE => {
                if 0.0 < self.attribute_of(p, SPREAD_ATTRIBUTE) {
                    return Ok(());
                }
                for q in 1..self.players.len() {
                    let theirs = self.attribute_of(q, SPREAD_ATTRIBUTE);
                    if q != p && theirs <= 0.0 {
                        self.store_attribute(q, SPREAD_ATTRIBUTE, v + theirs);
                    }
                }
            }
            _ => return self.players[p].add_attribute(attribute, v),
        }
        // FUN_14165d040's store of the sum
        let sum = v + self.attribute_of(p, attribute);
        self.store_attribute(p, attribute, sum);
        Ok(())
    }

    /// Attribute 0x32's side effect: player `p`'s allied vision (`+0x2cc`) on, the masks rebuilt
    /// (the world's slot 0x78, `FUN_1415e92b0`) and its tile list refilled
    /// ([`Turns::refill_tiles`], `FUN_141285cb0`).
    fn allied_vision_on(&mut self, p: usize) -> Result<(), GenieError> {
        self.players[p].allied_vision = true;
        self.rebuild_vision()?;
        self.refill_tiles(p);
        Ok(())
    }

    /// `FUN_141615270` after player `p`'s change of the farms' food (from `before`, by `amount`):
    /// with attribute 31 at least 0 (`COMISS`, `JC`: not a NaN), each object the tracker files
    /// under the farm masters ([`FARMS`], their done objects, in that order,
    /// [`Turns::tracked_key`]) from type 40 on with an action (its Make) has the food still to make (`+0x44`) grow: with `held` its food
    /// (`+0x310`) and `f = held + still to make`, it takes `(f / before)^exponent × amount + f −
    /// held` (`powf`, here libm's).
    fn farm_food(&mut self, p: usize, before: f32, amount: f32) -> Result<(), GenieError> {
        let exponent = self.attribute_of(p, FARM_EXPONENT);
        if crate::fmath::below_or_unordered(exponent, 0.0) {
            return Ok(());
        }
        let mut ids = Vec::new();
        for farm in FARMS {
            ids.extend(self.tracked_key(p, farm));
        }
        for id in ids {
            let Some((q, u)) = self.object(id) else {
                continue;
            };
            if self.master_of(q, u.life.master_id)?.kind < master_type::ACTION {
                continue;
            }
            let (_, u) = self.object_mut(id).expect("found above");
            let held = u.life.amount;
            let Some(make) = u.make.as_mut() else {
                continue;
            };
            let f = held + make.remaining;
            let grown = libm::powf(f / before, exponent);
            make.remaining = (grown * amount + f) - held;
        }
        Ok(())
    }
}

/// The modes of `FUN_1415c3f20`'s technology change (kind 8) that change a cost, with the
/// resource and the cost modifier's mode (`DAT_142ed71b0`'s map: 0 to 3 set food, wood, stone,
/// gold; 0x4000 to 0x4003 add to them; 0xd to 0x10 multiply them, 0x11 every one).
const fn tech_cost_mode(mode: i16) -> Option<(i16, u8)> {
    match mode {
        0..=3 => Some((mode, 0)),
        0x4000..=0x4003 => Some((mode - 0x4000, 1)),
        0xd..=0x10 => Some((mode - 0xd, 2)),
        0x11 => Some((-1, 2)),
        _ => None,
    }
}

/// The modes that change a research time (`DAT_142f8f340`'s map: −1 sets, −2 adds, −3
/// multiplies).
const fn tech_time_mode(mode: i16) -> Option<u8> {
    match mode {
        -1 => Some(0),
        -2 => Some(1),
        -3 => Some(2),
        _ => None,
    }
}

/// 1.0, 2.0 and 3.0 (`DAT_14330eb8c`, `DAT_142e8c150`, `DAT_142eb1c98`): a state change's values
/// (kind 8's mode 12): 0 disables, 1 makes it unavailable, 2 available and enabled, 3 researched.
const STATE_UNAVAILABLE: f32 = crate::image::effect::STATE_UNAVAILABLE;
const STATE_AVAILABLE: f32 = crate::image::effect::STATE_AVAILABLE;
const STATE_RESEARCHED: f32 = crate::image::effect::STATE_RESEARCHED;
/// −1.0 (`DAT_14330ec20`): a research time without a `.dat` location, and a set's "none".
const NO_TIME: f32 = crate::image::effect::NO_TIME;

impl Turns {
    /// `FUN_1415c3f20`'s technology change (kind 8, its `0x6f` case) for player `p`'s technology
    /// `tech` by `mode` and `v` at location `location` (below 1: every location; past the
    /// technology's locations a new one is added, a copy of the last with no building): a cost
    /// mode changes a cost ([`Turns::tech_cost`]); 4 sets the location's building to a master the
    /// player has, 5 its button; 9 the stacking, 10 the research cap; 12 the state; 18 the
    /// effect override (an already researched technology's new effect would run at once, from
    /// the world's effect table, not held); the research time modes ([`tech_time_mode`]) change
    /// the time ([`Turns::research_time`]) and update the dependents. The icon (6), name (7),
    /// description (8) and hotkey (11) overrides are the interface's. A technology past the
    /// player's tree changes nothing (the game also reads its one past the end, not modelled).
    fn modify_tech(
        &mut self,
        p: usize,
        tech: i16,
        mode: i16,
        v: f32,
        location: i16,
    ) -> Result<(), GenieError> {
        let Some(t) = usize::try_from(tech)
            .ok()
            .filter(|&t| t < self.players[p].techs.len())
        else {
            return Ok(());
        };
        let (mut at, mut building) = (None, -1);
        if 0 < location {
            let state = &mut self.players[p].techs[t];
            let i = usize::try_from(location).expect("above 0");
            if i < state.locations.len() {
                building = state.locations[i].building;
                at = Some(i);
            } else {
                // the game copies the last entry; a technology without locations would read before
                // an empty list's start (a crash no recording shows)
                let Some(&last) = state.locations.last() else {
                    return Ok(());
                };
                at = Some(state.locations.len());
                state.locations.push(crate::tech::Location {
                    building: -1,
                    ..last
                });
            }
        }
        if let Some((resource, cost_mode)) = tech_cost_mode(mode) {
            self.tech_cost(p, tech, resource, v, cost_mode);
            return Ok(());
        }
        if let Some(time_mode) = tech_time_mode(mode) {
            self.research_time(p, tech, v, time_mode, building)?;
            return self.update_dependents(p, t);
        }
        let short = crate::checkpoint::cvttss2si(v) as i16;
        match mode {
            4 => {
                let has = self
                    .masters
                    .get(p)
                    .is_some_and(|m| m.get(crate::checkpoint::cvttss2si(v) as i16).is_some());
                if has {
                    let state = &mut self.players[p].techs[t];
                    for (i, l) in state.locations.iter_mut().enumerate() {
                        if at.is_none_or(|a| a == i) {
                            l.building = short;
                        }
                    }
                }
            }
            5 => {
                let state = &mut self.players[p].techs[t];
                for (i, l) in state.locations.iter_mut().enumerate() {
                    if at.is_none_or(|a| a == i) {
                        l.button = crate::checkpoint::cvttss2si(v) as u8;
                    }
                }
            }
            9 => self.players[p].techs[t].stack = v != 0.0,
            10 => self.players[p].techs[t].research_cap = short,
            12 => {
                let state = &mut self.players[p].techs[t];
                if v == 0.0 {
                    state.state = -1;
                } else if v == STATE_UNAVAILABLE {
                    state.state = 0;
                } else if v == STATE_AVAILABLE {
                    state.state = 1;
                    state.enabled = true;
                } else if v == STATE_RESEARCHED {
                    let _ = self.force_research(p, t)?;
                }
            }
            18 => {
                let state = &mut self.players[p].techs[t];
                state.effect_override = short;
                if state.state == RESEARCHED {
                    return Err(GenieError::unsupported(
                        "a researched technology's new effect (the world's effect table)",
                    ));
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// `FUN_1413cdcd0` with `param_3` 1 (kind 8's state 3) for player `p`'s technology `t`: one in
    /// state 0 to 2 completes as [`Turns::complete_research`] (the civ's disabled technologies are
    /// not asked); any other gives its effect again and counts in attributes 0x15 and 0x63 (its
    /// listed costs) whatever its location, its state left.
    fn force_research(&mut self, p: usize, t: usize) -> Result<bool, GenieError> {
        if (0..=2).contains(&self.players[p].techs[t].state) {
            return self.complete_research(p, t, None);
        }
        let def = self.techs[t].clone();
        self.research_effect_known(p, t)?;
        self.do_effect(p, &def.effect, None)?;
        let owner = &self.players[p];
        let get = |k: i16| crate::tech::Attributes::get(owner, k);
        let factors = crate::tech::factors(t as i16, &get)?;
        let listed = owner.techs[t].listed_costs(&def, factors);
        let spent = listed.iter().map(|&(_, a)| i32::from(a)).sum::<i32>() as f32;
        let owner = &mut self.players[p];
        owner.add_attribute(TECHS_RESEARCHED, 1.0)?;
        owner.add_attribute(RESEARCH_SPENT, spent)?;
        Ok(true)
    }

    /// The research time modifier (`'g'`, `FUN_1413d1210`) of player `p`'s technology `tech` by
    /// `v` in `mode`: each location (`location` −1; else its location of that building,
    /// `FUN_1413c8930`) changes its adjustment (`+0xc`) against the `.dat`'s time there (−1
    /// without one): mode 2 adds `(v − 1) × time` rounded half away from 1 (in 16 bits), mode 0
    /// sets it so the time is `v` (none for −1, which leaves 0), any other adds `v` (in 16 bits).
    /// A technology in state 1 whose work done reaches its new time completes
    /// ([`Turns::complete_research`]), but for a set to −1. The game walks a copy of the
    /// locations.
    fn research_time(
        &mut self,
        p: usize,
        tech: i16,
        v: f32,
        mode: u8,
        location: i16,
    ) -> Result<(), GenieError> {
        let Some(t) = usize::try_from(tech)
            .ok()
            .filter(|&t| t < self.players[p].techs.len())
        else {
            return Ok(());
        };
        let buildings: Vec<i16> = if location == -1 {
            self.players[p].techs[t]
                .locations
                .iter()
                .map(|l| l.building)
                .collect()
        } else {
            vec![location]
        };
        for building in buildings {
            let time = self
                .techs
                .get(t)
                .and_then(|d| d.locations.iter().find(|&&(b, _)| b == building))
                .map_or(NO_TIME, |&(_, time)| f32::from(time));
            let state = &mut self.players[p].techs[t];
            let i = state
                .locations
                .iter()
                .position(|l| l.building == building)
                .unwrap_or(0);
            // a building comes from the locations (kind 8 adds one first), so only an empty list
            // finds none, which the game's walk of the copy never reaches
            let Some(entry) = state.locations.get_mut(i) else {
                continue;
            };
            let short = |x: f32| f32::from(crate::checkpoint::cvttss2si(x) as i16);
            let adjustment = match mode {
                2 => {
                    let half = if crate::fmath::below_or_unordered(v, ONE) {
                        -ROUND
                    } else {
                        ROUND
                    };
                    short((v - ONE) * time + half) + entry.adjustment
                }
                0 => {
                    let s = if v == NO_TIME {
                        0
                    } else {
                        crate::checkpoint::cvttss2si(v) as i16
                    };
                    f32::from(s) - time
                }
                _ => short(v) + entry.adjustment,
            };
            entry.adjustment = adjustment;
            let done = state.state == 1
                && adjustment + time <= state.research_done
                && (mode != 0 || v != NO_TIME);
            if done {
                let _ = self.complete_research(p, t, None)?;
            }
        }
        Ok(())
    }
}

impl Turns {
    /// Slot 0x1a0 (`FUN_14160e410`) of player `p`: master `from` upgrades to `to` by `mode`
    /// ([`Turns::upgrade_objects`]), with the volleys' hooks around it (the abilities, `+0x930`,
    /// are not held).
    fn upgrade_master(
        &mut self,
        p: usize,
        from: i16,
        to: i16,
        mode: i16,
        object: Option<i32>,
    ) -> Result<(), GenieError> {
        self.all_garrison_volleys(p, false)?;
        self.upgrade_objects(p, from, to, mode, object)?;
        self.all_garrison_volleys(p, true)
    }

    /// `FUN_141657d10`, the upgrade's own part: of two masters `from` and `to` of player `p`'s
    /// table, mode 2 with the effect's `object` gives that object a master of its own
    /// (`FUN_141186d10`, a copy of the one it reads) where slot 0xa8 lets it
    /// ([`crate::upgrade::upgradable`]), runs its slot 0x98 to `to` keeping its master
    /// ([`Turns::upgrade_object`]) and copies `to` into its own master (slot 8). Any other mode
    /// goes through the live objects in order: one reading `from` takes its slot 0x98 (its master
    /// kept in modes −1 and 3), or past its use a master of its own and its old death spawn's
    /// copy (`FUN_141201d50`); one reading `from`'s dead unit master (`+0x5e4`, when `to`'s
    /// differs by its copy id) takes a master of its own; in mode 3 one whose own master has
    /// `from`'s id takes slot 0x98 keeping it and copies `to` into it (a building's research
    /// buttons, `FUN_1411c78f0`, are the interface's). Then the sleeping objects reading `from`
    /// take slot 0x98. Upgrades to 0x67f and 0x6a1 move the tracker's list of `from` to `to`'s
    /// (`FUN_14104fd10`, [`crate::tracker::Tracker::move_all`]). In modes −1 and 3 two masters of
    /// one type end with `from` taking `to`'s values ([`Master::copy_from`]; the masters' slot
    /// 0xa8, their graphics' loading, is not simulation).
    pub(crate) fn upgrade_objects(
        &mut self,
        p: usize,
        from: i16,
        to: i16,
        mode: i16,
        object: Option<i32>,
    ) -> Result<(), GenieError> {
        use crate::order::Place;
        use crate::upgrade::upgradable;
        let Some(masters) = self.masters.get(p) else {
            return Ok(());
        };
        let (Some(old), Some(new)) = (masters.get(from), masters.get(to)) else {
            return Ok(());
        };
        // the dead unit master whose objects take masters of their own: `from`'s when `to`'s
        // differs by its copy id
        let dead = masters.get(old.death_spawns[0]).filter(|d| {
            masters
                .get(new.death_spawns[0])
                .is_some_and(|e| d.copy_id != e.copy_id)
        });
        let dead = dead.cloned();
        let keep = mode == -1 || mode == 3;
        let (old, new) = (old.clone(), new.clone());
        if mode == 2
            && let Some(id) = object
        {
            if let Some(place) = self.place(id) {
                let u = self.object_at(place);
                let reads = self.master_read(p, u)?;
                if upgradable(reads.kind, u.life.state) {
                    self.object_at_mut(place).own_master = Some(Box::new(reads.clone()));
                    self.upgrade_object(p, place, (&reads, &new), true)?;
                    if let Some(own) = self.object_at_mut(place).own_master.as_mut() {
                        own.copy_from(&new);
                    }
                }
            }
        } else {
            for i in 0..self.players[p].objects.len() {
                let place = Place::Live(p, i);
                let u = &self.players[p].objects[i];
                let table = u.own_master.is_none();
                if table && u.life.master_id == from {
                    if upgradable(old.kind, u.life.state) {
                        self.upgrade_object(p, place, (&old, &new), keep)?;
                    } else {
                        self.own_and_dead_master(p, i, &old);
                    }
                } else if table && dead.as_ref().is_some_and(|d| d.id == u.life.master_id) {
                    let d = dead.clone().expect("checked");
                    self.players[p].objects[i].own_master = Some(Box::new(d));
                } else if mode == 3
                    && let Some(own) = u.own_master.as_deref().filter(|m| m.id == from)
                    && upgradable(own.kind, u.life.state)
                {
                    let own = own.clone();
                    self.upgrade_object(p, place, (&own, &new), true)?;
                    if let Some(m) = self.players[p].objects[i].own_master.as_mut() {
                        m.copy_from(&new);
                    }
                }
            }
            for i in 0..self.players[p].sleeping.len() {
                let u = &self.players[p].sleeping[i];
                if u.own_master.is_none() && u.life.master_id == from {
                    self.upgrade_object(p, Place::Sleeping(p, i), (&old, &new), keep)?;
                }
            }
        }
        // the upgrades to 0x67f and 0x6a1 move the tracker's list of `from` (FUN_14104fd10)
        if matches!(to, 0x67f | 0x6a1) {
            self.track_move_all(p, from, to);
        }
        if keep
            && old.kind == new.kind
            && let Some(m) = self.masters[p].get_mut(from)
        {
            m.copy_from(&new);
        }
        Ok(())
    }

    /// The master object `u` of player `p` reads: its own, else its player's.
    fn master_read(&self, p: usize, u: &Unit) -> Result<Master, GenieError> {
        u.own_master.as_deref().map_or_else(
            || self.master_of(p, u.life.master_id).cloned(),
            |m| Ok(m.clone()),
        )
    }

    /// An object of master `old` past its use (slot 0xa8 false) in an upgrade: a master of its own
    /// (`FUN_141186d10`: a copy of `old` unless it has one) and its dead master
    /// (`FUN_141201d50`: unless it has one, a copy of `old`'s death spawn, `+0x5e4`, of type 10 to
    /// 40, 70 or 80, from its player's table; its slot 0xa8, the graphics, is not simulation).
    fn own_and_dead_master(&mut self, p: usize, i: usize, old: &Master) {
        let dead = self
            .masters
            .get(p)
            .and_then(|m| m.get(old.death_spawns[0]))
            .cloned();
        let u = &mut self.players[p].objects[i];
        if u.own_master.is_none() {
            u.own_master = Some(Box::new(old.clone()));
        }
        if u.dead_master.is_none() && 0 <= old.death_spawns[0] {
            u.dead_master = dead
                .filter(|d| crate::turn::DEAD_COPY_TYPES.contains(&d.kind))
                .map(Box::new);
        }
    }

    /// Slot 0x1a8 (`FUN_1416136f0`) of player `p`: `count` objects of master `master` (Gaia's when
    /// the player lacks it) at each of its built objects of building master `at` (state 2, in its
    /// list's order), up to attribute 234's count of buildings when above 0, in a playback (the
    /// screen's mode is not 4). Each object: a male villager (0x53) draws its sex from the Player
    /// stream ("SpawnObject": `r % 100` above 49 a female, 0x125); the owner's slot 0x1c0 makes
    /// it at the building's place with its flag ([`Turns::make_new_object`]); its slot 0x1a0 takes
    /// it into the building where the building has room for one more (slot 0x198,
    /// [`Turns::has_room`]); unless the player's attribute 283 is above 0 it comes out at once
    /// toward no point (`FUN_1411e7220`, [`Turns::let_out`]); a relic (0x11d) counts in the
    /// world's `+0x844` ([`Turns::spawned_relics`]). The local player's statistics of effect
    /// 0x318's spawns are not simulation.
    fn spawn_at(&mut self, p: usize, master: i16, at: i16, count: i16) -> Result<(), GenieError> {
        /// The male and female villagers' masters.
        const MALE_VILLAGER: i16 = 0x53;
        const FEMALE_VILLAGER: i16 = 0x125;
        /// The relic's master, which the world counts.
        const RELIC: i16 = 0x11d;
        let has = |q: usize, id: i16| self.masters.get(q).is_some_and(|m| m.get(id).is_some());
        let building = self
            .masters
            .get(p)
            .and_then(|m| m.get(at))
            .is_some_and(|m| m.kind == master_type::BUILDING);
        if !building || !(has(p, master) || has(0, master)) || count <= 0 {
            return Ok(());
        }
        let owner = if has(p, master) { p } else { 0 };
        let attribute =
            |q: &crate::turn::Player, i: usize| q.attributes.get(i).copied().unwrap_or(0.0);
        let limit = attribute(&self.players[p], 234) as i32;
        let buildings: Vec<i32> = self.players[p]
            .objects
            .iter()
            .filter(|u| u.life.master_id == at && u.life.state == 2)
            .map(|u| u.id)
            .collect();
        for (k, &b) in buildings.iter().enumerate() {
            if 0 < limit && limit <= k as i32 {
                return Ok(());
            }
            for _ in 0..count {
                let made = if master == MALE_VILLAGER {
                    let r = self.streams.get(Stream::Player).rand15() as i32;
                    if 49 < r % 100 {
                        FEMALE_VILLAGER
                    } else {
                        MALE_VILLAGER
                    }
                } else {
                    master
                };
                let Some((_, building)) = self.object(b) else {
                    break;
                };
                let position = building.position;
                let Some(new) = self.make_new_object(owner, made, position, true)? else {
                    continue;
                };
                let Some((bp, bu)) = self.object(b) else {
                    break;
                };
                if self.has_room(bp, bu, None)? {
                    let Some(crate::order::Place::Live(np, ni)) = self.place(new) else {
                        continue;
                    };
                    self.enter_forced(np, ni, b)?;
                }
                if attribute(&self.players[p], 283) <= 0.0 {
                    let Some(crate::order::Place::Live(bp, bi)) = self.place(b) else {
                        break;
                    };
                    let _ = self.let_out(bp, bi, new, [-1.0, -1.0], (None, false))?;
                }
                if master == RELIC {
                    self.spawned_relics += 1;
                }
            }
        }
        Ok(())
    }

    /// The objects' kinds (`FUN_140ec2ac0`'s 200 to 206): 200 to 202 change the
    /// objects with masters of their own of master `a` or class `b` (`FUN_141659dd0`; the slice
    /// holds no own masters: none); 203 to 205 the effect's object, which first takes a master of
    /// its own (`FUN_14165abe0`, `FUN_141186d10`, not held); 206 changes the effect's object's
    /// master (`FUN_14165ae40`). Without an object only 200 to 202 run.
    const fn object_effect(c: &EffectCommand, object: Option<i32>) -> Result<(), GenieError> {
        match (c.kind, object) {
            (kind::OBJECTS_SET..kind::OBJECT_SET, _) | (_, None) => Ok(()),
            (kind::OBJECT_UPGRADE, Some(_)) => Err(GenieError::unsupported(
                "an effect's master change of its object (kind 206, FUN_14165ae40)",
            )),
            (_, Some(_)) => Err(GenieError::unsupported(
                "an effect on its object's own master (kinds 203 to 205, FUN_141186d10)",
            )),
        }
    }

    /// `FUN_1415c1f70` (a multiply of a cost by a negative amount) of player `p`: the cost
    /// attribute (103 to 106) names the resource (food, wood, gold 3, stone 2); `how` −2 removes
    /// its first cost entry from master `unit` (`FUN_141612df0`), −1 adds one of it, amount −1
    /// and paid, in the first empty entry (`FUN_14160c450`); a `unit` of 900 and up names class
    /// `unit − 900`, every master of it by id. Only a master from type 70 on has costs. The game
    /// scans four entries, the fourth past the three (not modelled); the slice holds the entries
    /// of a resource only, so an added one goes after them.
    fn cost_entries(&mut self, p: usize, unit: i16, attribute: i16, how: i32) {
        let resource = match attribute {
            103 => 0,
            104 => 1,
            105 => 3,
            106 => 2,
            _ => return,
        };
        if how != -2 && how != -1 {
            return;
        }
        let Some(masters) = self.masters.get_mut(p) else {
            return;
        };
        let class = (900..1000).contains(&unit).then(|| unit - 900);
        for m in masters.iter_mut() {
            let named = class.map_or(m.id == unit, |c| m.class == c);
            if !named || m.kind < master_type::COMBAT {
                continue;
            }
            if how == -2 {
                if let Some(i) = m.cost.iter().position(|c| c.0 == resource) {
                    let _ = m.cost.remove(i);
                }
            } else if m.cost.len() < 3 {
                m.cost.push((resource, -1, true));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::masters::{Master, Masters};
    use crate::turn::{Player, Unit};

    /// A combat unit's charge after its master's maximum charge changes (`FUN_1412163c0`'s
    /// `;`): a set and an add keep the charge's share of the maximum, without a maximum a set
    /// takes the value and an add adds it, a multiply scales the charge.
    #[test]
    fn a_charge_keeps_its_share_of_a_changed_maximum() {
        assert_eq!(charge_after(10.0, 20.0, Change::Set, 40.0), 20.0);
        assert_eq!(charge_after(10.0, 0.0, Change::Set, 40.0), 40.0);
        assert_eq!(charge_after(10.0, 20.0, Change::Add, 20.0), 20.0);
        assert_eq!(charge_after(10.0, 0.0, Change::Add, 5.0), 15.0);
        assert_eq!(charge_after(10.0, 20.0, Change::Multiply, 1.5), 15.0);
    }

    /// An object has actions while it holds a top action or its production list holds one (a
    /// building's Make); an empty queue or none at all is no action.
    #[test]
    fn an_object_has_actions_while_it_holds_one() {
        let mut u = Unit::default();
        assert!(!has_actions(&u));
        u.production = Some(crate::production::Production::default());
        assert!(!has_actions(&u));
        u.production.as_mut().unwrap().action = Some(crate::production::Making::Research(
            crate::production::MakeTech {
                state: 6,
                paid: false,
                tech: 22,
                master: -1,
                progress: 0.0,
            },
        ));
        assert!(has_actions(&u));
    }

    /// An Attack's slot 0x50 (`FUN_140a7f800`): its sprites take the master's unless they are
    /// its task's (the walking sprite the task's moving one, the attack and standing sprites the
    /// task's proceeding or working one), a sprite it lacks stays lacking, and only a combat
    /// master's idle attack and second attack sprites replace its own.
    #[test]
    fn an_attack_takes_its_master_s_sprites_but_its_task_s() {
        let g = |id| crate::sprite::Graphic {
            id,
            frames: 10,
            facets: 1,
            speed_multiplier: 1.0,
            frame_duration: 0.1,
            replay_delay: 0.0,
            flags: 0,
        };
        let arms = crate::attack::Arms {
            move_sprite: Some(g(1)),
            sprite: Some(g(2)),
            second_sprite: Some(g(3)),
            idle_sprite: Some(g(4)),
            idle_attack_sprite: None,
            missile: -1,
            fire_frame: 0,
            interrupt_frame: -1,
        };
        let sprites = MasterSprites {
            moving: Some(g(11)),
            attack: Some(g(12)),
            standing: Some(g(14)),
            idle_attack: Some(g(15)),
            second: Some(g(13)),
            combat: false,
        };
        let mut a = arms;
        follow_master(&mut a, None, &sprites);
        assert_eq!(
            [
                a.move_sprite,
                a.sprite,
                a.idle_sprite,
                a.second_sprite,
                a.idle_attack_sprite
            ],
            [Some(g(11)), Some(g(12)), Some(g(14)), Some(g(3)), None]
        );
        // the task's moving (1) and working (4, the standing sprite's) sprites stay
        let mut a = arms;
        follow_master(&mut a, Some([1, 9, 4, 9]), &sprites);
        assert_eq!(
            [a.move_sprite, a.sprite, a.idle_sprite],
            [Some(g(1)), Some(g(12)), Some(g(4))]
        );
        // a sprite it lacks stays lacking; a combat master's last two replace its own
        let mut a = crate::attack::Arms {
            move_sprite: None,
            ..arms
        };
        follow_master(
            &mut a,
            None,
            &MasterSprites {
                combat: true,
                ..sprites
            },
        );
        assert_eq!(
            [a.move_sprite, a.second_sprite, a.idle_attack_sprite],
            [None, Some(g(13)), Some(g(15))]
        );
    }

    /// `FUN_1412163c0`'s population set (110) over the master's first three storage entries: the
    /// headroom (4) takes back the old amount's magnitude and adds the value, the population (11)
    /// and resource 19 take back the old amount and add the value's magnitude; other kinds and a
    /// fourth entry change nothing.
    #[test]
    fn a_population_set_moves_each_storage_entry_to_the_new_value() {
        let mut t = Turns::new(
            0,
            0,
            0,
            vec![0],
            vec![Player {
                attributes: vec![0.0; 40],
                ..Player::default()
            }],
        );
        let master = Master {
            id: 70,
            storage: vec![
                (HEADROOM, -5.0, 0),
                (POPULATION, 3.0, 0),
                (7, 1.0, 0),
                (RESOURCE_19, 2.0, 0),
            ],
            ..Master::default()
        };
        t.population_storage(0, &master, -8.0).unwrap();
        let at = |i: i16| t.players[0].attributes[i as usize];
        assert_eq!(
            [at(HEADROOM), at(POPULATION), at(7), at(RESOURCE_19)],
            [-3.0, 5.0, 0.0, 0.0]
        );
    }

    /// `FUN_14118c540`: an object below state 7, not inside another, of a master with a height
    /// takes its master's record again (kind and class not 0); one dying, inside or of a flat
    /// master keeps none.
    #[test]
    fn an_object_takes_its_record_again_unless_dying_inside_or_flat() {
        let unit = |id, state: u8, inside: Option<i32>| {
            let mut u = Unit {
                id,
                position: [5.5, 5.5, 0.0],
                ..Unit::default()
            };
            u.life.master_id = 70;
            u.life.state = state;
            u.life.inside = inside;
            u
        };
        let mut t = Turns::new(
            0,
            0,
            0,
            vec![0],
            vec![Player {
                objects: vec![unit(1, 2, None), unit(2, 7, None), unit(3, 2, Some(9))],
                ..Player::default()
            }],
        );
        t.spatial = crate::spatial::Spatial::new([10, 10]);
        for id in 1..=3 {
            t.spatial.insert(
                id,
                crate::spatial::Placed {
                    position: [5.5, 5.5, 0.0],
                    radius: [0.5, 0.5],
                    class: 0,
                    player: 0,
                    master: 70,
                    obstruction_type: 2,
                    buildable_on: 0,
                    tall: true,
                    speed: 0.0,
                    unknown: None,
                    obstruction: None,
                },
            );
        }
        let master = Master {
            id: 70,
            height: 2.0,
            obstruction_type: 2,
            obstruction_class: 2,
            ..Master::default()
        };
        for id in 1..=3 {
            t.give_record(id, &master);
        }
        let record = |t: &Turns, id| t.spatial.get(id).and_then(|p| p.obstruction);
        let wall = Some(crate::spatial::Obstruction { kind: 2, class: 2 });
        assert_eq!(
            [record(&t, 1), record(&t, 2), record(&t, 3)],
            [wall, None, None]
        );
        // a flat master gives none
        t.spatial.drop_record(1);
        t.give_record(
            1,
            &Master {
                height: 0.0,
                ..master
            },
        );
        assert_eq!(record(&t, 1), None);
    }

    /// `FUN_1416136f0` (slot 0x1a8) of villagers (0x53) at player 1's town centre: each draws
    /// its sex from the Player stream (above 49 a female, 0x125), the owner's slot 0x1c0 makes it
    /// at the centre's place, it goes inside (the centre has room) and, attribute 283 at 0, comes
    /// out at once toward no point (no gather point: it stays beside the centre). A master the
    /// player lacks, Gaia lacking it too, spawns nothing.
    #[test]
    fn an_effect_spawns_its_villagers_at_each_built_town_centre() {
        use crate::garrison::tests::{TC, VILLAGER, let_out_world};
        let mut t = let_out_world();
        let standing = crate::sprite::Graphic {
            id: 300,
            frames: 10,
            facets: 1,
            speed_multiplier: 1.0,
            frame_duration: 0.1,
            replay_delay: 0.0,
            flags: 1,
        };
        let _ = t.graphics.insert(300, standing);
        for ms in &mut t.masters {
            ms.get_mut(VILLAGER).unwrap().standing_sprite = 300;
            let female = Master {
                id: 0x125,
                ..ms.get(VILLAGER).unwrap().clone()
            };
            *ms = Masters::new(ms.iter().cloned().chain([female]));
            ms.get_mut(TC).unwrap().garrison = 15;
        }
        t.players[1].attributes = vec![0.0; 600];
        let before = t.players[1].objects.len();
        let mut stream = t.streams.get(Stream::Player).clone();
        t.spawn_at(1, VILLAGER, TC, 2).unwrap();
        let made: Vec<(i16, Option<i32>)> = t.players[1].objects[before..]
            .iter()
            .map(|u| (u.life.master_id, u.life.inside))
            .collect();
        let sex = |r: u32| if 49 < r as i32 % 100 { 0x125 } else { VILLAGER };
        let expected = [sex(stream.rand15()), sex(stream.rand15())];
        assert_eq!(made, vec![(expected[0], None), (expected[1], None)]);
        assert_eq!(t.object(100).unwrap().1.garrison, Vec::<i32>::new());
        // a master neither the player nor Gaia has
        let mut t = let_out_world();
        let before = t.players[1].objects.len();
        t.spawn_at(1, 999, TC, 2).unwrap();
        assert_eq!(t.players[1].objects.len(), before);
    }

    /// Loom's effect (22) on player 2 of MarvinBlue/426048731 in update 4588: villagers (class 4)
    /// of 25 hit points take 15 more, one melee and two pierce armor.
    #[test]
    fn loom_adds_hit_points_to_its_villagers_and_their_master_and_armor_to_the_master() {
        let villager = |id, hp| {
            let mut u = Unit {
                id,
                ..Unit::default()
            };
            u.life.master_id = 83;
            u.life.hp = hp;
            u
        };
        let player = Player {
            objects: vec![villager(9175, 25.0), villager(9177, 10.0)],
            ..Player::default()
        };
        let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
        t.masters = vec![Masters::new([Master {
            id: 83,
            kind: 70,
            class: 4,
            hit_points: 25,
            armors: vec![(4, 0), (3, 0)],
            ..Master::default()
        }])];
        let loom = [
            EffectCommand {
                kind: 4,
                a: -1,
                b: 4,
                c: 0,
                d: 15.0,
            },
            EffectCommand {
                kind: 4,
                a: -1,
                b: 4,
                c: 8,
                d: 1025.0,
            },
            EffectCommand {
                kind: 4,
                a: -1,
                b: 4,
                c: 8,
                d: 770.0,
            },
        ];
        // the load files the objects in the players' trackers
        t.load_trackers();
        t.do_effect(0, &loom, None).unwrap();
        let hp: Vec<f32> = t.players[0].objects.iter().map(|u| u.life.hp).collect();
        // (25 + 15) × 10 / 25 = 16
        assert_eq!(hp, [40.0, 16.0]);
        let m = t.masters[0].get(83).unwrap();
        assert_eq!((m.hit_points, m.armors.clone()), (40, vec![(4, 1), (3, 2)]));
    }

    /// A unit of `master` with `hp` hit points.
    /// A building's construction whose build points reached its build time: built, so the load
    /// completes it in its player's tracker ([`Turns::load_trackers`]).
    fn built() -> crate::build::Construction {
        crate::build::Construction {
            progress: 1.0,
            desolid: false,
            builders: 0,
            hit_points: 1,
            build_time: 1,
            record: crate::spatial::Obstruction { kind: 0, class: 0 },
            snow: false,
            sprite_override: false,
            started: false,
        }
    }

    fn unit_of(id: i32, master: i16, hp: f32) -> Unit {
        let mut u = Unit {
            id,
            ..Unit::default()
        };
        u.life.master_id = master;
        u.life.hp = hp;
        u.life.state = 2;
        u
    }

    fn combat(id: i16, class: i16, hit_points: i16) -> Master {
        Master {
            id,
            kind: 70,
            class,
            hit_points,
            ..Master::default()
        }
    }

    fn attribute(kind: u8, a: i16, b: i16, c: i16, d: f32) -> EffectCommand {
        EffectCommand { kind, a, b, c, d }
    }

    /// `FUN_14165a400` (a master within the table, kind 0 with its id): only that master's
    /// objects and the master change; a master id the table holds no master at (a null slot)
    /// changes nothing, the class besides.
    #[test]
    fn an_effect_on_one_master_changes_its_objects_and_it_alone() {
        let player = Player {
            objects: vec![unit_of(1, 4, 30.0), unit_of(2, 5, 30.0)],
            ..Player::default()
        };
        let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
        t.masters = vec![Masters::new([
            combat(4, 6, 60),
            combat(5, 6, 60),
            combat(7, 6, 60),
        ])];
        // set master 4's hit points to 90: its unit keeps its share, 30 × 90 / 60 = 45
        // the load files the objects in the players' trackers
        t.load_trackers();
        t.do_effect(0, &[attribute(0, 4, 6, 0, 90.0)], None)
            .unwrap();
        let hp: Vec<f32> = t.players[0].objects.iter().map(|u| u.life.hp).collect();
        assert_eq!(hp, [45.0, 30.0]);
        let hps: Vec<i16> = t.masters[0].iter().map(|m| m.hit_points).collect();
        assert_eq!(hps, [90, 60, 60]);
        // master 6 lies within the table (up to 7) but has no master: nothing, not the class
        t.do_effect(0, &[attribute(4, 6, 6, 0, 10.0)], None)
            .unwrap();
        let hps: Vec<i16> = t.masters[0].iter().map(|m| m.hit_points).collect();
        assert_eq!(hps, [90, 60, 60]);
        // past the table the class changes: (60 + 10) × 30 / 60 = 35
        t.do_effect(0, &[attribute(4, 100, 6, 0, 10.0)], None)
            .unwrap();
        let hps: Vec<i16> = t.masters[0].iter().map(|m| m.hit_points).collect();
        assert_eq!(hps, [100, 70, 70]);
        assert_eq!(t.players[0].objects[1].life.hp, 35.0);
    }

    /// `FUN_141614f40`'s hooks: a building's volleys lose the shares of the combat units inside
    /// before the change and take the new shares after (`FUN_141217ac0`, `FUN_14120b910`); the
    /// change of the archers' attack moves an archer's share.
    #[test]
    fn a_change_moves_the_shares_of_the_units_inside_a_building() {
        let mut tower = unit_of(10, 79, 1000.0);
        tower.garrison = vec![11];
        tower.firepower = 2.0;
        let mut archer = unit_of(11, 4, 30.0);
        archer.life.inside = Some(10);
        let player = Player {
            objects: vec![tower, archer],
            ..Player::default()
        };
        let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
        let mut tower_master = combat(79, 52, 1020);
        tower_master.kind = 80;
        tower_master.reload = 2.0;
        tower_master.weapon_range = 8.0;
        tower_master.weapons = vec![(3, 5)];
        let mut archer_master = combat(4, 0, 30);
        archer_master.reload = 2.0;
        archer_master.weapons = vec![(3, 4)];
        archer_master.garrison_firepower = 1.0;
        t.masters = vec![Masters::new([archer_master, tower_master])];
        // the tower's rate: 5 / 2 = 2.5; the archer's share 4 / 2 / 2.5 = 0.8, then 6 / 2 / 2.5
        t.do_effect(0, &[attribute(4, -1, 0, 9, f32::from(0x302_i16))], None)
            .unwrap();
        assert_eq!(t.masters[0].get(4).unwrap().weapons, [(3, 6)]);
        assert_eq!(t.players[0].objects[0].firepower, 2.0 - 0.8 + 1.2);
    }

    /// The class path (`FUN_14165a000`): a building of the class takes the change, then its
    /// annexes of the class, which the tracker files under no class; a foundation of the class
    /// (pending) takes it too.
    #[test]
    fn a_class_effect_reaches_buildings_their_annexes_and_foundations() {
        let mut main = unit_of(20, 30, 100.0);
        main.life.master_id = 30;
        main.construction = Some(built());
        let mut annex = unit_of(21, 31, 50.0);
        annex.annex_of = Some(20);
        annex.construction = Some(built());
        let mut foundation = unit_of(22, 30, 10.0);
        foundation.life.state = 0;
        let player = Player {
            objects: vec![main, annex, foundation],
            ..Player::default()
        };
        let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
        let building = |id, hit_points, head_unit, annexes| Master {
            id,
            kind: 80,
            class: 3,
            hit_points,
            building: Some(crate::masters::BuildingMaster {
                stack_unit: -1,
                head_unit,
                annexes,
                ..crate::masters::BuildingMaster::default()
            }),
            ..Master::default()
        };
        t.masters = vec![Masters::new([
            building(30, 200, -1, [31, -1, -1, -1]),
            building(31, 100, 30, [-1; 4]),
        ])];
        // multiply the class's hit points by 1.5: every object's hit points × 1.5 + 0.5
        // the load files the objects in the players' trackers
        t.load_trackers();
        t.do_effect(0, &[attribute(5, -1, 3, 0, 1.5)], None)
            .unwrap();
        let hp: Vec<f32> = t.players[0].objects.iter().map(|u| u.life.hp).collect();
        assert_eq!(hp, [150.0, 75.0, 15.0]);
        let hps: Vec<i16> = t.masters[0].iter().map(|m| m.hit_points).collect();
        assert_eq!(hps, [300, 150]);
    }

    /// The allies' block (14: add) reaches the player's mutual allies, itself among them, and
    /// Gaia's (44) Gaia alone.
    #[test]
    fn a_team_effect_reaches_the_mutual_allies() {
        let player = |relations: Vec<u8>| Player {
            relations,
            ..Player::default()
        };
        let mut t = Turns::new(
            0,
            0,
            0,
            vec![0; 4],
            vec![
                player(vec![0, 3, 3, 3]),
                player(vec![3, 0, 0, 3]),
                player(vec![3, 0, 0, 3]),
                player(vec![3, 3, 3, 0]),
            ],
        );
        t.masters = vec![Masters::new([combat(4, 6, 60)]); 4];
        t.do_effect(1, &[attribute(14, -1, 6, 0, 5.0)], None)
            .unwrap();
        let hps: Vec<i16> = t
            .masters
            .iter()
            .map(|m| m.get(4).unwrap().hit_points)
            .collect();
        assert_eq!(hps, [60, 65, 65, 60]);
        t.do_effect(1, &[attribute(44, -1, 6, 0, 5.0)], None)
            .unwrap();
        let hps: Vec<i16> = t
            .masters
            .iter()
            .map(|m| m.get(4).unwrap().hit_points)
            .collect();
        assert_eq!(hps, [65, 65, 65, 60]);
        // the enemies' block (24): player 3 counts player 1 an enemy and is counted one
        t.do_effect(1, &[attribute(24, -1, 6, 0, 5.0)], None)
            .unwrap();
        let hps: Vec<i16> = t
            .masters
            .iter()
            .map(|m| m.get(4).unwrap().hit_points)
            .collect();
        assert_eq!(hps, [65, 65, 65, 65]);
    }

    /// A set of a resource storage (`FUN_141195a50`'s 21): each object's player loses the old
    /// amount and gains the new (slot 0xf8 twice), unless the master keeps it (`'@'`).
    #[test]
    fn a_storage_set_moves_the_player_s_attribute_by_each_object() {
        let player = Player {
            objects: vec![unit_of(1, 70, 550.0), unit_of(2, 70, 550.0)],
            attributes: vec![0.0; 40],
            ..Player::default()
        };
        let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
        let mut house = combat(70, 3, 550);
        house.storage = vec![(4, 5.0, 4), (-1, 0.0, 0), (-1, 0.0, 0)];
        t.masters = vec![Masters::new([house])];
        t.players[0].attributes[4] = 10.0;
        // the load files the objects in the players' trackers
        t.load_trackers();
        t.do_effect(0, &[attribute(0, 70, -1, 21, 15.0)], None)
            .unwrap();
        assert_eq!(t.players[0].attributes[4], 30.0);
        assert_eq!(t.masters[0].get(70).unwrap().storage[0].1, 15.0);
    }

    /// `FUN_141195a50`'s line of sight on a static object (kind 10) on the map: its old circle
    /// (the seer's own, 2) goes and the new one is explored (a set: the value; an add: the
    /// master's plus it; a multiply: the master's times it, truncated); off the map (state 0)
    /// nothing. A set of the second storage (26) moves its attribute as the first does.
    #[test]
    fn a_static_object_s_line_of_sight_and_second_storage_take_the_change() {
        let (w, h) = (20, 20);
        let n = (w * h) as usize;
        let sight = |t: &Turns| {
            let map = &t.spatial.sight.as_ref().unwrap().maps[0];
            map.counts.iter().filter(|&&c| c > 0).count()
        };
        let world = || {
            let player = Player {
                objects: vec![unit_of(1, 50, 10.0)],
                attributes: vec![0.0; 40],
                ..Player::default()
            };
            let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
            let stone = Master {
                id: 50,
                kind: 10,
                class: 11,
                hit_points: 10,
                line_of_sight: 2.0,
                storage: vec![(-1, 0.0, 0), (4, 5.0, 0), (-1, 0.0, 0)],
                ..Master::default()
            };
            t.masters = vec![Masters::new([stone])];
            t.load_trackers();
            let mut v = crate::visibility::Visibility::new(
                w,
                h,
                vec![crate::visibility::VisibleMap {
                    counts: vec![0; n],
                    explored: 0,
                }],
                vec![0; n],
                Vec::new(),
                0,
                vec![false],
            );
            let seer = crate::visibility::Seer {
                player: 0,
                master: 50,
                los: 2.0,
                state: 2,
                explored: true,
                building: None,
                umv: 0,
                capture: 0,
                kind: 10,
            };
            v.add_seer(1, seer);
            let position = [10.5, 10.5, 0.0];
            let radius = [0.5, 0.5];
            let _ = v.sight_on(1, position, radius, false, false);
            t.spatial.sight = Some(v);
            t.spatial.insert(
                1,
                crate::spatial::Placed {
                    position,
                    radius,
                    class: 11,
                    player: 0,
                    master: 50,
                    obstruction_type: 0,
                    buildable_on: 0,
                    tall: false,
                    speed: 0.0,
                    unknown: None,
                    obstruction: None,
                },
            );
            t
        };
        // the tiles a circle of `los` explores, on a map of its own
        let circle = |los| {
            let mut t = world();
            let v = t.spatial.sight.as_mut().unwrap();
            v.sight_off(1, [10.5, 10.5, 0.0], [0.5, 0.5], false, false);
            let _ = v.sight_on_los(1, [10.5, 10.5, 0.0], [0.5, 0.5], false, los, false);
            sight(&t)
        };
        let (two, four, five, six) = (circle(2), circle(4), circle(5), circle(6));
        assert!(two < four && four < five && five < six);
        let mut t = world();
        assert_eq!(sight(&t), two);
        // a set to 4.9: truncated to 4
        t.do_effect(0, &[attribute(0, -1, 11, 1, 4.9)], None)
            .unwrap();
        assert_eq!(sight(&t), four);
        // an add of 3: the master's 2 (the clone before its own change) plus 3
        let mut t = world();
        t.do_effect(0, &[attribute(4, -1, 11, 1, 3.0)], None)
            .unwrap();
        assert_eq!(sight(&t), five);
        // a multiply by 3: the master's 2 times 3
        let mut t = world();
        t.do_effect(0, &[attribute(5, -1, 11, 1, 3.0)], None)
            .unwrap();
        assert_eq!(sight(&t), six);
        // off the map: the circle stays
        let mut t = world();
        t.players[0].objects[0].life.state = 0;
        t.do_effect(0, &[attribute(0, -1, 11, 1, 6.0)], None)
            .unwrap();
        assert_eq!(sight(&t), two);
        // the second storage: its attribute 4 loses the old 5 and takes the new 8
        let mut t = world();
        t.players[0].attributes[4] = 10.0;
        t.do_effect(0, &[attribute(0, -1, 11, 26, 8.0)], None)
            .unwrap();
        assert_eq!(t.players[0].attributes[4], 13.0);
    }

    /// What the slice keeps beside a master follows its change: its objects' (live and asleep)
    /// dying sprite (none below 0), death spawn (a first spawn of 0 or more) and remains (the
    /// second), a foundation's build time and record, a placed object's height test and a
    /// seer's line of sight; another master's objects and a master the player lacks keep theirs.
    #[test]
    fn an_object_s_copies_of_its_master_follow_its_change() {
        use crate::masters::attribute as a;
        let mut site = unit_of(1, 4, 30.0);
        site.construction = Some(built());
        let player = Player {
            objects: vec![site, unit_of(3, 5, 30.0)],
            sleeping: vec![unit_of(2, 4, 30.0)],
            ..Player::default()
        };
        let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
        let mut m = combat(4, 6, 60);
        (m.death_sprite, m.death_spawns) = (7, [0, 9]);
        (m.build_time, m.obstruction_type, m.obstruction_class) = (33, 5, 2);
        (m.line_of_sight, m.tall) = (6.0, true);
        t.masters = vec![Masters::new([m, combat(5, 6, 60)])];
        for id in [1, 3] {
            t.spatial.insert(
                id,
                crate::spatial::Placed {
                    position: [5.5, 5.5, 0.0],
                    radius: [0.5, 0.5],
                    class: 6,
                    player: 0,
                    master: 4,
                    obstruction_type: 0,
                    buildable_on: 0,
                    tall: false,
                    speed: 0.0,
                    unknown: None,
                    obstruction: None,
                },
            );
        }
        let mut v = crate::visibility::Visibility::new(
            10,
            10,
            Vec::new(),
            vec![0; 100],
            Vec::new(),
            0,
            Vec::new(),
        );
        let seer = crate::visibility::Seer {
            player: 0,
            master: 4,
            los: 2.0,
            state: 2,
            explored: true,
            building: None,
            umv: 0,
            capture: 0,
            kind: 70,
        };
        v.add_seer(1, seer);
        v.add_seer(3, seer);
        t.spatial.sight = Some(v);
        let all = [
            a::DYING_GRAPHIC,
            a::DEAD_UNIT_ID,
            a::BLOOD_UNIT,
            a::TRAIN_TIME,
            a::OBSTRUCTION_TYPE,
            a::SIZE_Z,
            a::LINE_OF_SIGHT,
        ];
        for attribute in all {
            t.master_followers(0, 4, attribute);
            t.master_followers(0, 99, attribute);
        }
        let follows = |u: &Unit| (u.life.dying_sprite, u.life.death_spawn, u.life.remains);
        let p = &t.players[0];
        assert_eq!(follows(&p.objects[0]), (Some(7), true, 9));
        assert_eq!(follows(&p.sleeping[0]), (Some(7), true, 9));
        assert_eq!(follows(&p.objects[1]), (None, false, -1));
        let c = p.objects[0].construction.unwrap();
        assert_eq!(c.build_time, 33);
        assert_eq!(c.record, crate::spatial::Obstruction { kind: 5, class: 2 });
        let tall = |id| t.spatial.get(id).unwrap().tall;
        assert_eq!((tall(1), tall(3)), (true, false));
        let los = |id| t.spatial.sight.as_ref().unwrap().seer(id).unwrap().los;
        assert_eq!((los(1), los(3)), (6.0, 2.0));
        // a dying sprite below 0 is none
        t.masters[0].get_mut(4).unwrap().death_sprite = -1;
        t.master_followers(0, 4, a::DYING_GRAPHIC);
        assert_eq!(t.players[0].objects[0].life.dying_sprite, None);
    }

    /// A speed's change reaches what a unit of the master moves with (the game reads its master).
    #[test]
    fn a_speed_change_reaches_the_units_movement() {
        let mut walker = unit_of(1, 4, 25.0);
        let mover = crate::hunt::tests::walker();
        let stats = mover.stats;
        walker.mover = Some(mover);
        let player = Player {
            objects: vec![walker],
            ..Player::default()
        };
        let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
        let mut villager = combat(4, 4, 25);
        villager.max_speed = 0.8;
        villager.moving = Some(stats);
        t.masters = vec![Masters::new([villager])];
        t.do_effect(0, &[attribute(5, -1, 4, 5, 1.1)], None)
            .unwrap();
        let s = t.players[0].objects[0].mover.as_ref().unwrap().stats;
        // no graphics: the sprites' factors are 0, the speeds the master's
        let speed = 0.8_f32 * 1.1;
        assert_eq!(
            (s.speed, s.walk_speed, s.stand_speed),
            (speed, speed, speed)
        );
        // a speed the loader left at 0 under the moving values stops the change first
        t.masters[0].get_mut(4).unwrap().max_speed = 0.0;
        assert!(
            t.do_effect(0, &[attribute(5, -1, 4, 5, 1.1)], None)
                .is_err()
        );
        let stats = t.players[0].objects[0].mover.as_ref().unwrap().stats;
        assert_eq!(stats.speed, speed);
    }

    /// The resource commands (kind 1, `FUN_141620c70`): mode 0 sets the attribute, any other adds
    /// to it, scaled by the attribute the third argument names; kind 6 multiplies it
    /// (`FUN_1416614c0`).
    #[test]
    fn the_resource_commands_set_add_and_multiply_an_attribute() {
        let player = Player {
            attributes: vec![0.0; 300],
            ..Player::default()
        };
        let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
        t.players[0].attributes[69] = 2.0;
        t.do_effect(0, &[attribute(1, 78, 0, -1, 0.15)], None)
            .unwrap();
        // Horse Collar: 75 × attribute 69 added to 36
        t.do_effect(0, &[attribute(1, 36, 1, 69, 75.0)], None)
            .unwrap();
        t.do_effect(0, &[attribute(6, 78, -1, -1, 2.0)], None)
            .unwrap();
        // gold, a resource, lives beside the attributes
        t.do_effect(0, &[attribute(1, 3, 1, -1, 50.0)], None)
            .unwrap();
        let a = &t.players[0].attributes;
        assert_eq!(
            (a[78], a[36], t.players[0].resources[3]),
            (0.3, 150.0, 50.0)
        );
    }

    /// A change of the farms' food (resource 0x24) grows the food each farm still makes:
    /// `(f / before)^exponent × amount + f − held` with `f = held + still to make`
    /// (`FUN_141615270`), the exponent attribute 31.
    #[test]
    fn a_farm_food_change_grows_the_farms_food() {
        let mut farm = unit_of(5, 50, 480.0);
        farm.life.amount = 75.0;
        farm.construction = Some(built());
        farm.make = Some(crate::make::MakeAction {
            state: 1,
            remaining: 100.0,
            total: 175.0,
            task: crate::masters::Task::default(),
        });
        let player = Player {
            objects: vec![farm],
            attributes: vec![0.0; 300],
            ..Player::default()
        };
        let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
        let mut master = combat(50, 49, 480);
        master.kind = 80;
        t.masters = vec![Masters::new([master])];
        t.players[0].attributes[31] = 1.0;
        t.players[0].attributes[36] = 175.0;
        // the load files the objects in the players' trackers
        t.load_trackers();
        t.do_effect(0, &[attribute(1, 36, 1, -1, 75.0)], None)
            .unwrap();
        assert_eq!(t.players[0].attributes[36], 250.0);
        // (175 / 175)^1 × 75 + 175 − 75
        assert_eq!(t.players[0].objects[0].make.unwrap().remaining, 175.0);
    }

    /// Cartography (resource 50, added): the allied vision turns on and the masks are rebuilt
    /// before the attribute takes its sum (`FUN_141616820`); Treason's set (0xb7) rebuilds them
    /// after; resource 0xc4 spreads to the players at most 0 unless the player's is above 0; the
    /// scripts' effect function (0x21 above 0) is not modelled.
    #[test]
    fn the_resource_commands_side_effects() {
        let player = |relations: Vec<u8>| Player {
            relations,
            attributes: vec![0.0; 300],
            ..Player::default()
        };
        let mut t = Turns::new(
            0,
            0,
            0,
            vec![0; 3],
            vec![
                player(vec![0, 3, 3]),
                player(vec![3, 0, 0]),
                player(vec![3, 0, 0]),
            ],
        );
        t.spatial.vision = vec![[0; 2]; 3];
        t.do_effect(1, &[attribute(1, 50, 1, -1, 1.0)], None)
            .unwrap();
        assert!(t.players[1].allied_vision);
        assert_eq!(t.players[1].attributes[50], 1.0);
        assert_eq!(t.spatial.vision[1][0] & 0b110, 0b110);
        t.do_effect(2, &[attribute(1, 0xc4, 0, -1, 7.0)], None)
            .unwrap();
        let spread: Vec<f32> = t.players.iter().map(|p| p.attributes[0xc4]).collect();
        assert_eq!(spread, [0.0, 7.0, 7.0]);
        t.do_effect(1, &[attribute(1, 0xc4, 0, -1, 9.0)], None)
            .unwrap();
        assert_eq!(t.players[1].attributes[0xc4], 7.0);
        t.do_effect(1, &[attribute(1, 0xb7, 0, -1, 1.0)], None)
            .unwrap();
        assert_eq!(t.spatial.vision[1][0] & 0b110, 0b110);
        assert!(
            t.do_effect(1, &[attribute(1, 0x21, 0, -1, 7.0)], None)
                .is_err()
        );
    }

    /// A technology at the barracks (master 12, 30 seconds in the `.dat`) in state 1 with 20
    /// seconds of work done.
    fn barracks_tech() -> Turns {
        use crate::tech::{Location, TechDef, TechState};
        let state = TechState {
            state: 1,
            research_done: 20.0,
            locations: vec![Location {
                building: 12,
                time: 30,
                button: 3,
                adjustment: 0.0,
            }],
            building_master: -1,
            ..TechState::default()
        };
        let player = Player {
            techs: vec![state],
            attributes: vec![0.0; 300],
            ..Player::default()
        };
        let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
        t.techs = vec![TechDef {
            required: [-1; 6],
            locations: vec![(12, 30)],
            ..TechDef::default()
        }];
        t
    }

    /// `FUN_1413d1210`: mode 0 sets the time (the adjustment is the value less the `.dat`'s
    /// time), mode 1 adds the truncated value, mode 2 adds `(v − 1) × time` rounded half away
    /// from 1; once the work done reaches the new time a technology in state 1 completes.
    #[test]
    fn the_research_time_modifier_changes_the_adjustment_and_may_complete() {
        let mut t = barracks_tech();
        t.do_effect(0, &[attribute(b'g', 0, -1, 1, 4.9)], None)
            .unwrap();
        assert_eq!(t.players[0].techs[0].locations[0].adjustment, 4.0);
        t.do_effect(0, &[attribute(b'g', 0, -1, 2, 0.8)], None)
            .unwrap();
        // (0.8 − 1) × 30 − 0.5 = −6.5, truncated −6, plus 4
        assert_eq!(t.players[0].techs[0].locations[0].adjustment, -2.0);
        assert_eq!(t.players[0].techs[0].state, 1);
        // set to 20: the adjustment −10, and 20 seconds are done
        t.do_effect(0, &[attribute(b'g', 0, -1, 0, 20.0)], None)
            .unwrap();
        assert_eq!(t.players[0].techs[0].locations[0].adjustment, -10.0);
        assert_eq!(t.players[0].techs[0].state, 3);
    }

    /// Kind 8 (`FUN_1415c3f20`'s technology change): a cost mode changes a cost (0x4002: add to
    /// stone), 4 a location's building to a master the player has, 5 its button, 10 the research
    /// cap, 12 the state, −1 the research time (with the dependents' update).
    #[test]
    fn a_technology_change_reaches_its_costs_locations_and_state() {
        use crate::tech::Cost;
        let mut t = barracks_tech();
        t.techs[0].costs[0] = Cost {
            attribute: 2,
            amount: 100,
            paid: 1,
        };
        t.masters = vec![Masters::new([combat(87, 3, 1000)])];
        t.do_effect(0, &[attribute(8, 0, 0x4002, -1, 25.0)], None)
            .unwrap();
        assert_eq!(t.players[0].techs[0].cost_adj, [25, 0, 0]);
        // a building the player lacks leaves the location
        t.do_effect(0, &[attribute(8, 0, 4, -1, 88.0)], None)
            .unwrap();
        t.do_effect(0, &[attribute(8, 0, 4, -1, 87.0)], None)
            .unwrap();
        t.do_effect(0, &[attribute(8, 0, 5, -1, 7.0)], None)
            .unwrap();
        t.do_effect(0, &[attribute(8, 0, 10, -1, 2.0)], None)
            .unwrap();
        let s = &t.players[0].techs[0];
        assert_eq!((s.locations[0].building, s.locations[0].button), (87, 7));
        assert_eq!(s.research_cap, 2);
        // a location past the list adds a copy of the last, without a building
        t.do_effect(0, &[attribute(8, 0, 5, 3, 9.0)], None).unwrap();
        let s = &t.players[0].techs[0];
        assert_eq!(s.locations.len(), 2);
        assert_eq!((s.locations[1].building, s.locations[1].button), (-1, 9));
        t.do_effect(0, &[attribute(8, 0, 12, -1, 0.0)], None)
            .unwrap();
        assert_eq!(t.players[0].techs[0].state, -1);
        t.do_effect(0, &[attribute(8, 0, 12, -1, 2.0)], None)
            .unwrap();
        assert_eq!(
            (t.players[0].techs[0].state, t.players[0].techs[0].enabled),
            (1, true)
        );
        // state 3 completes it (its first location has a building and a button: it counts);
        // once researched, again gives its effect and counts whatever its location
        t.do_effect(0, &[attribute(8, 0, 12, -1, 3.0)], None)
            .unwrap();
        assert_eq!(t.players[0].techs[0].state, 3);
        assert_eq!(t.players[0].attributes[0x15], 1.0);
        t.do_effect(0, &[attribute(8, 0, 12, -1, 3.0)], None)
            .unwrap();
        assert_eq!(t.players[0].attributes[0x15], 2.0);
    }

    /// Kind 8's other modes: a technology past the player's tree changes nothing; a location
    /// within the list changes it alone (−2, a research time's add: at its building); one past
    /// the list of a technology without locations changes nothing; 9 the stacking, 12 with 1
    /// makes it unavailable; 18 the effect override (a researched technology's new effect would
    /// run from the world's effect table, not modelled).
    #[test]
    fn a_technology_change_s_location_stacking_and_effect_override() {
        use crate::tech::Location;
        let mut t = barracks_tech();
        t.players[0].techs[0].locations.push(Location {
            building: 13,
            time: 40,
            button: 4,
            adjustment: 0.0,
        });
        t.do_effect(0, &[attribute(8, 5, 9, -1, 1.0)], None)
            .unwrap();
        assert!(!t.players[0].techs[0].stack);
        t.do_effect(0, &[attribute(8, 0, 9, -1, 1.0)], None)
            .unwrap();
        assert!(t.players[0].techs[0].stack);
        t.do_effect(0, &[attribute(8, 0, 5, 1, 9.0)], None).unwrap();
        t.do_effect(0, &[attribute(8, 0, -2, 1, 5.9)], None)
            .unwrap();
        let s = &t.players[0].techs[0];
        let at = |i: usize| (s.locations[i].button, s.locations[i].adjustment);
        assert_eq!([at(0), at(1)], [(3, 0.0), (9, 5.0)]);
        t.do_effect(0, &[attribute(8, 0, 12, -1, 1.0)], None)
            .unwrap();
        assert_eq!(t.players[0].techs[0].state, 0);
        t.do_effect(0, &[attribute(8, 0, 18, -1, 7.0)], None)
            .unwrap();
        assert_eq!(t.players[0].techs[0].effect_override, 7);
        t.players[0].techs[0].state = 3;
        assert!(
            t.do_effect(0, &[attribute(8, 0, 18, -1, 8.0)], None)
                .is_err()
        );
        // no locations: a location past the list changes nothing
        let mut t = barracks_tech();
        t.players[0].techs[0].locations.clear();
        t.do_effect(0, &[attribute(8, 0, 5, 2, 9.0)], None).unwrap();
        assert_eq!(t.players[0].techs[0].locations.len(), 0);
    }

    /// Kind 3 (`FUN_14160e410`, mode −1): the old master takes the new one's values but keeps its
    /// id, class, availability and disabling (`Master::copy_from`); its objects first take the
    /// new one through their slot 0x98 ([`Turns::upgrade_object`]).
    #[test]
    fn an_upgrade_copies_the_new_master_into_the_old() {
        let player = Player::default();
        let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
        let mut militia = combat(74, 6, 40);
        militia.available = true;
        let mut man_at_arms = combat(75, 6, 45);
        man_at_arms.weapons = vec![(4, 6)];
        man_at_arms.available = false;
        t.masters = vec![Masters::new([militia, man_at_arms])];
        t.do_effect(0, &[attribute(3, 74, 75, -1, 0.0)], None)
            .unwrap();
        let m = t.masters[0].get(74).unwrap();
        assert_eq!(
            (m.id, m.hit_points, m.weapons.clone()),
            (74, 45, vec![(4, 6)])
        );
        assert!(m.available);
        // mode 1 leaves the master
        t.masters[0].get_mut(75).unwrap().hit_points = 50;
        t.do_effect(0, &[attribute(3, 74, 75, 1, 0.0)], None)
            .unwrap();
        assert_eq!(t.masters[0].get(74).unwrap().hit_points, 45);
        // with an object: its slot 0x98 keeps its share of the hit points (45 of 45: 50 of 50)
        t.players[0].objects.push(unit_of(1, 74, 45.0));
        t.do_effect(0, &[attribute(3, 74, 75, -1, 0.0)], None)
            .unwrap();
        let u = &t.players[0].objects[0];
        assert_eq!((u.life.master_id, u.life.hp), (74, 50.0));
        assert_eq!(t.masters[0].get(74).unwrap().hit_points, 50);
    }

    /// Kind 7 (`FUN_1416136f0`) makes objects at the built buildings of a master, which this file
    /// leaves to the spawning; with none built it does nothing. Kind 5 on a cost by a negative
    /// amount removes (−2) or adds (−1) a cost entry (`FUN_1415c1f70`).
    #[test]
    fn spawns_and_cost_entries() {
        let mut centre = unit_of(1, 109, 2400.0);
        centre.life.state = 0;
        let player = Player {
            objects: vec![centre],
            ..Player::default()
        };
        let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
        let mut tc = combat(109, 80, 2400);
        tc.kind = 80;
        let mut serjeant = combat(1658, 6, 60);
        serjeant.cost = vec![(0, 60, true), (2, 35, true)];
        t.masters = vec![Masters::new([tc, serjeant])];
        t.do_effect(0, &[attribute(7, 1658, 109, 5, 0.0)], None)
            .unwrap();
        t.players[0].objects[0].life.state = 2;
        assert!(
            t.do_effect(0, &[attribute(7, 1658, 109, 5, 0.0)], None)
                .is_err()
        );
        t.do_effect(0, &[attribute(5, 1658, -1, 106, -2.0)], None)
            .unwrap();
        assert_eq!(t.masters[0].get(1658).unwrap().cost, [(0, 60, true)]);
        t.do_effect(0, &[attribute(5, 1658, -1, 105, -1.0)], None)
            .unwrap();
        assert_eq!(
            t.masters[0].get(1658).unwrap().cost,
            [(0, 60, true), (3, -1, true)]
        );
    }

    /// The objects' kinds: 200 to 202 change the objects with masters of their own (none held);
    /// 203 to 206 need the effect's object, which a research has not.
    #[test]
    fn the_object_kinds_without_an_object_do_nothing() {
        let mut t = Turns::new(0, 0, 0, vec![0], vec![Player::default()]);
        t.masters = vec![Masters::new([combat(82, 52, 4800)])];
        for kind in 200..=206 {
            t.do_effect(0, &[attribute(kind, 82, -1, 8, 770.0)], None)
                .unwrap();
        }
        assert!(
            t.do_effect(0, &[attribute(204, 82, -1, 0, 1500.0)], Some(5))
                .is_err()
        );
        assert_eq!(t.masters[0].get(82).unwrap().hit_points, 4800);
    }

    /// The enable command (`FUN_1416665b0`, kind 2): `available` takes the byte unless the master
    /// is disabled; master 0x26d is never enabled in game mode 11.
    #[test]
    fn the_enable_command_sets_available_unless_disabled() {
        let mut t = Turns::new(0, 0, 0, vec![0], vec![Player::default()]);
        let mut locked = combat(5, 6, 10);
        locked.disabled = true;
        t.masters = vec![Masters::new([
            combat(4, 6, 10),
            locked,
            combat(0x26d, 3, 10),
        ])];
        t.do_effect(
            0,
            &[attribute(2, 4, 1, -1, 0.0), attribute(2, 5, 1, -1, 0.0)],
            None,
        )
        .unwrap();
        let available: Vec<bool> = t.masters[0].iter().map(|m| m.available).collect();
        assert_eq!(available, [true, false, false]);
        t.settings.game_mode = 11;
        t.do_effect(0, &[attribute(2, 0x26d, 1, -1, 0.0)], None)
            .unwrap();
        assert!(!t.masters[0].get(0x26d).unwrap().available);
        t.settings.game_mode = 0;
        assert!(
            t.do_effect(0, &[attribute(2, 0x26d, 1, -1, 0.0)], None)
                .is_err()
        );
        t.do_effect(0, &[attribute(2, 4, 0, -1, 0.0)], None)
            .unwrap();
        assert!(!t.masters[0].get(4).unwrap().available);
    }

    /// A research whose effect reaches its building (the object `FUN_1413cdcd0` passes, which its
    /// callers do not yet) stops before it changes anything; one with an effect override too.
    #[test]
    fn a_research_whose_effect_needs_its_building_stops() {
        use crate::tech::{TechDef, TechState};
        let state = TechState {
            state: 2,
            building_master: -1,
            ..TechState::default()
        };
        let player = Player {
            techs: vec![state.clone(), state],
            attributes: vec![0.0; 300],
            ..Player::default()
        };
        let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
        let def = |effect| TechDef {
            required: [-1; 6],
            effect_id: 7,
            effect,
            ..TechDef::default()
        };
        t.techs = vec![
            def(vec![attribute(204, 82, -1, 0, 1500.0)]),
            def(vec![attribute(4, -1, 4, 0, 15.0)]),
        ];
        assert!(t.complete_research(0, 0, None).is_err());
        assert_eq!(t.players[0].techs[0].state, 2);
        t.players[0].techs[1].effect_override = 9;
        assert!(t.complete_research(0, 1, None).is_err());
        t.players[0].techs[1].effect_override = 7;
        assert_eq!(t.complete_research(0, 1, None), Ok(true));
    }

    /// An age (101 to 103) completes as any research and adds its event (0 to 2) to the
    /// player's history at the world's time (`FUN_1413cdcd0`'s end); another research adds none.
    #[test]
    fn an_age_adds_its_event_to_the_history() {
        use crate::tech::{TechDef, TechState};
        let state = TechState {
            state: 2,
            building_master: -1,
            ..TechState::default()
        };
        let player = Player {
            techs: vec![state; 104],
            attributes: vec![0.0; 300],
            ..Player::default()
        };
        let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
        t.techs = vec![
            TechDef {
                required: [-1; 6],
                ..TechDef::default()
            };
            104
        ];
        t.world_time = 612_345;
        t.players[0].history.entries = vec![[3, 0]; 20];
        assert_eq!(t.complete_research(0, 100, None), Ok(true));
        assert_eq!(t.players[0].history.events, []);
        assert_eq!(t.complete_research(0, 102, None), Ok(true));
        assert_eq!(t.players[0].techs[102].state, 3);
        assert_eq!(
            t.players[0].history.events,
            [crate::history::Event {
                kind: crate::history::CASTLE_AGE,
                time_slice: 20,
                world_time: 612_345,
                ..crate::history::Event::default()
            }]
        );
    }

    /// A technology whose requirement is researched and whose research takes no time at a
    /// location (`FUN_1413cce70`'s call of `FUN_1413cdcd0`) is done at once, and so are those
    /// that depend on it in turn; one still missing a requirement stays unavailable.
    #[test]
    fn a_dependent_technology_done_already_completes_with_its_requirement() {
        use crate::tech::{Location, TechDef, TechState};
        let def = |required: i16, count| TechDef {
            required: [required, -1, -1, -1, -1, -1],
            required_count: count,
            ..TechDef::default()
        };
        let free = TechState {
            locations: vec![Location {
                building: 109,
                ..Location::default()
            }],
            building_master: -1,
            ..TechState::default()
        };
        let player = Player {
            techs: vec![TechState::default(), free.clone(), free.clone(), free],
            ..Player::default()
        };
        let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
        t.techs = vec![def(-1, 0), def(0, 1), def(1, 1), def(0, 2)];
        assert_eq!(t.complete_research(0, 0, None), Ok(true));
        let states: Vec<i16> = t.players[0].techs.iter().map(|s| s.state).collect();
        assert_eq!(states, [3, 3, 3, 0]);
    }

    /// `FUN_1413cce70`'s civilisation test: a dependent of another civilisation turns −1; one of
    /// its own of mode 0 does too under the full tech tree (`+0x63`), not one of mode 1; one for
    /// every civilisation (civ 0) is available either way.
    #[test]
    fn the_full_tech_tree_disables_a_civ_s_own_technologies_but_those_of_mode_1() {
        use crate::tech::{TechDef, TechState};
        let def = |required: i16, civ: i16, mode: i16| TechDef {
            required: [required, -1, -1, -1, -1, -1],
            required_count: i16::from(required != -1),
            civ,
            mode,
            ..TechDef::default()
        };
        let states = |full_tech_tree: bool| {
            let player = Player {
                civ: 5,
                techs: vec![TechState::default(); 5],
                ..Player::default()
            };
            let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
            t.settings.full_tech_tree = full_tech_tree;
            t.techs = vec![
                def(-1, 0, 0),
                def(0, 5, 0),
                def(0, 5, 1),
                def(0, 7, 0),
                def(0, 0, 0),
            ];
            assert_eq!(t.complete_research(0, 0, None), Ok(true));
            t.players[0]
                .techs
                .iter()
                .map(|s| s.state)
                .collect::<Vec<i16>>()
        };
        assert_eq!(states(false), [3, 1, 1, -1, 1]);
        assert_eq!(states(true), [3, -1, 1, -1, 1]);
    }

    /// The technology commands of an effect: the disable command (`'f'`, MarvinBlue/426048737's
    /// civ effect disabling technology 238 in update 1) sets a technology to −1 but a researched
    /// one, one of type 0x21 to 1, and leaves one of the shared kind (a building's own); the cost
    /// modifier (`'e'`) adds to, sets or scales a paid cost entry's adjustment.
    #[test]
    fn the_technology_commands_disable_and_adjust_costs() {
        use crate::tech::{Cost, TechDef, TechState};
        let state = |s: i16| TechState {
            state: s,
            ..TechState::default()
        };
        let player = Player {
            techs: vec![state(1), state(3), state(0), state(1)],
            ..Player::default()
        };
        let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
        let kind = |k: i16| TechDef {
            kind: k,
            costs: [
                Cost {
                    attribute: 0,
                    amount: 150,
                    paid: 1,
                },
                Cost {
                    attribute: 3,
                    amount: 50,
                    paid: 1,
                },
                Cost::default(),
            ],
            ..TechDef::default()
        };
        t.techs = vec![kind(0), kind(0), kind(0x21), kind(crate::tech::SHARED_KIND)];
        let command = |kind: u8, a: i16, b: i16, c: i16, d: f32| EffectCommand { kind, a, b, c, d };
        for tech in 0..4 {
            t.do_effect(0, &[command(b'f', -1, -1, -1, tech as f32)], None)
                .unwrap();
        }
        let states: Vec<i16> = t.players[0].techs.iter().map(|s| s.state).collect();
        assert_eq!(states, [-1, 3, 1, 1]);
        // gold of technology 0 costs 10 more; food set to 100; every paid cost scaled by 0.5
        t.do_effect(0, &[command(b'e', 0, 3, 1, 10.0)], None)
            .unwrap();
        assert_eq!(t.players[0].techs[0].cost_adj, [0, 10, 0]);
        t.do_effect(0, &[command(b'e', 0, 0, 0, 100.0)], None)
            .unwrap();
        assert_eq!(t.players[0].techs[0].cost_adj, [-50, 10, 0]);
        t.do_effect(0, &[command(b'e', 1, -1, 2, 0.5)], None)
            .unwrap();
        assert_eq!(t.players[0].techs[1].cost_adj, [-75, -25, 0]);
        // the research time modifier changes a location's adjustment; a technology without
        // locations has none to change
        t.do_effect(0, &[command(b'g', 0, -1, 1, 5.0)], None)
            .unwrap();
    }

    /// `FUN_1413cdb00` reads the id as a short (`MOVZX EDX, BX` in `FUN_14163a500`): 65538 is
    /// technology 2, 65535 is −1, every technology.
    #[test]
    fn the_disable_command_reads_its_id_s_low_16_bits() {
        use crate::tech::{TechDef, TechState};
        let state = |s: i16| TechState {
            state: s,
            ..TechState::default()
        };
        let player = Player {
            techs: vec![state(0), state(0), state(0)],
            ..Player::default()
        };
        let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
        t.techs = vec![TechDef::default(); 3];
        let disable = |d: f32| EffectCommand {
            kind: b'f',
            a: -1,
            b: -1,
            c: -1,
            d,
        };
        t.do_effect(0, &[disable(65538.0)], None).unwrap();
        let states: Vec<i16> = t.players[0].techs.iter().map(|s| s.state).collect();
        assert_eq!(states, [0, 0, -1]);
        t.do_effect(0, &[disable(65535.0)], None).unwrap();
        let states: Vec<i16> = t.players[0].techs.iter().map(|s| s.state).collect();
        assert_eq!(states, [-1, -1, -1]);
    }

    /// `FUN_1413d10f0` converts with `CVTTSS2SI` and adds the low word: an amount of 2^31 or
    /// more adds 0 (0x80000000's low word), where a saturating cast would add −1.
    #[test]
    fn a_cost_change_past_the_i32_range_adds_nothing() {
        use crate::tech::{Cost, TechDef, TechState};
        let player = Player {
            techs: vec![TechState::default()],
            ..Player::default()
        };
        let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
        t.techs = vec![TechDef {
            costs: [
                Cost {
                    attribute: 0,
                    amount: 150,
                    paid: 1,
                },
                Cost::default(),
                Cost::default(),
            ],
            ..TechDef::default()
        }];
        let change = EffectCommand {
            kind: b'e',
            a: 0,
            b: 0,
            c: 1,
            d: 3.0e9,
        };
        t.do_effect(0, &[change], None).unwrap();
        assert_eq!(t.players[0].techs[0].cost_adj, [0, 0, 0]);
        // mode 2 at 1.5: (1.5 − 1) × 150 + 0.5 = 75.5, truncated to 75
        let scale = EffectCommand {
            c: 2,
            d: 1.5,
            ..change
        };
        t.do_effect(0, &[scale], None).unwrap();
        assert_eq!(t.players[0].techs[0].cost_adj, [75, 0, 0]);
    }

    /// A dead building's master technology goes back to 0 (`FUN_1413d4ec0`) when no other
    /// building of the master stands, and only from a state above 0.
    #[test]
    fn the_last_building_of_a_master_withdraws_its_technology() {
        use crate::tech::{TechDef, TechState};
        let building = |id, state| {
            let mut u = Unit {
                id,
                ..Unit::default()
            };
            u.life.master_id = 1640;
            u.life.state = state;
            u
        };
        let player = Player {
            objects: vec![building(1, 3), building(2, 2)],
            techs: vec![TechState {
                state: 3,
                ..TechState::default()
            }],
            ..Player::default()
        };
        let mut t = Turns::new(0, 0, 0, vec![0], vec![player]);
        t.techs = vec![TechDef {
            required: [-1; 6],
            ..TechDef::default()
        }];
        t.withdraw_tech(0, 1640, 0).unwrap();
        assert_eq!(t.players[0].techs[0].state, 3);
        t.players[0].objects.truncate(1);
        t.withdraw_tech(0, 1640, 0).unwrap();
        assert_eq!(t.players[0].techs[0].state, 0);
    }
}

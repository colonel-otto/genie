//! A research's completion and the effects it gives: `FUN_1413cdcd0` (the technology done, its
//! effect, the technologies that depend on it, the player's statistics), the effects'
//! `TRIBE_Effects::do_effect` (`FUN_141462440`, then the base `FUN_140ec2ac0`) and the player's
//! attribute modifier (slot 0x158 -> slot 0x148, `FUN_1416121b0`: the hooks `FUN_141614f40`
//! around `FUN_14165a000`).

use crate::masters::{AttributeWorld, Master, Masters, master_type};
use crate::rng::Stream;
use crate::tech::{EffectCommand, is_age};
use crate::turn::{Turns, Unit};
use crate::unit_ai::Unsupported;

/// The attribute counting the technologies researched (`0x15`) and the one summing their costs
/// (`0x63`), which a research done at a location with a button adds to.
const TECHS_RESEARCHED: i16 = 0x15;
const RESEARCH_SPENT: i16 = 0x63;
/// The attribute the dependents' update sets to 1 when none of them is left (`+0x138`).
const ATTRIBUTE_39: usize = 39;
/// The master whose enabling first asks the game's mode (`0x26d`, `FUN_1416665b0`).
const MODE_TESTED_MASTER: i16 = 0x26d;
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
    /// 0x63. Not held: the local player's statistics and achievements. Not modelled: a research
    /// paid in relics (`FUN_1413d0e70`), an effect override, an age's history (`FUN_141560ae0`)
    /// and the civ's disabled technologies (`FUN_140ea93a0`; such a technology is never
    /// available to be researched).
    pub(crate) fn complete_research(
        &mut self,
        p: usize,
        t: usize,
        building: Option<i16>,
    ) -> Result<bool, Unsupported> {
        use crate::tech::Attributes;
        let def = self.techs[t].clone();
        let state = &self.players[p].techs[t];
        if !(0..=2).contains(&state.state) {
            return Ok(false);
        }
        if def.costs[0].attribute == 7 {
            return Err(Unsupported("a research paid in relics (FUN_1413d0e70)"));
        }
        if state.effect_override != -1 && state.effect_override != def.effect_id {
            return Err(Unsupported("a technology's effect override"));
        }
        if is_age(t as i16) {
            return Err(Unsupported(
                "an age's completion (its history, FUN_141560ae0)",
            ));
        }
        self.players[p].techs[t].state = 3;
        self.do_effect(p, def.effect_id, &def.effect, None)?;
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
        Ok(true)
    }

    /// A completed building's technology `t` for player `p` (the end of `FUN_1411dbb70`'s
    /// `set_state(2)`): [`Turns::complete_research`] with `param_3` 0 and no building, so the
    /// technology's first location decides its statistics; a technology beyond the player's tree
    /// is not researched.
    pub(crate) fn research_built(&mut self, p: usize, t: i16) -> Result<(), Unsupported> {
        let tree = self.players[p].techs.len();
        if tree == 0 || self.techs.len() != tree {
            return Err(Unsupported(
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
    ) -> Result<(), Unsupported> {
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
    /// `+0x60` turns to −1 when it is another civ's (or its civ's under the game's option
    /// `+0x63` while not of mode 1, not modelled) or not for this game mode; else with fewer of
    /// its requirements researched than it needs it turns to 0 (from above 0), with them all to
    /// 1 (from below 1) unless its research is done already (its work done reaching a
    /// location's time), which completes it ([`Turns::complete_research`]). Each such technology, and each in
    /// state 2, counts; none counted sets attribute 39 to 1 where it is 0. The game's lists hold
    /// the dependents by id, as the `.dat`'s requirements give them.
    pub(crate) fn update_dependents(&mut self, p: usize, t: usize) -> Result<(), Unsupported> {
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
            // another civ's: −1; the civ's own depends on the game's option +0x63 unless of mode 1
            let other_civ = if def.civ < 1 {
                false
            } else if def.civ == civ {
                if def.mode != 1 {
                    return Err(Unsupported(
                        "a civ's own technology in the dependents' update (the option +0x63)",
                    ));
                }
                false
            } else {
                true
            };
            if other_civ {
                self.players[p].techs[d].state = -1;
                continue;
            }
            // FUN_1413cf2f0: modes 0 and 1 are for every game
            if !matches!(def.mode, 0 | 1) {
                return Err(Unsupported(
                    "a technology for some game modes (FUN_1413cf2f0)",
                ));
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
    fn disable_tech(
        &mut self,
        p: usize,
        tech: i32,
        object: Option<i32>,
    ) -> Result<(), Unsupported> {
        let count = self.players[p].techs.len().min(self.techs.len());
        let held = usize::try_from(tech).ok().filter(|&t| t < count);
        if held.is_some_and(|t| self.techs[t].kind == crate::tech::SHARED_KIND) {
            let building = object.and_then(|id| self.object(id)).is_some_and(|(q, u)| {
                self.master_of(q, u.life.master_id)
                    .is_ok_and(|m| m.kind == master_type::BUILDING)
            });
            if building {
                return Err(Unsupported("a research of the shared kind (FUN_1411c49a0)"));
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
    /// (slot 0xa8) is not simulation. Not modelled: the enabling of master `0x26d` (it asks the
    /// game's mode and settings) and a command for one building (`FUN_1411c69f0`,
    /// `FUN_1411c6a50`: the building's list, not held).
    fn enable_master(
        &mut self,
        p: usize,
        master: i16,
        enable: u8,
        one: bool,
        object: Option<i32>,
    ) -> Result<(), Unsupported> {
        if self.masters.get(p).and_then(|m| m.get(master)).is_none() {
            return Ok(());
        }
        if enable == 1 && master == MODE_TESTED_MASTER {
            return Err(Unsupported(
                "the enabling of master 0x26d (the game's mode)",
            ));
        }
        if let (true, Some(id)) = (one, object) {
            let building = self.object(id).is_some_and(|(q, u)| {
                self.master_of(q, u.life.master_id)
                    .is_ok_and(|m| m.kind == master_type::BUILDING)
            });
            if building {
                return Err(Unsupported("an enable command for one building"));
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
/// The effect whose spawn commands (kind 7) count for the local player's statistics (`0x318`).
const COUNTED_SPAWNS: i16 = 0x318;
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

    /// `TRIBE_Effects::do_effect` (`FUN_141462440`) of effect `effect_id`, its commands `effect`,
    /// for player `p` with the effect's object `object` (none for a research's): first its
    /// technology commands in order, the cost modifier (`'e'`, [`Turns::tech_cost`]), the disable
    /// command (`'f'`, an id other than −1, [`Turns::disable_tech`]) and the research time
    /// modifier (`'g'`, [`Turns::research_time`]); then the base's ([`Turns::base_effect`]).
    pub(crate) fn do_effect(
        &mut self,
        p: usize,
        effect_id: i16,
        effect: &[EffectCommand],
        object: Option<i32>,
    ) -> Result<(), Unsupported> {
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
        self.base_effect(p, effect_id, effect, object)
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
        effect_id: i16,
        effect: &[EffectCommand],
        object: Option<i32>,
    ) -> Result<(), Unsupported> {
        let mut farms: Option<(f32, f32)> = None;
        for c in effect {
            if kind::OBJECTS_SET <= c.kind && c.kind <= kind::OBJECT_UPGRADE {
                self.object_effect(p, c, object)?;
                continue;
            }
            if 4 < c.kind / 10 || 8 < c.kind % 10 {
                continue;
            }
            let party = match c.kind / 10 {
                0 => Party::Own,
                1 => Party::Allies,
                2 => Party::Enemies,
                3 => Party::Neutrals,
                _ => Party::Gaia,
            };
            let own = party == Party::Own;
            let attribute = (c.c & 0xff) as u8;
            match c.kind % 10 {
                kind::SET | kind::ADD => {
                    let change = if c.kind % 10 == kind::SET {
                        Change::Set
                    } else {
                        Change::Add
                    };
                    for q in self.party(p, party) {
                        self.modify_attribute(q, change, c.a, c.b, c.d, attribute)?;
                    }
                }
                kind::MULTIPLY => {
                    // a cost attribute by a negative amount (COMISS 0, d; JBE: not a NaN) is a
                    // cost entry's change, but in Gaia's block (FUN_141666ee0)
                    let cost = COST_ATTRIBUTES.contains(&c.c) && 0.0 > c.d && party != Party::Gaia;
                    for q in self.party(p, party) {
                        if cost {
                            let unit = if c.b < 0 { c.a } else { c.b };
                            self.cost_entries(q, unit, c.c, crate::checkpoint::cvttss2si(c.d))?;
                        } else {
                            self.modify_attribute(q, Change::Multiply, c.a, c.b, c.d, attribute)?;
                        }
                    }
                }
                kind::RESOURCE => {
                    // the effect's player's attribute `c` scales the amount
                    let amount = self.scaled_amount(p, c);
                    if own && c.a == FARM_FOOD {
                        farms = Some((self.attribute_of(p, FARM_FOOD), amount));
                    } else if c.a < 0 {
                        continue;
                    }
                    let mode = (c.b & 0xff) as u8;
                    for q in self.party(p, party) {
                        self.modify_resource(q, c.a, amount, mode)?;
                    }
                }
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
                kind::RESOURCE_MULTIPLY => {
                    if 0 <= c.a {
                        for q in self.party(p, party) {
                            self.multiply_resource(q, c.a, c.d)?;
                        }
                    }
                }
                kind::SPAWN => {
                    let counted = own && effect_id == COUNTED_SPAWNS;
                    for q in self.party(p, party) {
                        self.spawn_at(q, c.a, c.b, c.c, counted)?;
                    }
                }
                kind::TECH => {
                    for q in self.party(p, party) {
                        self.modify_tech(q, c.a, c.b, c.d, c.c)?;
                    }
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

/// The classes the player's object tracker files under no class or apart from the buildings'
/// pending list (`FUN_141658430`): 51 and 54 count as done from the start.
const NEVER_PENDING: [i16; 2] = [51, 54];
/// The tracker's ids run below this (`FUN_141658430`: a master id of 9000 or more is none).
const TRACKED_MASTERS: i16 = 9000;
/// The resources the population attribute (110) moves an object's storage of (`FUN_1412163c0`):
/// headroom (4), and current population (11) and 19, which hold its magnitude.
const HEADROOM: i16 = 4;
const POPULATION: i16 = 11;
const RESOURCE_19: i16 = 19;
/// A storage flag of `'@'` (64): the object's storage stays with it (`FUN_141195a50`).
const KEPT_STORAGE: u8 = 64;
/// A hit point total a capped multiply holds an object's below (`FUN_1411962e0`: `0x46fffe00`).
const OBJECT_HP_CAP: f32 = 32767.0;
/// A multiply's rounding (`DAT_142e7f384`, 0.5; an image role to add: `effect::ROUND`).
const ROUND: f32 = 0.5;

/// The key a player's object tracker (`player+0x18`) files an object under (`FUN_141658430`):
/// its key master (the stack unit a building master names, else its own; the unit lines' table
/// that keys some masters by their line's, `FUN_140fdd6c0`, is not held), the key master's class
/// when below 100 (none for an annex: a head and no annexes of its own), and whether it waits in
/// the pending lists (a building, but of class 51 or 54, until its completion moves it,
/// `FUN_14104eac0`). None: the tracker's default key (−1), for a master the player lacks or one
/// of 9000 or more.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TrackerKey {
    master: i16,
    class: Option<i16>,
    pending: bool,
}

fn tracker_key(masters: &Masters, u: &Unit) -> Option<TrackerKey> {
    let m = masters
        .get(u.life.master_id)
        .filter(|m| m.id < TRACKED_MASTERS)?;
    let mut key = m;
    let mut classed = true;
    let mut flagged = false;
    if m.kind == master_type::BUILDING {
        if let Some(b) = m.building.as_ref() {
            if b.stack_unit != -1 {
                key = masters
                    .get(b.stack_unit)
                    .filter(|s| s.id < TRACKED_MASTERS)?;
            }
            if b.head_unit != -1 && b.annexes[0] == -1 {
                classed = false;
            }
        }
        flagged = true;
    }
    let class = (classed && (0..100).contains(&key.class)).then_some(key.class);
    if class.is_some_and(|c| NEVER_PENDING.contains(&c)) {
        flagged = false;
    }
    // a building leaves the pending lists when its build points reach its build time (its
    // state 2); one dying still pending never got there
    let done = match u.life.state {
        0 | 1 => false,
        2 => true,
        _ => u
            .construction
            .as_ref()
            .is_none_or(|c| f32::from(c.build_time) <= c.progress),
    };
    Some(TrackerKey {
        master: key.id,
        class,
        pending: flagged && !done,
    })
}

impl Turns {
    /// What the masters' setters read of the world for player `p` ([`AttributeWorld`]): the
    /// sprite table (the `.dat`'s graphics), no handicap (`player+0x668`, not held: none in the
    /// games probed) and the player's `+0x648` unset (not held: 0 for every player probed).
    fn attribute_world(
        graphics: &std::collections::BTreeMap<i16, crate::sprite::Graphic>,
    ) -> impl Fn(i32) -> bool + '_ {
        move |id| i16::try_from(id).is_ok_and(|id| graphics.contains_key(&id))
    }

    /// The ids of player `p`'s objects its tracker files under class `class` (`FUN_14104f8f0`,
    /// key class + 900) and those pending under it (`FUN_14104f9e0`, key class + 90900), each
    /// in the player's lists' order: the live objects, then the sleeping ones (the tracker's
    /// own order, by filing, is not held; the changes to the objects do not depend on it).
    fn tracked_class(&self, p: usize, class: i16) -> (Vec<i32>, Vec<i32>) {
        let (mut done, mut pending) = (Vec::new(), Vec::new());
        let Some(masters) = self.masters.get(p) else {
            return (done, pending);
        };
        let player = &self.players[p];
        for u in player.objects.iter().chain(&player.sleeping) {
            match tracker_key(masters, u) {
                Some(k) if k.class == Some(class) && !k.pending => done.push(u.id),
                Some(k) if k.class == Some(class) => pending.push(u.id),
                _ => {}
            }
        }
        (done, pending)
    }

    /// The ids of player `p`'s objects of master `master` its tracker files under it
    /// (`FUN_14104f960`; `FUN_14165a860` keeps those of that very master), and for a building
    /// master those pending under it, or under the stack unit it heads (`FUN_14104fa50`, key
    /// + 90000), whatever their masters.
    fn tracked_master(&self, p: usize, master: &Master) -> (Vec<i32>, Vec<i32>) {
        let (mut done, mut pending) = (Vec::new(), Vec::new());
        let Some(masters) = self.masters.get(p) else {
            return (done, pending);
        };
        let player = &self.players[p];
        let objects = || player.objects.iter().chain(&player.sleeping);
        for u in objects().filter(|u| u.life.master_id == master.id) {
            if tracker_key(masters, u).is_some_and(|k| k.master == master.id && !k.pending) {
                done.push(u.id);
            }
        }
        if master_type::BUILDING <= master.kind {
            let stack = master.building.as_ref().map_or(-1, |b| b.stack_unit);
            let key = masters
                .get(stack)
                .filter(|s| {
                    s.kind == master_type::BUILDING
                        && s.building
                            .as_ref()
                            .is_some_and(|b| b.head_unit == master.id)
                })
                .map_or(master.id, |s| s.id);
            for u in objects() {
                if tracker_key(masters, u).is_some_and(|k| k.master == key && k.pending) {
                    pending.push(u.id);
                }
            }
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
    ) -> Result<bool, Unsupported> {
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
    ) -> Result<(), Unsupported> {
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
    ) -> Result<(), Unsupported> {
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
    fn population_storage(&mut self, p: usize, master: &Master, v: f32) -> Result<(), Unsupported> {
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
    ) -> Result<(), Unsupported> {
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
/// points (an image role to add: `effect::ONE`).
const ONE: f32 = 1.0;
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
        || u.missile.is_some()
        || u.make.is_some()
        || u.production.as_ref().is_some_and(|q| q.action)
}

impl Turns {
    /// Slot 0x148 (`FUN_1416121b0`) of player `p`: the hooks before (`FUN_141614f40` with 0),
    /// the change of attribute `attribute` by `v` on master `unit` or on the masters and
    /// objects of class `class` ([`Turns::change_attribute`]), the hooks after (with 1).
    pub(crate) fn modify_attribute(
        &mut self,
        p: usize,
        change: Change,
        unit: i16,
        class: i16,
        v: f32,
        attribute: u8,
    ) -> Result<(), Unsupported> {
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
    ) -> Result<(), Unsupported> {
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
    fn garrison_volleys(&mut self, p: usize, id: i32, after: bool) -> Result<(), Unsupported> {
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
    fn show_changed_sprite(&mut self, p: usize, id: i32, attribute: u8) -> Result<(), Unsupported> {
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
        let moving = g(master.move_sprite);
        let attack = g(master.attack_sprite);
        let standing = g(master.standing_sprite);
        let combat = master_type::COMBAT <= master.kind;
        let idle_attack = g(master.idle_attack_sprite);
        let second = g(master.attack_sprite2);
        let follow = |arms: &mut crate::attack::Arms, task: Option<[i16; 4]>| {
            let of_task = |s: Option<crate::sprite::Graphic>, i: usize| {
                task.zip(s).is_some_and(|(t, s)| t[i] == s.id)
            };
            if arms.move_sprite.is_some() && !of_task(arms.move_sprite, 0) {
                arms.move_sprite = moving;
            }
            if arms.sprite.is_some() && !of_task(arms.sprite, 1) && !of_task(arms.sprite, 2) {
                arms.sprite = attack;
            }
            if arms.idle_sprite.is_some()
                && !of_task(arms.idle_sprite, 1)
                && !of_task(arms.idle_sprite, 2)
            {
                arms.idle_sprite = standing;
            }
            if combat {
                arms.idle_attack_sprite = idle_attack;
                arms.second_sprite = second;
            }
        };
        let (_, u) = self.object_mut(id).expect("found above");
        // a unit's own Attack's task (the work command's attack task) has no sprites
        if let Some(attack) = u.attack.as_mut() {
            follow(&mut attack.arms, None);
        }
        if let Some(hunt) = u.hunt.as_mut() {
            let task = hunt.task.sprites;
            if let Some(attack) = hunt.attack.as_mut() {
                follow(&mut attack.arms, Some(task));
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
    ) -> Result<(), Unsupported> {
        let Some(masters) = self.masters.get(p) else {
            return Ok(());
        };
        let table = masters
            .iter()
            .map(|m| i32::from(m.id) + 1)
            .max()
            .unwrap_or(0);
        if 0 <= unit && i32::from(unit) < table {
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
        let ids: Vec<i16> = self.masters[p]
            .iter()
            .filter(|m| m.class == class)
            .map(|m| m.id)
            .collect();
        for id in ids {
            self.change_one_master(p, id, change, v, attribute);
        }
        Ok(())
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
    ) -> Result<(), Unsupported> {
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
                        return Err(Unsupported(
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

impl Turns {
    /// Slot 0x1b0 (`FUN_141620c70`) of player `p` on resource `attribute`. Not yet ported.
    fn modify_resource(
        &mut self,
        _p: usize,
        _attribute: i16,
        _amount: f32,
        _mode: u8,
    ) -> Result<(), Unsupported> {
        Err(Unsupported("an effect command the slice does not model"))
    }

    /// Slot 0x1b8 (`FUN_141620e30`). Not yet ported.
    fn multiply_resource(
        &mut self,
        _p: usize,
        _attribute: i16,
        _v: f32,
    ) -> Result<(), Unsupported> {
        Err(Unsupported("an effect command the slice does not model"))
    }

    /// Slot 0x1a0 (`FUN_14160e410`). Not yet ported.
    fn upgrade_master(
        &mut self,
        _p: usize,
        _from: i16,
        _to: i16,
        _mode: i16,
        _object: Option<i32>,
    ) -> Result<(), Unsupported> {
        Err(Unsupported("an effect command the slice does not model"))
    }

    /// Slot 0x1a8 (`FUN_1416136f0`). Not yet ported.
    fn spawn_at(
        &mut self,
        _p: usize,
        _master: i16,
        _at: i16,
        _count: i16,
        _counted: bool,
    ) -> Result<(), Unsupported> {
        Err(Unsupported("an effect command the slice does not model"))
    }

    /// `FUN_1415c3f20` with 8. Not yet ported.
    fn modify_tech(
        &mut self,
        _p: usize,
        _tech: i16,
        _mode: i16,
        _v: f32,
        _location: i16,
    ) -> Result<(), Unsupported> {
        Err(Unsupported("an effect command the slice does not model"))
    }

    /// `FUN_1415c1f70`. Not yet ported.
    fn cost_entries(
        &mut self,
        _p: usize,
        _unit: i16,
        _attribute: i16,
        _how: i32,
    ) -> Result<(), Unsupported> {
        Err(Unsupported("an effect command the slice does not model"))
    }

    /// `FUN_141615270`. Not yet ported.
    fn farm_food(&mut self, _p: usize, _before: f32, _amount: f32) -> Result<(), Unsupported> {
        Err(Unsupported("an effect command the slice does not model"))
    }

    /// The objects' kinds 200 to 206. Not yet ported.
    fn object_effect(
        &mut self,
        _p: usize,
        _c: &EffectCommand,
        _object: Option<i32>,
    ) -> Result<(), Unsupported> {
        Err(Unsupported("an effect command the slice does not model"))
    }

    /// `FUN_1413d1210`. Not yet ported.
    fn research_time(
        &mut self,
        _p: usize,
        _tech: i16,
        _v: f32,
        _mode: u8,
        _location: i16,
    ) -> Result<(), Unsupported> {
        Err(Unsupported(
            "an effect's research time command ('g', FUN_1413d1210)",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::masters::{Master, Masters};
    use crate::turn::{Player, Unit};

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
        t.do_effect(0, -1, &loom, None).unwrap();
        let hp: Vec<f32> = t.players[0].objects.iter().map(|u| u.life.hp).collect();
        // (25 + 15) × 10 / 25 = 16
        assert_eq!(hp, [40.0, 16.0]);
        let m = t.masters[0].get(83).unwrap();
        assert_eq!((m.hit_points, m.armors.clone()), (40, vec![(4, 1), (3, 2)]));
    }

    /// A unit of `master` with `hp` hit points.
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
        t.do_effect(0, -1, &[attribute(0, 4, 6, 0, 90.0)], None)
            .unwrap();
        let hp: Vec<f32> = t.players[0].objects.iter().map(|u| u.life.hp).collect();
        assert_eq!(hp, [45.0, 30.0]);
        let hps: Vec<i16> = t.masters[0].iter().map(|m| m.hit_points).collect();
        assert_eq!(hps, [90, 60, 60]);
        // master 6 lies within the table (up to 7) but has no master: nothing, not the class
        t.do_effect(0, -1, &[attribute(4, 6, 6, 0, 10.0)], None)
            .unwrap();
        let hps: Vec<i16> = t.masters[0].iter().map(|m| m.hit_points).collect();
        assert_eq!(hps, [90, 60, 60]);
        // past the table the class changes: (60 + 10) × 30 / 60 = 35
        t.do_effect(0, -1, &[attribute(4, 100, 6, 0, 10.0)], None)
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
        t.do_effect(0, -1, &[attribute(4, -1, 0, 9, f32::from(0x302_i16))], None)
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
        let mut annex = unit_of(21, 31, 50.0);
        annex.annex_of = Some(20);
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
        t.do_effect(0, -1, &[attribute(5, -1, 3, 0, 1.5)], None)
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
        t.do_effect(1, -1, &[attribute(14, -1, 6, 0, 5.0)], None)
            .unwrap();
        let hps: Vec<i16> = t
            .masters
            .iter()
            .map(|m| m.get(4).unwrap().hit_points)
            .collect();
        assert_eq!(hps, [60, 65, 65, 60]);
        t.do_effect(1, -1, &[attribute(44, -1, 6, 0, 5.0)], None)
            .unwrap();
        let hps: Vec<i16> = t
            .masters
            .iter()
            .map(|m| m.get(4).unwrap().hit_points)
            .collect();
        assert_eq!(hps, [65, 65, 65, 60]);
        // the enemies' block (24): player 3 counts player 1 an enemy and is counted one
        t.do_effect(1, -1, &[attribute(24, -1, 6, 0, 5.0)], None)
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
        t.do_effect(0, -1, &[attribute(0, 70, -1, 21, 15.0)], None)
            .unwrap();
        assert_eq!(t.players[0].attributes[4], 30.0);
        assert_eq!(t.masters[0].get(70).unwrap().storage[0].1, 15.0);
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
        t.do_effect(0, -1, &[attribute(5, -1, 4, 5, 1.1)], None)
            .unwrap();
        let s = t.players[0].objects[0].mover.as_ref().unwrap().stats;
        // no graphics: the sprites' factors are 0, the speeds the master's
        let speed = 0.8_f32 * 1.1;
        assert_eq!(
            (s.speed, s.walk_speed, s.stand_speed),
            (speed, speed, speed)
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
            t.do_effect(0, -1, &[command(b'f', -1, -1, -1, tech as f32)], None)
                .unwrap();
        }
        let states: Vec<i16> = t.players[0].techs.iter().map(|s| s.state).collect();
        assert_eq!(states, [-1, 3, 1, 1]);
        // gold of technology 0 costs 10 more; food set to 100; every paid cost scaled by 0.5
        t.do_effect(0, -1, &[command(b'e', 0, 3, 1, 10.0)], None)
            .unwrap();
        assert_eq!(t.players[0].techs[0].cost_adj, [0, 10, 0]);
        t.do_effect(0, -1, &[command(b'e', 0, 0, 0, 100.0)], None)
            .unwrap();
        assert_eq!(t.players[0].techs[0].cost_adj, [-50, 10, 0]);
        t.do_effect(0, -1, &[command(b'e', 1, -1, 2, 0.5)], None)
            .unwrap();
        assert_eq!(t.players[0].techs[1].cost_adj, [-75, -25, 0]);
        // the research time modifier is not modelled
        assert!(
            t.do_effect(0, -1, &[command(b'g', 0, -1, 1, 5.0)], None)
                .is_err()
        );
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
        t.do_effect(0, -1, &[disable(65538.0)], None).unwrap();
        let states: Vec<i16> = t.players[0].techs.iter().map(|s| s.state).collect();
        assert_eq!(states, [0, 0, -1]);
        t.do_effect(0, -1, &[disable(65535.0)], None).unwrap();
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
        t.do_effect(0, -1, &[change], None).unwrap();
        assert_eq!(t.players[0].techs[0].cost_adj, [0, 0, 0]);
        // mode 2 at 1.5: (1.5 − 1) × 150 + 0.5 = 75.5, truncated to 75
        let scale = EffectCommand {
            c: 2,
            d: 1.5,
            ..change
        };
        t.do_effect(0, -1, &[scale], None).unwrap();
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

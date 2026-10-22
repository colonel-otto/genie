//! A research's completion and the effects it gives: `FUN_1413cdcd0` (the technology done, its
//! effect, the technologies that depend on it, the player's statistics), the effects'
//! `TRIBE_Effects::do_effect` (`FUN_141462440`, then the base `FUN_140ec2ac0`) and the player's
//! attribute modifier (slot 0x158 -> slot 0x148, `FUN_1416121b0`: the hooks `FUN_141614f40`
//! around `FUN_14165a000`).

use crate::tech::{EffectCommand, is_age};
use crate::turn::Turns;
use crate::unit_ai::Unsupported;

/// The attribute counting the technologies researched (`0x15`) and the one summing their costs
/// (`0x63`), which a research done at a location with a button adds to.
const TECHS_RESEARCHED: i16 = 0x15;
const RESEARCH_SPENT: i16 = 0x63;
/// The attribute the dependents' update sets to 1 when none of them is left (`+0x138`).
const ATTRIBUTE_39: usize = 39;
/// The effect command kind that adds to an attribute (4) and the object and master attributes
/// it is modelled for: the hit points (0) and the armor (8, the class in the amount's high byte,
/// the amount in its low byte).
const ATTRIBUTE_ADD: u8 = 4;
/// The effect command kind that enables or disables a master (2, `FUN_1416665b0`).
const ENABLE_MASTER: u8 = 2;
/// The master whose enabling first asks the game's mode (`0x26d`, `FUN_1416665b0`).
const MODE_TESTED_MASTER: i16 = 0x26d;
const HIT_POINTS: i16 = 0;
const ARMOR: i16 = 8;
/// The `TRIBE_Effects` command kinds that change technologies (`'e'`, `'f'`, `'g'`): a cost, the
/// disabling and a research time.
const TECH_KINDS: std::ops::RangeInclusive<u8> = b'e'..=b'g';
const TECH_COST: u8 = b'e';
const TECH_DISABLE: u8 = b'f';
const TECH_TIME: u8 = b'g';
/// The technology type that a disable command makes available instead (`FUN_1413cdb00`: 0x21).
const REENABLED_KIND: i16 = 0x21;
/// A researched technology's state, which a disable command keeps (`FUN_1413cdb00`).
const RESEARCHED: i16 = 3;
/// The combat master type, whose attribute modifier (slot 0x20, `FUN_141266a60`) is modelled.
const COMBAT: u8 = 70;

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
        self.do_effect(p, &def.effect)?;
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

    /// `TRIBE_Effects::do_effect` (`FUN_141462440`) of the commands `effect` for player `p`: first
    /// its technology commands in order, the cost modifier (`'e'`, [`Turns::tech_cost`]) and the
    /// disable command (`'f'`, [`Turns::disable_tech`]; the research time modifier `'g'`,
    /// `FUN_1413d1210`, is not modelled); then the base's (`FUN_140ec2ac0`), of which the
    /// attribute modifier (kind 4, [`Turns::attribute_add`]) and the enable command (kind 2) are
    /// modelled. The effect's object (`param_4`) is none: a research's.
    pub(crate) fn do_effect(
        &mut self,
        p: usize,
        effect: &[EffectCommand],
    ) -> Result<(), Unsupported> {
        for c in effect {
            // the technology commands read the third argument's low byte as the mode
            let mode = (c.c & 0xff) as u8;
            match c.kind {
                TECH_COST => self.tech_cost(p, c.a, c.b, c.d, mode),
                TECH_DISABLE => {
                    let tech = c.d as i32;
                    if tech != -1 {
                        self.disable_tech(p, tech);
                    }
                }
                TECH_TIME => {
                    return Err(Unsupported(
                        "an effect's research time command ('g', FUN_1413d1210)",
                    ));
                }
                _ => {}
            }
        }
        for c in effect.iter().filter(|c| !TECH_KINDS.contains(&c.kind)) {
            match c.kind {
                ATTRIBUTE_ADD => self.attribute_add(p, c.a, c.b, c.d, c.c)?,
                ENABLE_MASTER if 0 <= c.a => self.enable_master(p, c.a, c.b, c.c)?,
                ENABLE_MASTER => {}
                _ => return Err(Unsupported("an effect command the slice does not model")),
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

    /// The disable command (`'f'`, `FUN_14163a500` without an object) of player `p`'s technology
    /// `tech`: one of the shared kind (`FUN_1413d1020`) is a building's own, which a command
    /// without the building lets be; any other goes through `FUN_1413cdb00`: a technology of type
    /// 0x21 turns to 1 (it may be researched), any other but a researched one (state 3) to −1;
    /// a negative id does so to every technology.
    fn disable_tech(&mut self, p: usize, tech: i32) {
        let count = self.players[p].techs.len().min(self.techs.len());
        let held = usize::try_from(tech).ok().filter(|&t| t < count);
        if held.is_some_and(|t| self.techs[t].kind == crate::tech::SHARED_KIND) {
            return;
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
    }

    /// The enable command (`FUN_1416665b0`) of player `p` on its master `master` (`enable`'s low
    /// byte 1 or 0): a master the player does not have is let be (its id check against `+0x260`
    /// is taken to hold); otherwise the master's slot 0x58 (`FUN_141126280`) sets `available`
    /// unless it is `disabled`. Its sprites' loading on an enable (slot 0xa8) is not simulation.
    /// Not modelled: the enabling of master `0x26d` (it asks the game's mode) and a command for
    /// one building (`which` 1 with the object, `FUN_1411c69f0`, `FUN_1411c6a50`).
    fn enable_master(
        &mut self,
        p: usize,
        master: i16,
        enable: i16,
        which: i16,
    ) -> Result<(), Unsupported> {
        let Some(m) = self.masters.get_mut(p).and_then(|m| m.get_mut(master)) else {
            return Ok(());
        };
        let enable = enable as u8;
        if enable == 1 && master == MODE_TESTED_MASTER {
            return Err(Unsupported(
                "the enabling of master 0x26d (the game's mode)",
            ));
        }
        if which == 1 {
            return Err(Unsupported("an enable command for one building"));
        }
        if !m.disabled {
            m.available = enable != 0;
        }
        Ok(())
    }

    /// The attribute modifier (slot 0x158 -> slot 0x148 with 1, `FUN_1416121b0`) of player `p`:
    /// `amount` added to attribute `attribute` of the master `unit`, else of every master of
    /// `class` and the player's objects of it (`FUN_14165a000`). Each object of the class first
    /// (slot 0x80, `FUN_1412167d0` -> `FUN_141196060`): its hit points take the master's added
    /// share, `(short)((master's + amount) × its own / master's)`; then each master of the class
    /// by id (slot 0x20, `FUN_141266a60` down to `FUN_141126af0`): its hit points grow by the
    /// amount (16 bits), and an armor amount (its low byte; its high byte the class) adds to the
    /// armor of that class it lists. The hooks around it (`FUN_141614f40`) change a building's
    /// garrison bonus (`FUN_141217ac0`, `FUN_14120b910`), not modelled with units garrisoned,
    /// and an object's abilities (`+0x930`, none held). Not modelled: one master, the other
    /// attributes, an object of the class asleep or annexed, other master types.
    pub(crate) fn attribute_add(
        &mut self,
        p: usize,
        unit: i16,
        class: i16,
        amount: f32,
        attribute: i16,
    ) -> Result<(), Unsupported> {
        let masters = self
            .masters
            .get(p)
            .ok_or(Unsupported("an effect for a player without masters"))?;
        if 0 <= unit && masters.get(unit).is_some() {
            return Err(Unsupported("an effect on one master (FUN_14165a400)"));
        }
        if class < 0 {
            return Ok(());
        }
        if !matches!(attribute, HIT_POINTS | ARMOR) {
            return Err(Unsupported(
                "an effect on an attribute the slice does not model",
            ));
        }
        let owner = &self.players[p];
        if owner.objects.iter().any(|u| u.life.inside.is_some()) {
            return Err(Unsupported(
                "an effect while units are garrisoned (FUN_141217ac0)",
            ));
        }
        let of_class = |id: i16| masters.get(id).is_some_and(|m| m.class == class);
        if owner.sleeping.iter().any(|u| of_class(u.life.master_id)) {
            return Err(Unsupported("an effect on a sleeping object"));
        }
        if owner.objects.iter().any(|u| {
            u.annex_of.is_some_and(|a| {
                owner
                    .objects
                    .iter()
                    .any(|b| b.id == a && of_class(b.life.master_id))
            })
        }) {
            return Err(Unsupported("an effect on a building with annexes"));
        }
        if attribute == HIT_POINTS {
            for u in &mut self.players[p].objects {
                let Some(m) = masters.get(u.life.master_id).filter(|m| m.class == class) else {
                    continue;
                };
                let base = f32::from(m.hit_points);
                u.life.hp = ((((base + amount) * u.life.hp) / base) as i32 as i16).into();
            }
        }
        let masters = self.masters.get_mut(p).expect("above");
        for m in masters.iter_mut().filter(|m| m.class == class) {
            if m.kind != COMBAT {
                return Err(Unsupported("an effect on a master of another type"));
            }
            if attribute == HIT_POINTS {
                m.hit_points = m.hit_points.wrapping_add(amount as i32 as i16);
            } else {
                let (code, delta) = if amount < 0.0 {
                    let u = (-amount) as i32 as u16;
                    (u, (u & 0xff).wrapping_neg())
                } else {
                    let u = amount as i32 as u16;
                    (u, u & 0xff)
                };
                let armor_class = (code as i16) >> 8;
                // the master's armor list (+0x808) may be missing: then nothing changes
                if let Some(a) = m.armors.iter_mut().find(|a| a.0 == armor_class) {
                    a.1 = a.1.wrapping_add(delta as i16);
                }
            }
        }
        Ok(())
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
        t.do_effect(0, &loom).unwrap();
        let hp: Vec<f32> = t.players[0].objects.iter().map(|u| u.life.hp).collect();
        // (25 + 15) × 10 / 25 = 16
        assert_eq!(hp, [40.0, 16.0]);
        let m = t.masters[0].get(83).unwrap();
        assert_eq!((m.hit_points, m.armors.clone()), (40, vec![(4, 1), (3, 2)]));
        // a garrisoned unit's building would change its garrison bonus: not modelled
        t.players[0].objects[1].life.inside = Some(1);
        assert!(t.do_effect(0, &loom).is_err());
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
            t.do_effect(0, &[command(b'f', -1, -1, -1, tech as f32)])
                .unwrap();
        }
        let states: Vec<i16> = t.players[0].techs.iter().map(|s| s.state).collect();
        assert_eq!(states, [-1, 3, 1, 1]);
        // gold of technology 0 costs 10 more; food set to 100; every paid cost scaled by 0.5
        t.do_effect(0, &[command(b'e', 0, 3, 1, 10.0)]).unwrap();
        assert_eq!(t.players[0].techs[0].cost_adj, [0, 10, 0]);
        t.do_effect(0, &[command(b'e', 0, 0, 0, 100.0)]).unwrap();
        assert_eq!(t.players[0].techs[0].cost_adj, [-50, 10, 0]);
        t.do_effect(0, &[command(b'e', 1, -1, 2, 0.5)]).unwrap();
        assert_eq!(t.players[0].techs[1].cost_adj, [-75, -25, 0]);
        // the research time modifier is not modelled
        assert!(t.do_effect(0, &[command(b'g', 0, -1, 1, 5.0)]).is_err());
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
        t.do_effect(0, &[disable(65538.0)]).unwrap();
        let states: Vec<i16> = t.players[0].techs.iter().map(|s| s.state).collect();
        assert_eq!(states, [0, 0, -1]);
        t.do_effect(0, &[disable(65535.0)]).unwrap();
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
        t.do_effect(0, &[change]).unwrap();
        assert_eq!(t.players[0].techs[0].cost_adj, [0, 0, 0]);
        // mode 2 at 1.5: (1.5 − 1) × 150 + 0.5 = 75.5, truncated to 75
        let scale = EffectCommand {
            c: 2,
            d: 1.5,
            ..change
        };
        t.do_effect(0, &[scale]).unwrap();
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

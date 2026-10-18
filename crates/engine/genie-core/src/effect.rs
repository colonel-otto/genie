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
const HIT_POINTS: i16 = 0;
const ARMOR: i16 = 8;
/// The `TRIBE_Effects` command kinds that change technologies (`'e'`, `'f'`, `'g'`).
const TECH_KINDS: std::ops::RangeInclusive<u8> = b'e'..=b'g';
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

    /// `FUN_1413cce70` for player `p`'s technology `t`: each technology that requires it, by id
    /// (`+0x38`'s lists), not of the shared kind: one in state 0 or 1 and not held by
    /// `+0x60` turns to −1 when it is another civ's (or its civ's under the game's option
    /// `+0x63` while not of mode 1, not modelled) or not for this game mode; else with fewer of
    /// its requirements researched than it needs it turns to 0 (from above 0), with them all to
    /// 1 (from below 1) unless its research is done already (its work done reaching a
    /// location's time, which completes it, not modelled). Each such technology, and each in
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
            let done = if state.building_master == -1 {
                state.locations.iter().any(reached)
            } else {
                state.location(state.building_master).is_some_and(reached)
            };
            if done {
                return Err(Unsupported(
                    "a dependent technology done on its requirements (FUN_1413cdcd0)",
                ));
            }
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

    /// `TRIBE_Effects::do_effect` (`FUN_141462440`) of the commands `effect` for player `p`: the
    /// technology commands (`'e'` to `'g'`) are not modelled; then the base's
    /// (`FUN_140ec2ac0`), of which the attribute modifier (kind 4,
    /// [`Turns::attribute_add`]) is modelled.
    pub(crate) fn do_effect(
        &mut self,
        p: usize,
        effect: &[EffectCommand],
    ) -> Result<(), Unsupported> {
        if effect.iter().any(|c| TECH_KINDS.contains(&c.kind)) {
            return Err(Unsupported(
                "an effect's technology commands (FUN_1413d10f0)",
            ));
        }
        for c in effect {
            match c.kind {
                ATTRIBUTE_ADD => self.attribute_add(p, c.a, c.b, c.d, c.c)?,
                _ => return Err(Unsupported("an effect command the slice does not model")),
            }
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
}

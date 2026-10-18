//! The players' technologies (`TRIBE_Player_Tech`, `player+0x760`): the `.dat`'s technology
//! records ([`TechDef`], the tree's `+0x28`, 0x88 bytes each) and each player's state of each
//! ([`TechState`], `Tech_Player_Tree`, 0x80 bytes, saved). 158041: the cost list
//! `FUN_1413d4a00`, the research test `FUN_1413cd600`, the payment `FUN_1413d4370` (from
//! `FUN_14162fd90`), the repeat test `FUN_1413d1050`.

use crate::unit_ai::Unsupported;

/// The age technologies (`FUN_1413cc3c0`: 101 to 103), whose costs the research cost modifier
/// leaves alone.
pub fn is_age(tech: i16) -> bool {
    (101..104).contains(&tech)
}

/// Spies (0x198), whose cost grows with the enemies' villagers (`FUN_1413cbd80`).
const SPIES: i16 = 0x198;
/// The technology whose cost adjustments scale by attribute 38 (0x2f3) instead of 1.
const SCALED_BY_38: i16 = 0x2f3;
/// The attribute that scales research costs above 0 (`+0x2a8`: 85).
const RESEARCH_COST_MODIFIER: i16 = 85;
/// The attribute 0x2f3's cost adjustments scale by (`+0x130`).
const ATTRIBUTE_38: i16 = 38;
/// The resource a cost the player cannot meet takes the rest from (`+0x18`: gold).
const GOLD: i16 = 3;
/// The technology type whose research the buildings that may do it share (`FUN_1413d1020`).
pub const SHARED_KIND: i16 = 0x20;

/// One cost entry of a technology in the `.dat`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Cost {
    /// The attribute it takes (−1 none).
    pub attribute: i16,
    pub amount: i16,
    /// 0 not paid, 1 paid from the player's attribute, 2 from the building (`FUN_141204310`).
    pub paid: u8,
}

/// A technology of the `.dat` as the tech tree holds it (its 0x88-byte record).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TechDef {
    /// `+0x2a`, `+0x30`, `+0x36`: the cost entries' attributes, amounts and paid flags.
    pub costs: [Cost; 3],
    /// `+0x3c`, the technology's type ([`SHARED_KIND`]).
    pub kind: i16,
    /// Its effect (`+0x3a`) and the effect's commands (the world's effect table `+0x128`).
    pub effect_id: i16,
    pub effect: Vec<EffectCommand>,
    /// `+0x18`, `+0x24`: the technologies it requires and how many of them.
    pub required: [i16; 6],
    pub required_count: i16,
    /// `+0x26`: the civ it belongs to (below 1 every civ's); `+0x28`: the game modes it is for
    /// (0 every one, `FUN_1413cf2f0`).
    pub civ: i16,
    pub mode: i16,
    /// `+0x70`: where it is researched, the building's master and the seconds it takes.
    pub locations: Vec<(i16, i16)>,
}

/// One command of an effect (12 bytes): its kind and arguments.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EffectCommand {
    pub kind: u8,
    pub a: i16,
    pub b: i16,
    pub c: i16,
    pub d: f32,
}

/// The effect command kinds whose first preparation (`FUN_140ec4540` from the research's start,
/// `FUN_1413d1530`) marks a master with the unit it turns into (`+0x330`).
const MARKS_UPGRADE: [u8; 5] = [3, 0x0d, 0x17, 0x21, 0x2b];

impl TechDef {
    /// Its effect upgrades a unit (a command of kind 3), which keeps RESEARCH's queue to one at
    /// a time (`FUN_1411d70f0`).
    pub fn upgrades(&self) -> bool {
        self.effect.iter().any(|c| c.kind == 3)
    }

    /// Its effect marks a master with an upgrade when its research starts ([`MARKS_UPGRADE`]).
    pub fn marks_upgrade(&self) -> bool {
        self.effect.iter().any(|c| MARKS_UPGRADE.contains(&c.kind))
    }
}

/// Where a technology is researched (an entry of `+0x68`, 16 bytes): the building's master,
/// the seconds it takes there, its button and the seconds added (`+0xc`, `FUN_1413cec90`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Location {
    pub building: i16,
    pub time: u16,
    pub button: u8,
    pub adjustment: f32,
}

/// A player's state of one technology (`Tech_Player_Tree`, 0x80 bytes, saved).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TechState {
    /// `+0x0`: the research's work done.
    pub research_done: f32,
    /// `+0x4`: 1 when it may be researched, 4 once queued (−1 and 0 never).
    pub state: i16,
    /// `+0x6`: added to the `.dat`'s amounts.
    pub cost_adj: [i16; 3],
    /// `+0xc`.
    pub times_researched: i16,
    /// `+0xe`.
    pub allow_multiple: bool,
    /// `+0x40`: its researches queued and done are held to its cap.
    pub stack: bool,
    /// `+0x42`, `+0x48`, `+0x4e`: the cost overrides' amounts, attributes and use (−2 the `.dat`'s
    /// entry where it is paid, 0 none, else the override).
    pub overrides: [(i16, i16, i16); 3],
    /// `+0x56`: the cap of its researches queued and done (below 0: 0x100).
    pub research_cap: i16,
    /// `+0x58`: its researches queued.
    pub pending: i16,
    /// `+0x60` (`mEnabledTech`): the dependents' update leaves it alone.
    pub enabled: bool,
    /// `+0x62`: the effect it gives instead of its own (−1 none).
    pub effect_override: i16,
    /// `+0x64`: the master of the building it was last queued at.
    pub building_master: i16,
    /// `+0x68`.
    pub locations: Vec<Location>,
}

/// The two factors of a technology's costs: the research cost modifier (attribute 85, none
/// for the age technologies), which scales the costs above 0, and the scale of the cost
/// adjustments (1; attribute 38 for 0x2f3). Spies is not modelled.
pub fn factors(
    tech: i16,
    attribute: &dyn Fn(i16) -> Result<f32, Unsupported>,
) -> Result<(f32, i16), Unsupported> {
    if tech == SPIES {
        return Err(Unsupported("the cost of Spies (FUN_1413cbd80)"));
    }
    let modifier = if is_age(tech) {
        0.0
    } else {
        attribute(RESEARCH_COST_MODIFIER)?
    };
    let scale = if tech == SCALED_BY_38 {
        attribute(ATTRIBUTE_38)? as i32 as i16
    } else {
        1
    };
    Ok((modifier, scale))
}

impl TechState {
    /// `FUN_1413cbd20`: the location at the building's master `building`, else the first.
    pub fn location(&self, building: i16) -> Option<&Location> {
        self.locations
            .iter()
            .find(|l| l.building == building)
            .or(self.locations.first())
    }

    /// `FUN_1411c3190` for a research in state 2 or 4 (else none): the costs its queue entry
    /// records, attribute and amount: an override in use (or a `.dat` entry without an attribute,
    /// under −2) gives its attribute and amount, else the `.dat`'s entry; the adjustment times
    /// the scale adds to the amount (in 32 bits), which the modifier above 0 then scales (not
    /// truncated). An entry without either is (−1, 0).
    pub fn entry_costs(&self, def: &TechDef, (modifier, scale): (f32, i16)) -> [(i32, f32); 3] {
        let mut out = [(-1, 0.0); 3];
        if !matches!(self.state, 2 | 4) {
            return out;
        }
        for (j, slot) in out.iter_mut().enumerate() {
            let (amount, attribute, used) = self.overrides[j];
            let c = def.costs[j];
            let (attribute, base) = match used {
                0 => continue,
                -2 if 0 <= c.attribute => (c.attribute, c.amount),
                _ => (attribute, amount),
            };
            if attribute < 0 {
                continue;
            }
            let mut v = (i32::from(self.cost_adj[j]) * i32::from(scale) + i32::from(base)) as f32;
            if 0.0 < modifier {
                v *= modifier;
            }
            *slot = (i32::from(attribute), v);
        }
        out
    }

    /// The research's start test (`FUN_1413cbef0`) against what its queue entry `paid`: each
    /// cost with a paid `.dat` entry needs the player's attribute and what the entry paid of that
    /// attribute to cover its amount; false when one does not. A difference left to pay
    /// (`FUN_1411c68b0`) and a cost the building holds are not modelled.
    pub fn start_test(
        &self,
        def: &TechDef,
        factors: (f32, i16),
        paid: &[(i32, f32); 3],
        attribute: &dyn Fn(i16) -> Result<f32, Unsupported>,
    ) -> Result<bool, Unsupported> {
        let (modifier, scale) = factors;
        for j in 0..3 {
            let (amount, attr, used) = self.overrides[j];
            let c = def.costs[j];
            let attr = if used == 1 { attr } else { c.attribute };
            if attr < 0 || c.paid == 0 {
                continue;
            }
            let mut v = if used == 1 {
                f32::from(amount)
            } else {
                (i32::from(self.cost_adj[j]) * i32::from(scale)) as f32 + f32::from(c.amount)
            };
            if 0.0 < modifier {
                v = (v * modifier) as i32 as i16 as f32;
            }
            let before: f32 = paid
                .iter()
                .filter(|&&(k, _)| k != -1 && k == i32::from(attr))
                .map(|&(_, a)| a)
                .fold(0.0, |s, a| s + a);
            if c.paid == 2 {
                return Err(Unsupported(
                    "a research cost the building holds (FUN_141204310)",
                ));
            }
            if attribute(attr)? + before < v {
                return Ok(false);
            }
            if 0.0 < v - before {
                return Err(Unsupported(
                    "a research's start that pays a difference (FUN_1411c68b0)",
                ));
            }
        }
        Ok(true)
    }

    /// `FUN_1413d4a00`: the three costs, attribute and amount, as the pending debit and the
    /// building's queue read them. An override in use (or a `.dat` entry not paid, under −2)
    /// gives its attribute and amount, else the `.dat`'s entry does; the adjustment times the
    /// scale adds to the amount, in 16 bits, which the modifier then scales (truncated). An
    /// override of 0 gives none; an amount below 1 leaves no attribute.
    pub fn listed_costs(&self, def: &TechDef, (modifier, scale): (f32, i16)) -> [(i16, i16); 3] {
        let mut out = [(-1, 0); 3];
        for (j, slot) in out.iter_mut().enumerate() {
            let (amount, attribute, used) = self.overrides[j];
            let c = def.costs[j];
            let (attribute, base) = match used {
                0 => continue,
                -2 if c.paid != 0 => (c.attribute, c.amount),
                _ => (attribute, amount),
            };
            slot.0 = attribute;
            if attribute < 0 {
                continue;
            }
            let mut v = self.cost_adj[j].wrapping_mul(scale).wrapping_add(base);
            if 0.0 < modifier {
                v = (f32::from(v) * modifier) as i32 as i16;
            }
            slot.1 = v;
        }
        for slot in &mut out {
            if slot.1 < 1 {
                slot.0 = -1;
            }
        }
        out
    }

    /// A cost entry as the research test and the payment read it: the attribute (the
    /// override's where it is in use, 1), the amount and how it is paid, when the entry is paid
    /// at all (its attribute at least 0, its `.dat` entry paid or an override other than 0); the
    /// modifier above 0 scales the amount (truncated).
    fn payment(
        &self,
        def: &TechDef,
        j: usize,
        (modifier, scale): (f32, i16),
    ) -> Option<(i16, f32, u8)> {
        let (amount, attribute, used) = self.overrides[j];
        let c = def.costs[j];
        let attribute = if used == 1 { attribute } else { c.attribute };
        if attribute < 0 || (c.paid == 0 && used == 0) {
            return None;
        }
        let mut v = if used == 1 {
            f32::from(amount)
        } else {
            (i32::from(self.cost_adj[j]) * i32::from(scale)) as f32 + f32::from(c.amount)
        };
        if 0.0 < modifier {
            v = (v * modifier) as i32 as i16 as f32;
        }
        Some((attribute, v, c.paid))
    }

    /// The research test (`FUN_1413cd600` without a building's holdings, `param_4` and
    /// `param_5` 0): a technology not in state 1 passes only with `repeat` (`param_6`), a state
    /// other than −1 and 0 and multiple researches allowed, or of the shared kind; then each
    /// paid cost needs the player's attribute to hold at least its amount. The game's
    /// per-technology overrides (`FUN_140f39f40` on the game's `+0x18`) are not held: none
    /// (probed for Loom, 22).
    pub fn can_research(
        &self,
        def: &TechDef,
        factors: (f32, i16),
        repeat: bool,
        attribute: &dyn Fn(i16) -> Result<f32, Unsupported>,
    ) -> Result<bool, Unsupported> {
        let shared = def.kind == SHARED_KIND;
        if self.state != 1
            && (!repeat || self.state == -1 || self.state == 0 || !self.allow_multiple)
            && !shared
        {
            return Ok(false);
        }
        for j in 0..3 {
            let Some((attr, amount, paid)) = self.payment(def, j, factors) else {
                continue;
            };
            if paid == 2 {
                return Err(Unsupported(
                    "a research cost the building holds (FUN_141204310)",
                ));
            }
            if attribute(attr)? < amount {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// The payment (`FUN_1413d4370` with `param_3` 1 and no location test, `param_5` −1): the
    /// research test first (false: nothing paid); then each paid cost comes off its attribute,
    /// or, beyond what the attribute holds, the rest off gold and the attribute to 0. The
    /// technology takes state 4 and the building's master `building`. The game's statistics
    /// (`FUN_14097daf0`) are not held; a cost the building pays (`FUN_1411c7470`) and a
    /// technology of the shared kind (`FUN_1411c49a0`) are not modelled.
    pub fn pay(
        &mut self,
        def: &TechDef,
        factors: (f32, i16),
        repeat: bool,
        building: i16,
        attributes: &mut dyn Attributes,
    ) -> Result<bool, Unsupported> {
        if !self.can_research(def, factors, repeat, &|i| attributes.get(i))? {
            return Ok(false);
        }
        for j in 0..3 {
            let Some((attr, amount, paid)) = self.payment(def, j, factors) else {
                continue;
            };
            if paid == 2 {
                return Err(Unsupported(
                    "a research cost the building pays (FUN_1411c7470)",
                ));
            }
            let have = attributes.get(attr)?;
            if amount <= have {
                attributes.set(attr, have - amount)?;
            } else {
                let gold = attributes.get(GOLD)?;
                attributes.set(GOLD, gold - (amount - have))?;
                attributes.set(attr, 0.0)?;
            }
        }
        if def.kind == SHARED_KIND {
            return Err(Unsupported("a research of the shared kind (FUN_1411c49a0)"));
        }
        self.state = 4;
        self.building_master = building;
        Ok(true)
    }

    /// `FUN_1413d1050`: the technology may be queued again (not of the shared kind, multiple
    /// researches allowed, and its researches queued and done below its cap with `stack`, or
    /// below 0x100 under the player's `+0x648`, `many`).
    pub fn repeatable(&self, def: &TechDef, many: bool) -> bool {
        if def.kind == SHARED_KIND || !self.allow_multiple {
            return false;
        }
        let done = i32::from(self.pending) + i32::from(self.times_researched);
        let capped = if (self.research_cap as u16) > 0x7fff {
            done < 0x100
        } else {
            done < i32::from(self.research_cap)
        };
        (self.stack && capped) || (many && done < 0x100)
    }
}

/// The player's attributes as a technology's costs read and write them.
pub trait Attributes {
    fn get(&self, i: i16) -> Result<f32, Unsupported>;
    fn set(&mut self, i: i16, v: f32) -> Result<(), Unsupported>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Loom (22) as player 2 of MarvinBlue/426048731 holds it before its RESEARCH in 3842.
    fn loom() -> (TechState, TechDef) {
        let state = TechState {
            state: 1,
            allow_multiple: true,
            overrides: [(-1, -1, -2); 3],
            research_cap: -1,
            building_master: -1,
            locations: vec![Location {
                building: 109,
                time: 25,
                button: 6,
                adjustment: 0.0,
            }],
            ..TechState::default()
        };
        let none = Cost {
            attribute: -1,
            amount: 0,
            paid: 0,
        };
        let gold = Cost {
            attribute: 3,
            amount: 50,
            paid: 1,
        };
        let def = TechDef {
            costs: [gold, none, none],
            ..TechDef::default()
        };
        (state, def)
    }

    struct Held([f32; 4]);

    impl Attributes for Held {
        fn get(&self, i: i16) -> Result<f32, Unsupported> {
            Ok(self.0[i as usize])
        }
        fn set(&mut self, i: i16, v: f32) -> Result<(), Unsupported> {
            self.0[i as usize] = v;
            Ok(())
        }
    }

    #[test]
    fn loom_lists_its_gold_and_pays_it_once_queued() {
        let (mut s, d) = loom();
        assert_eq!(s.listed_costs(&d, (0.0, 1)), [(3, 50), (-1, 0), (-1, 0)]);
        assert!(!s.repeatable(&d, false));
        // the game's attributes in 3842 (80 food, 157 wood, 200 stone, 100 gold) and after it
        let mut held = Held([79.956116, 157.05705, 200.0, 100.0]);
        assert_eq!(s.pay(&d, (0.0, 1), false, 109, &mut held), Ok(true));
        assert_eq!(held.0, [79.956116, 157.05705, 200.0, 50.0]);
        assert_eq!((s.state, s.building_master), (4, 109));
        // queued, it may not be researched again
        assert_eq!(s.pay(&d, (0.0, 1), false, 109, &mut held), Ok(false));
        // its queue entry records the 50 gold, which its start finds paid
        let paid = s.entry_costs(&d, (0.0, 1));
        assert_eq!(paid, [(3, 50.0), (-1, 0.0), (-1, 0.0)]);
        let get = |i: i16| held.get(i);
        assert_eq!(s.start_test(&d, (0.0, 1), &paid, &get), Ok(true));
        // without the entry's payment the 50 gold left would do too, but that difference is
        // the game's FUN_1411c68b0
        assert!(s.start_test(&d, (0.0, 1), &[(-1, 0.0); 3], &get).is_err());
        assert_eq!(s.location(109).map(|l| l.time), Some(25));
    }

    #[test]
    fn a_cost_beyond_the_attribute_takes_the_rest_from_gold() {
        let (mut s, mut d) = loom();
        d.costs[0].attribute = 1;
        let mut held = Held([0.0, 30.0, 0.0, 100.0]);
        // the test asks for 50 wood: 30 is too little
        assert_eq!(s.pay(&d, (0.0, 1), false, 109, &mut held), Ok(false));
        // the modifier scales the amount (truncated) before the test
        assert_eq!(s.pay(&d, (0.5, 1), false, 109, &mut held), Ok(true));
        assert_eq!(held.0, [0.0, 5.0, 0.0, 100.0]);
        assert_eq!(s.listed_costs(&d, (0.5, 1)), [(1, 25), (-1, 0), (-1, 0)]);
    }
}

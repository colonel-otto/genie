//! A technology's change (kind 8): its state, a forced research and its research time.

use crate::GenieError;
use crate::effect::{ONE, RESEARCH_SPENT, RESEARCHED, ROUND, TECHS_RESEARCHED};
use crate::turn::Turns;

/// The modes of `FUN_1415c3f20`'s technology change (kind 8) that change a cost, with the
/// resource and the cost modifier's mode (`DAT_142ed71b0`'s map: 0 to 3 set food, wood, stone,
/// gold; 0x4000 to 0x4003 add to them; 0xd to 0x10 multiply them, 0x11 every one).
pub(super) const fn tech_cost_mode(mode: i16) -> Option<(i16, u8)> {
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
pub(super) const fn tech_time_mode(mode: i16) -> Option<u8> {
    match mode {
        -1 => Some(0),
        -2 => Some(1),
        -3 => Some(2),
        _ => None,
    }
}

/// 1.0, 2.0 and 3.0 (`DAT_14330eb8c`, `DAT_142e8c150`, `DAT_142eb1c98`): a state change's values
/// (kind 8's mode 12): 0 disables, 1 makes it unavailable, 2 available and enabled, 3 researched.
pub(super) const STATE_UNAVAILABLE: f32 = crate::image::effect::STATE_UNAVAILABLE;
pub(super) const STATE_AVAILABLE: f32 = crate::image::effect::STATE_AVAILABLE;
pub(super) const STATE_RESEARCHED: f32 = crate::image::effect::STATE_RESEARCHED;
/// −1.0 (`DAT_14330ec20`): a research time without a `.dat` location, and a set's "none".
pub(super) const NO_TIME: f32 = crate::image::effect::NO_TIME;

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
    pub(super) fn modify_tech(
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
    pub(super) fn force_research(&mut self, p: usize, t: usize) -> Result<bool, GenieError> {
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
    pub(super) fn research_time(
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

//! What happens to an object once it is dead: the state steps of `RGE_Static_Object::update`
//! (158041 `0x14118ad50`) and `RGE_Animated_Object::update` (`0x141071880`), up to the update that
//! returns 1 and so takes the object off its player's list (`RGE_Object_List::update`,
//! `0x141166550`).
//!
//! The states are the save's `object_state`: 2 alive, 3 dying, 5 dead, 7 its resource going back,
//! 8 gone. Only the path of an object without a death sprite, a death spawn or an undead state is
//! modelled; anything else is [`Unsupported`].

use crate::unit_ai::Unsupported;

/// What the dead-object steps read from an object's master that the header save does not hold;
/// they come from the `.dat`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MasterData {
    /// Master `+0x620`: how much of the held attribute a dead object loses per second.
    pub decay: f32,
    /// Master `+0x490`: a dead object turns undead (state 6) instead of leaving.
    pub undead: bool,
}

/// Which update chain the object runs. `RGE_Animated_Object::update` and everything derived from
/// it (types 20 and up) adds a step after the static update's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Class {
    /// Type 10, `RGE_Static_Object`.
    #[default]
    Static,
    /// Types 20 and up.
    Animated,
}

/// The fields the dead-object steps read and write, with their offsets in 158041.
#[derive(Clone, Debug, PartialEq)]
pub struct Lifecycle {
    pub class: Class,
    /// `+0x640`, the save's `object_state`.
    pub state: u8,
    /// The masked hit points (`+0x278`).
    pub hp: f32,
    /// `+0x308`, −1 for none.
    pub attribute_type: i16,
    /// `+0x310`.
    pub amount: f32,
    /// The master's death sprite: an animated object with one plays it in state 4.
    pub dying_sprite: bool,
    /// The master spawns an object when this one dies.
    pub death_spawn: bool,
    /// `+0x2f0`: the object is inside another, whose update decides for it.
    pub inside: bool,
    /// `+0x643`: the next update moves the object to the sleeping list.
    pub goto_sleep: bool,
    /// The master's id: the static update has a special case for 61.
    pub master_id: i16,
    pub master: Option<MasterData>,
}

impl Default for Lifecycle {
    /// A live object that holds nothing.
    fn default() -> Self {
        Lifecycle {
            class: Class::Static,
            state: 2,
            hp: 1.0,
            attribute_type: -1,
            amount: 0.0,
            dying_sprite: false,
            death_spawn: false,
            inside: false,
            goto_sleep: false,
            master_id: -1,
            master: None,
        }
    }
}

/// `world+0x164`, the tick's dt in seconds: `cvtsi2ss` then `divss` by 1000 in f32.
pub fn seconds(dt: u32) -> f32 {
    dt as f32 / 1000.0
}

/// The resource a held attribute returns to its player (`FUN_140fdd470`), if any.
fn resource(attribute_type: i16) -> Option<i16> {
    match attribute_type {
        0..=3 | 9 | 56 => Some(attribute_type),
        15..=17 => Some(0),
        _ => None,
    }
}

impl Lifecycle {
    /// Whether the unit AI still runs: slot 4 (`0x140c66b10`) returns at once above state 2.
    pub fn is_alive(&self) -> bool {
        self.state <= 2
    }

    /// One update of `dt_seconds` ([`seconds`]). True when the object leaves its list.
    pub fn update(&mut self, dt_seconds: f32) -> Result<bool, Unsupported> {
        if self.goto_sleep {
            return Err(Unsupported("the object goes to sleep"));
        }
        match self.state {
            0 | 2 => {
                if self.hp < 1.0 {
                    self.die()?;
                }
                Ok(false)
            }
            3 | 5 => {
                self.dead(dt_seconds)?;
                Ok(false)
            }
            7 => {
                if resource(self.attribute_type).is_some() {
                    return Err(Unsupported(
                        "a dead object's resource goes back to its player",
                    ));
                }
                self.state = 8;
                Ok(false)
            }
            8 => Ok(true),
            _ => Err(Unsupported("object state not modelled")),
        }
    }

    /// The static update's death (state 3), then the animated update's step to state 5.
    fn die(&mut self) -> Result<(), Unsupported> {
        if self.inside {
            return Err(Unsupported("an object inside another dies"));
        }
        if self.death_spawn {
            return Err(Unsupported("a death spawn"));
        }
        if self.master_id == 61 {
            return Err(Unsupported("the static update's case for master 61"));
        }
        self.state = 3;
        if self.class == Class::Animated {
            if self.dying_sprite {
                return Err(Unsupported("a dying sprite (state 4)"));
            }
            self.state = 5;
        }
        Ok(())
    }

    /// States 3 and 5: once nothing is held the object moves on to state 7; until then the held
    /// amount decays, never below 0.
    fn dead(&mut self, dt_seconds: f32) -> Result<(), Unsupported> {
        let master = self
            .master
            .ok_or(Unsupported("no master data for a dead object"))?;
        if self.attribute_type == -1 || self.amount <= 0.0 {
            if master.undead {
                return Err(Unsupported("an undead object (state 6)"));
            }
            self.state = 7;
        } else if master.decay > 0.0 {
            self.amount -= master.decay * dt_seconds;
            if self.amount < 0.0 {
                self.amount = 0.0;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The states after each update until the object leaves, and the update (1-based) it leaves in.
    fn run(mut life: Lifecycle, dt: u32, limit: usize) -> (Vec<u8>, Option<usize>) {
        let mut states = Vec::new();
        for i in 1..=limit {
            if life.update(seconds(dt)).unwrap() {
                return (states, Some(i));
            }
            states.push(life.state);
        }
        (states, None)
    }

    fn dead(class: Class) -> Lifecycle {
        Lifecycle {
            class,
            hp: 0.0,
            master: Some(MasterData::default()),
            ..Lifecycle::default()
        }
    }

    #[test]
    fn an_object_with_hit_points_stays_alive() {
        let (states, left) = run(Lifecycle::default(), 104, 20);
        assert_eq!((states, left), (vec![2; 20], None));
    }

    #[test]
    fn a_dead_static_object_holds_its_state_while_its_amount_decays_then_leaves() {
        // FLARE3 (MarvinBlue/426048731 object 10025): 1.0 of attribute 12, decay 1.0 per second;
        // the game's states after updates 1 to 13, then it is gone in update 14
        let flare = Lifecycle {
            attribute_type: 12,
            amount: 1.0,
            master: Some(MasterData {
                decay: 1.0,
                undead: false,
            }),
            ..dead(Class::Static)
        };
        let (states, left) = run(flare, 104, 20);
        let mut expected = vec![3; 11];
        expected.extend([7, 8]);
        assert_eq!((states, left), (expected, Some(14)));
    }

    #[test]
    fn a_dead_animated_object_without_a_death_sprite_leaves_in_its_fourth_update() {
        // PLACEHOLDER2 and the empty town-centre annex of the same recording
        let (states, left) = run(dead(Class::Animated), 104, 20);
        assert_eq!((states, left), (vec![5, 7, 8], Some(4)));
    }

    #[test]
    fn the_decayed_amount_never_drops_below_zero() {
        let mut life = Lifecycle {
            state: 3,
            attribute_type: 12,
            amount: 0.05,
            master: Some(MasterData {
                decay: 1.0,
                undead: false,
            }),
            ..dead(Class::Static)
        };
        life.update(seconds(104)).unwrap();
        assert_eq!((life.state, life.amount), (3, 0.0));
    }

    #[test]
    fn the_paths_not_modelled_are_unsupported() {
        let cases = [
            Lifecycle {
                dying_sprite: true,
                ..dead(Class::Animated)
            },
            Lifecycle {
                death_spawn: true,
                ..dead(Class::Static)
            },
            Lifecycle {
                inside: true,
                ..dead(Class::Static)
            },
            Lifecycle {
                master_id: 61,
                ..dead(Class::Static)
            },
            Lifecycle {
                goto_sleep: true,
                ..Lifecycle::default()
            },
            Lifecycle {
                state: 5,
                master: Some(MasterData {
                    decay: 0.0,
                    undead: true,
                }),
                ..dead(Class::Animated)
            },
            Lifecycle {
                state: 5,
                master: None,
                ..dead(Class::Animated)
            },
            Lifecycle {
                state: 7,
                attribute_type: 0,
                ..dead(Class::Animated)
            },
            Lifecycle {
                state: 4,
                ..dead(Class::Animated)
            },
        ];
        for (i, mut life) in cases.into_iter().enumerate() {
            assert!(life.update(0.104).is_err(), "case {i}");
        }
    }

    #[test]
    fn the_unit_ai_runs_up_to_state_2() {
        let mut life = dead(Class::Animated);
        assert!(life.is_alive());
        life.update(0.104).unwrap();
        assert!(!life.is_alive());
    }
}

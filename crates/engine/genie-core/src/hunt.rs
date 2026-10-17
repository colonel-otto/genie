//! The Hunt action (`TRIBE_Action_Hunt`, type 110, vtable `0x142ed70f8`) up to the walk to its
//! target: a villager sent to a tree (or an animal) with a gather task attacks it first (state 12,
//! an [`Attack`] sub-action); a target that is dead already, the attack's end and the gathering
//! after it are not modelled yet.
//!
//! 158041: constructor `FUN_140ac2dd0`, start (slot 5) `0x140ac49b0`, update (slot 6)
//! `0x140ac7a00`, set_state (slot 16) `0x140ac60e0`.

use crate::attack::{Arms, Attack, Engagement, Strike};
use crate::move_to::Walk;
use crate::movement::Mover;
use crate::unit_ai::Unsupported;

/// The Hunt action's type, which word 9 adds.
pub const HUNT: i16 = 110;

/// The states the slice models: 12, attacking the target (`0xc`).
pub const ATTACK: u8 = 12;

/// A building's master type (`'P'`).
const BUILDING: u8 = 80;
/// Classes the hunt treats apart: livestock (58, `-0xe72`) and 49 (`-0xe7b`).
const LIVESTOCK: i16 = 58;
const CLASS_49: i16 = 49;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HuntAction {
    /// The target object (`+0x20`, its id at `+0x30`).
    pub target: i32,
    /// The action state (`+0x18`), which word 9 adds.
    pub state: u8,
    /// `+0x38`..`+0x40`: where the target stood when the hunt took it.
    pub point: [f32; 3],
    /// `+0x68`: the target's class.
    pub target_class: i16,
    pub task: HuntTask,
    /// The sub-action of state 12.
    pub attack: Option<Attack>,
}

/// What the hunt reads of its task (`+0x48`): its id in the master's list (`+0xa`; the work command
/// keeps a hunt on the task it finds), the work range (`+0x30`) and the resource it gathers
/// (`attribute_type`, `+0x20`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HuntTask {
    pub id: i16,
    pub range: f32,
    pub resource: i16,
    /// Its moving, proceeding, working and carrying sprites (`+0x40`..`+0x58`).
    pub sprites: [i16; 4],
}

/// What the hunt reads of its target when it starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quarry {
    pub id: i32,
    pub position: [f32; 3],
    /// Its master's class and type, its state (`+0x640`) and owner.
    pub class: i16,
    pub kind: u8,
    pub state: u8,
    pub player: usize,
    /// What it holds (`+0x310`).
    pub amount: f32,
    /// The hunter's player counts the target's player as its own or allied (`FUN_141663d90` with
    /// the ally flag).
    pub allied: bool,
}

/// Food and the food kinds of carcasses and fish (0, 15 to 17): a hunter keeps what it carries
/// of one for another.
fn food(kind: i16) -> bool {
    kind == 0 || (15..18).contains(&kind)
}

impl HuntAction {
    /// The factory's constructor (`FUN_140ac2dd0`; its target's add, slot 0x178, is the caller's)
    /// and the install's start (slot 5 with 1, `0x140ac49b0`): a hunter carrying another food
    /// keeps the amount as the task's kind (`FUN_14119d310` on `held`); then set_state(4). A
    /// target of another, unallied player (or of class 49) that is not dead yet is attacked first
    /// (set_state 12). `computer` is the owner's type test (`FUN_141659cf0`); `carry_rule` the
    /// game settings' test `FUN_141302150`, with which a hunter drops another resource it carries
    /// (a computer player's does when it carries none).
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        task: HuntTask,
        target: &Quarry,
        player: usize,
        computer: bool,
        carry_rule: bool,
        held: &mut (i16, f32),
        attack: &Engagement,
        arms: Arms,
        mover: &mut Mover,
    ) -> Result<Self, Unsupported> {
        let mut hunt = HuntAction {
            target: target.id,
            state: 0,
            point: target.position,
            target_class: target.class,
            task,
            attack: None,
        };
        if 0.0 < target.amount {
            let kind = task.resource;
            let both_food = food(kind) && food(held.0);
            let drops = computer && held.1 == 0.0;
            if held.0 != kind {
                if both_food {
                    held.0 = kind;
                } else if drops || carry_rule {
                    *held = (kind, 0.0);
                }
            }
        }
        if target.kind == BUILDING {
            return Err(Unsupported(
                "a hunt on a building (its annex and drop-site tests)",
            ));
        }
        if target.player == player {
            return Err(Unsupported(
                "a hunt on its own player's target (FUN_14109d4f0)",
            ));
        }
        hunt.chase(target, player, attack, arms, mover)?;
        Ok(hunt)
    }

    /// set_state(4) (`0x140ac60e0` case 4): the sub-actions go (an Attack's MoveTo stops the
    /// unit); a target not dead yet of another, unallied player (or of class 49) is attacked
    /// first: set_state(12) (case 0xc) with an Attack of the master's ranges.
    fn chase(
        &mut self,
        target: &Quarry,
        player: usize,
        attack: &Engagement,
        arms: Arms,
        mover: &mut Mover,
    ) -> Result<(), Unsupported> {
        self.tear_down(Some(mover))?;
        self.state = 4;
        self.point = target.position;
        if 2 < target.state || (target.class == LIVESTOCK && target.player == player) {
            return Err(Unsupported("a hunt on a dead target (its MoveTo)"));
        }
        if target.class != CLASS_49 && target.allied {
            return Err(Unsupported("a hunt on an allied target (set_state 13)"));
        }
        self.state = ATTACK;
        self.attack = Some(Attack::start(target.id, attack, arms, mover)?);
        Ok(())
    }

    /// Slot 9 (`0x140ac97c0`), the work command's new target for the hunt it keeps: true when the
    /// hunt takes it. Another target that is not its player's drop site (with a load to drop)
    /// becomes the hunt's (slot 0x68, `FUN_140ac7980`; the caller releases the old target and
    /// takes the new) and the hunt starts over at set_state(4). Its own target changes nothing.
    pub fn retarget(
        &mut self,
        target: &Quarry,
        player: usize,
        attack: &Engagement,
        arms: Arms,
        mover: &mut Mover,
    ) -> Result<bool, Unsupported> {
        if target.kind == BUILDING {
            return Err(Unsupported(
                "a hunt sent to a building (a farm or a drop site)",
            ));
        }
        if target.player == player {
            return Err(Unsupported(
                "a hunt sent to its own player's object (FUN_14109d4f0)",
            ));
        }
        if target.id == self.target {
            return Ok(false);
        }
        self.target = target.id;
        self.target_class = target.class;
        self.chase(target, player, attack, arms, mover)?;
        Ok(true)
    }

    /// The teardown (`FUN_140a73fc0`): the sub-actions go; the caller releases the target (slot
    /// 0x180).
    pub fn tear_down(&mut self, mover: Option<&mut Mover>) -> Result<(), Unsupported> {
        if let Some(mut a) = self.attack.take() {
            a.tear_down(mover)?;
        }
        Ok(())
    }

    /// One update (slot 6, `0x140ac7a00`) in state 12: a human player's hunter updates its Attack
    /// (`FUN_140a992b0`); the attack still under way (code 0) keeps the state. A computer
    /// player's hunt and the attack's end are not modelled.
    pub fn update(
        &mut self,
        walk: Walk<'_>,
        computer: bool,
        strike: &mut Strike<'_>,
    ) -> Result<(), Unsupported> {
        if self.state != ATTACK {
            return Err(Unsupported("a Hunt action in a state other than 12"));
        }
        if computer {
            return Err(Unsupported("a computer player's hunt (FUN_140fde4f0)"));
        }
        let attack = self
            .attack
            .as_mut()
            .ok_or(Unsupported("a Hunt in state 12 without its Attack"))?;
        match attack.update(walk, strike)? {
            0 | 6.. => Ok(()),
            5 => Err(Unsupported(
                "a Hunt whose Attack ended on its dead target (FUN_140ac44b0, set_state 4)",
            )),
            _ => Err(Unsupported("a Hunt whose Attack ended")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movement::{MoveStats, Velocity};

    fn walker() -> Mover {
        Mover {
            forward: [-1.0, 0.0, 0.0],
            right: [0.0, 0.0, 1.0],
            velocity: Some(Velocity {
                velocity: [0.0, 0.0, 0.8],
                acceleration: [0.0, 0.0, 1.0],
            }),
            paths: vec![],
            factor: 1.0,
            stats: MoveStats {
                speed: 0.8,
                turn_rate: f32::MAX,
                turn_radius: 0.0,
                turn_radius_speed: f32::MAX,
                radius: [0.2, 0.2, 0.5],
                restriction: 7,
                restriction_mask: 6,
                turns_at_once: true,
                move_algorithm: 0,
                min_collision: 1.0,
            },
            waiting: 1,
            wait_delays: 0,
            attack_moving: false,
            last_move: 0,
            substitute_count: 0,
        }
    }

    /// MarvinBlue/426048731's tree 9365 (Gaia, class 15, 100 wood) as 9181 meets it in update 748.
    fn tree() -> Quarry {
        Quarry {
            id: 9365,
            position: [24.5, 97.5, 0.0],
            class: 15,
            kind: 10,
            state: 2,
            player: 0,
            amount: 100.0,
            allied: false,
        }
    }

    fn engagement() -> Engagement {
        Engagement {
            gap_squared: 3.726 * 3.726,
            gap: 3.726,
            target_position: [24.5, 97.5, 0.0],
            speed: 0.8,
            range: 0.0,
            min_range: 0.0,
            can_move: true,
            stance: 0,
        }
    }

    const ARMS: Arms = Arms {
        sprite: None,
        second_sprite: None,
        idle_sprite: None,
        missile: -1,
        fire_frame: 0,
        interrupt_frame: -1,
    };

    const CHOP: HuntTask = HuntTask {
        id: 0,
        range: 0.0,
        resource: 15,
        sprites: [-1, 1239, 1961, 1960],
    };

    #[test]
    fn a_villager_sent_to_a_tree_attacks_it_first_and_walks_there() {
        let mut mover = walker();
        let mut held = (-1, 0.0);
        let hunt = HuntAction::start(
            CHOP,
            &tree(),
            2,
            false,
            false,
            &mut held,
            &engagement(),
            ARMS,
            &mut mover,
        )
        .unwrap();
        assert_eq!(hunt.state, ATTACK);
        let attack = hunt.attack.unwrap();
        assert_eq!(
            (attack.state, attack.move_to),
            (4, Some(crate::move_to::MoveTo::Start))
        );
        // the first MoveTo's teardown stopped the unit
        let v = mover.velocity.unwrap();
        assert_eq!(
            (v.velocity[2], v.acceleration[2], mover.waiting),
            (0.0, 0.0, 0)
        );
        // nothing carried, a human player and settings that keep it: the held kind stays
        assert_eq!(held, (-1, 0.0));
    }

    #[test]
    fn a_hunter_keeps_another_food_it_carries_and_drops_the_rest_by_the_settings() {
        let start = |held: (i16, f32), carry_rule: bool| {
            let mut held = held;
            HuntAction::start(
                CHOP,
                &tree(),
                2,
                false,
                carry_rule,
                &mut held,
                &engagement(),
                ARMS,
                &mut walker(),
            )
            .unwrap();
            held
        };
        assert_eq!(start((0, 7.0), false), (15, 7.0));
        assert_eq!(start((2, 7.0), false), (2, 7.0));
        assert_eq!(start((2, 7.0), true), (15, 0.0));
    }

    #[test]
    fn a_hunt_on_an_ally_or_a_dead_target_is_not_modelled() {
        let ally = Quarry {
            allied: true,
            ..tree()
        };
        let dead = Quarry { state: 3, ..tree() };
        for q in [ally, dead] {
            let mut held = (-1, 0.0);
            let r = HuntAction::start(
                CHOP,
                &q,
                2,
                false,
                false,
                &mut held,
                &engagement(),
                ARMS,
                &mut walker(),
            );
            assert!(r.is_err());
        }
    }
}

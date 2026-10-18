//! The Hunt action (`TRIBE_Action_Hunt`, type 110, vtable `0x142ed70f8`): a villager sent to a
//! tree (or an animal) with a gather task attacks it first (state 12, an [`Attack`] sub-action);
//! once it is dead the villager walks to it (state 4, a MoveTo sub-action), faces it (6) and
//! gathers (7). The drop-off (8 on) is not modelled yet.
//!
//! 158041: constructor `FUN_140ac2dd0`, start (slot 5) `0x140ac49b0`, update (slot 6)
//! `0x140ac7a00`, set_state (slot 16) `0x140ac60e0`.

use crate::attack::{Arms, Attack, Engagement, Strike, set_sprite};
use crate::damage::Victim;
use crate::move_to::{
    Goal, MoveTo, MoveWait, Walk, start_walk, target_moved, walk_on, walk_or_give_up,
};
use crate::movement::{Mover, footprint_gap};
use crate::sprite::Graphic;
use crate::unit_ai::Unsupported;

/// The Hunt action's type, which word 9 adds.
pub const HUNT: i16 = 110;

/// The states the slice models: 4 walking to a dead target, 6 facing it, 7 gathering from it, 8
/// taking the load to its drop site, 12 attacking it (`0xc`).
pub const WALK: u8 = 4;
pub const FACE: u8 = 6;
pub const GATHER: u8 = 7;
pub const DROP: u8 = 8;
pub const ATTACK: u8 = 12;
/// State 13: the hunt failed; its next update tells its unit AI (notify 0x1f9) and ends it.
pub const FAILED: u8 = 13;
/// A building whose top action is this (a farm at work) takes a hunt sent to it to set_state 15.
const FARM_AT_WORK: i16 = 0x7d;

/// A MoveTo starts in place within this gap of its target (`0x140aa2100`).
const ARRIVED: f32 = 0.01;
/// Own livestock farther than this squared from its drop site is led there first
/// (`DAT_142ecccc4`).
const HERD_REACH: f32 = crate::image::hunt::HERD_REACH;
/// A face on dead livestock beyond the task's range plus this walks again (`DAT_142eb5690`).
const GAP: f32 = crate::image::hunt::GAP;
/// Below this a target's or a load's remainder counts as nothing (`DAT_142eb5690`).
const REMAINDER: f32 = crate::image::hunt::REMAINDER;
/// A target's last bit adds its master's first storage amount times this (`DAT_142eb1e9c`).
const LAST_BIT: f32 = crate::image::hunt::LAST_BIT;
/// A task whose `attribute_type4` is this (`0x124`) stops its target's decay under player
/// attribute 292 (`FUN_140ac44b0`).
const DECAY_TASK: i16 = 0x124;

/// A building's master type (`'P'`).
const BUILDING: u8 = 80;
/// The range of the walk to the hunter's own player's livestock (`DAT_142eb4838`).
const OWN_LIVESTOCK_RANGE: f32 = crate::image::hunt::OWN_LIVESTOCK_RANGE;
/// The villagers' class, whose MoveTo waits once when the motion step gives up.
const VILLAGER_CLASS: i16 = 4;
/// Classes the hunt treats apart: livestock (58, `-0xe72`) and 49 (`-0xe7b`).
pub const LIVESTOCK: i16 = 58;
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
    /// The sub-action of state 4: a MoveTo to the target within `walk_range`, quiet (its
    /// `+0x60` and `+0x61` set: its end changes neither the sprite nor the unit's notifies).
    pub move_to: Option<MoveTo>,
    /// The MoveTo's range: the task's (`+0x30`) to a dead target, `OWN_LIVESTOCK_RANGE` to its
    /// own player's livestock.
    pub walk_range: f32,
    /// `+0x70`: the drop site its own livestock is led to, −1 for none yet.
    pub drop_site: i32,
    /// `+0x28` (its id at `+0x34`), the second target: the drop site the load goes to in state 8,
    /// −1 for none.
    pub carry_to: i32,
    /// `+0x44`: the state's timer (state 8 waits with it below 0 before it tries again).
    pub timer: f32,
    /// Its MoveTo's wait after the motion step gave up.
    pub move_wait: MoveWait,
}

/// What the drop-off reads of its drop site on arrival: that the site takes what the unit carries
/// (`FUN_140fdd190`) and the gap between the two footprints squared (`FUN_14118f8c0`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DropOff {
    pub takes: bool,
    pub gap_squared: f32,
}

/// What set_state(6) reads of a live livestock target (`0x140ac60e0` case 6).
pub struct Herd {
    /// The target's player is the hunter's.
    pub own: bool,
    /// For its own livestock, the drop site (the hunt's, else the nearest that takes livestock,
    /// `FUN_1411671e0`) and the gap between it and the target squared (`FUN_14118f8c0`).
    pub drop_site: Option<(i32, f32)>,
    /// The Attack set_state(12) starts.
    pub engagement: Engagement,
    pub arms: Arms,
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
    /// `+0x28`, `work_val1`: what one work gathers per second and work rate.
    pub work_value: f32,
    /// `+0x22`, `attribute_type2`: the player's attribute that scales the gathering, -1 none.
    pub productivity: i16,
    /// `+0x26`, `attribute_type4`.
    pub kind4: i16,
}

/// The hunt's sprites as graphics: the task's moving, working and carrying sprites, and the
/// master's standing sprite, which slot 0x68 takes for none.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HuntSprites {
    pub moving: Option<Graphic>,
    pub working: Option<Graphic>,
    pub carrying: Option<Graphic>,
    pub standing: Option<Graphic>,
}

/// What the gathering reads of its unit and changes besides the attack's [`Strike`].
pub struct Gatherer<'a> {
    /// `+0x308`, `+0x310`: what the unit carries.
    pub held_kind: &'a mut i16,
    pub held: &'a mut f32,
    /// The master's work rate (`+0x798`) plus the unit's powerup (slot 0x6e8, `+0xa30`).
    pub work_rate: f32,
    /// The master's `attribute_max_amount` (`+0x61c`): what the unit carries at most.
    pub capacity: f32,
    /// The player's attribute the task's `attribute_type2` names, when above 0; else 1.
    pub productivity: f32,
    /// The target's master's class and its first storage amount (`+0x610`).
    pub target_class: i16,
    pub target_storage: f32,
    /// The player's attribute 292 is 1: a task whose `attribute_type4` is 0x124 stops the
    /// target's decay (`FUN_140ac44b0`).
    pub stops_decay: bool,
    /// The unit AI's target class (`+0x4c`, `FUN_140c4cc50`), None for a unit without one.
    pub ai_class: Option<i32>,
    pub sprites: HuntSprites,
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
    /// Its top action's type (a building's `+0x880` list).
    pub top_action: Option<i16>,
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
            move_to: None,
            walk_range: task.range,
            drop_site: -1,
            carry_to: -1,
            timer: 0.0,
            move_wait: MoveWait::default(),
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
        // an own target among the hunter's master's drop sites (`+0x7a0`, FUN_14109d4f0) takes a
        // load there (set_state 8); a hunter carrying nothing does not get there
        if target.player == player && 0.0 < held.1 {
            return Err(Unsupported(
                "a loaded hunt on its own player's object (FUN_14109d4f0, set_state 8)",
            ));
        }
        hunt.chase(target, player, attack, arms, mover)?;
        Ok(hunt)
    }

    /// set_state(4) (`0x140ac60e0` case 4): the sub-actions go (an Attack's MoveTo stops the
    /// unit), the timer clears and the target's place becomes the point. A dead target, or the hunter's own
    /// player's livestock, is walked to: a quiet MoveTo within the task's range, or
    /// `OWN_LIVESTOCK_RANGE` to the livestock (`FUN_140aa0df0`; its start `0x140aa2100`: a unit
    /// within 0.01 of the target's footprint is there already). A target not dead yet of another,
    /// unallied player (or of class 49) is attacked first: set_state(12) (case 0xc) with an
    /// Attack of the master's ranges. A live allied one fails the hunt (set_state 13).
    pub(crate) fn chase(
        &mut self,
        target: &Quarry,
        player: usize,
        attack: &Engagement,
        arms: Arms,
        mover: &mut Mover,
    ) -> Result<(), Unsupported> {
        self.tear_down(Some(mover))?;
        self.state = 4;
        self.timer = 0.0;
        self.point = target.position;
        let own_livestock = target.class == LIVESTOCK && target.player == player;
        if 2 < target.state || own_livestock {
            self.move_wait = MoveWait::default();
            self.walk_range = if 2 < target.state {
                self.task.range
            } else {
                OWN_LIVESTOCK_RANGE
            };
            self.move_to = Some(if ARRIVED < attack.gap {
                if mover.stats.speed == 0.0 {
                    return Err(Unsupported("a MoveTo of a unit without speed (state 13)"));
                }
                MoveTo::Start
            } else {
                MoveTo::Arrived
            });
            return Ok(());
        }
        if target.class != CLASS_49 && target.allied {
            // LAB_140ac679d: set_state(13): the timer clears; the unit's sprite (slot 0x68 with
            // FUN_140ac5210's) is the caller's
            self.state = FAILED;
            self.timer = 0.0;
            return Ok(());
        }
        self.state = ATTACK;
        self.attack = Some(Attack::start(target.id, attack, arms, mover)?);
        Ok(())
    }

    /// Slot 9 (`0x140ac97c0`), the work command's new target for the hunt it keeps: true when the
    /// hunt takes it. A building at work as a farm (top action `0x7d`) takes it to set_state 15,
    /// not modelled. Its own player's drop site (one of its master's, `drops_here`,
    /// `FUN_14109d4f0`) while it carries something (`held`) takes the load there: the target goes
    /// (slot 0x68 with none; the caller releases it), the timer and the point clear, the drop site
    /// becomes the second target (`FUN_140a7a190`) with its class, and set_state(8). Another target
    /// becomes the hunt's (slot 0x68, `FUN_140ac7980`; the caller releases the old target and takes
    /// the new) and the hunt starts over at set_state(4). Its own target changes nothing.
    #[allow(clippy::too_many_arguments)]
    pub fn retarget(
        &mut self,
        target: &Quarry,
        player: usize,
        held: f32,
        drops_here: bool,
        attack: &Engagement,
        arms: Arms,
        mover: &mut Mover,
    ) -> Result<bool, Unsupported> {
        if target.kind == BUILDING && target.top_action == Some(FARM_AT_WORK) {
            return Err(Unsupported("a hunt sent to a farm at work (set_state 15)"));
        }
        if target.player == player && drops_here && 0.0 < held {
            self.target = -1;
            self.timer = 0.0;
            self.point = [-1.0; 3];
            self.carry_to = target.id;
            self.target_class = target.class;
            self.set_drop(attack.gap, mover)?;
            return Ok(true);
        }
        if target.id == self.target {
            return Ok(false);
        }
        self.target = target.id;
        self.target_class = target.class;
        self.carry_to = -1;
        self.chase(target, player, attack, arms, mover)?;
        Ok(true)
    }

    /// set_state(8) (`0x140ac60e0` case 8) with a load and its drop site (`carry_to`; without one
    /// the drop site search `FUN_1411671e0` would find it, not modelled): the sub-actions go and a
    /// quiet MoveTo to the drop site within the task's range starts (`FUN_140aa0df0`, as
    /// [`HuntAction::set_walk`]'s: within 0.01 of its footprint (`gap`) the unit is there already).
    pub(crate) fn set_drop(&mut self, gap: f32, mover: &mut Mover) -> Result<(), Unsupported> {
        self.tear_down(Some(mover))?;
        self.state = DROP;
        if self.carry_to == -1 {
            return Err(Unsupported("a drop-off without its drop site (its search)"));
        }
        self.walk_range = self.task.range;
        self.move_wait = MoveWait::default();
        self.move_to = Some(if ARRIVED < gap {
            if mover.stats.speed == 0.0 {
                return Err(Unsupported("a MoveTo of a unit without speed (state 13)"));
            }
            MoveTo::Start
        } else {
            MoveTo::Arrived
        });
        Ok(())
    }

    /// State 8's update (`0x140ac7a00` case 7): the MoveTo to the drop site walks (a MoveTo that
    /// cannot go on, codes 3 to 5, would wait 3 seconds, not modelled; the timer's wait neither).
    /// Once it ended (its update in state 1 returns 1: the list deletes it and its destructor stops
    /// the unit) the load goes to the player at a drop site that takes it within a tile
    /// (`site`; one that does not, set_state 2, or a farther one are not modelled): the resource
    /// of what the unit carries (`FUN_140fdd4f0`: food for 15 to 17, nothing outside 0 to 3, 9 and
    /// 56, then 0 beyond the player's attributes, `FUN_140efe5b0`), which the caller adds; the unit
    /// carries nothing after (`FUN_14119d310` with -1). Returns the resource and the amount. A
    /// hunt with a target walks back to it (set_state 4), one without searches for a new one
    /// (the unit AI's slots 0x1b0 and 0x80, `FUN_140c488b0`): neither is modelled, after the load.
    pub fn update_drop(
        &mut self,
        mut walk: Walk<'_>,
        held_kind: &mut i16,
        held: &mut f32,
        attributes: usize,
        site: Option<DropOff>,
    ) -> Result<Option<(i16, f32)>, Unsupported> {
        if self.timer < 0.0 {
            return Err(Unsupported("a drop-off waiting to try again (its timer)"));
        }
        match self.move_to {
            Some(MoveTo::Start) => {
                self.move_to = Some(MoveTo::Moving);
                let range = self.walk_range;
                walk.me.action_state = self.state;
                self.move_to = Some(start_walk(walk, Goal::Target(self.carry_to), range, true)?);
                return Ok(None);
            }
            Some(MoveTo::Moving) => {
                if target_moved(&walk, self.carry_to, true)? {
                    self.move_to = Some(MoveTo::Start);
                    return Ok(None);
                }
                walk.me.action_state = self.state;
                self.move_to = Some(walk_on(walk)?);
                return Ok(None);
            }
            Some(MoveTo::Arrived) => {
                self.move_to = None;
                walk.mover
                    .as_deref_mut()
                    .ok_or(Unsupported("a hunter without movement state"))?
                    .stop()?;
            }
            Some(MoveTo::Waiting | MoveTo::Failed) => {
                return Err(Unsupported(
                    "a drop-off's waiting or failed MoveTo (states 2, 13)",
                ));
            }
            None => return Err(Unsupported("a Hunt in state 8 without its MoveTo")),
        }
        let site = site.ok_or(Unsupported("a drop-off whose drop site is gone"))?;
        if !site.takes {
            return Err(Unsupported(
                "a drop site that does not take the load (set_state 2)",
            ));
        }
        if 1.0 < site.gap_squared {
            return Err(Unsupported(
                "a drop-off out of a tile of its drop site (its wait)",
            ));
        }
        let kind = match *held_kind {
            0..=3 | 9 | 56 => *held_kind,
            15..=17 => 0,
            _ => -1,
        };
        let kind = if (0..attributes as i16).contains(&kind) {
            kind
        } else {
            0
        };
        let amount = *held;
        *held_kind = -1;
        *held = 0.0;
        Ok(Some((kind, amount)))
    }

    /// The teardown (`FUN_140a73fc0`): the sub-actions go; the caller releases the target (slot
    /// 0x180).
    pub fn tear_down(&mut self, mut mover: Option<&mut Mover>) -> Result<(), Unsupported> {
        if let Some(mut a) = self.attack.take() {
            a.tear_down(mover.as_deref_mut())?;
        }
        if self.move_to.take().is_some()
            && let Some(m) = mover
        {
            m.stop()?;
        }
        Ok(())
    }

    /// One update (slot 6, `0x140ac7a00`): state 12 updates the Attack (`FUN_140a992b0`); its end
    /// on the dead target (code 5) runs `FUN_140ac44b0` and set_state(4). State 4 updates the
    /// MoveTo; its end (code 1) runs `FUN_140ac44b0` and set_state(6). State 6 faces the target
    /// (`FUN_141162db0`) and, while it holds something, set_state(7); state 7 gathers. A computer
    /// player's hunt and the other codes are not modelled.
    pub fn update(
        &mut self,
        mut walk: Walk<'_>,
        computer: bool,
        strike: &mut Strike<'_>,
        gather: &mut Gatherer<'_>,
        herd: Option<Herd>,
    ) -> Result<bool, Unsupported> {
        if computer {
            return Err(Unsupported("a computer player's hunt (FUN_140fde4f0)"));
        }
        let mut full = false;
        let r = match self.state {
            ATTACK => {
                let attack = self
                    .attack
                    .as_mut()
                    .ok_or(Unsupported("a Hunt in state 12 without its Attack"))?;
                match attack.update(walk.reborrow(), strike)? {
                    0 | 6.. => Ok(()),
                    5 => {
                        self.decay_check(gather, strike.victim)?;
                        self.set_walk(&mut walk, strike.victim)
                    }
                    _ => Err(Unsupported("a Hunt whose Attack ended")),
                }
            }
            // case 3: a hunt that gave up on its MoveTo waits (its timer below 0), then set_state 11
            WALK if self.timer < 0.0 => {
                self.timer += walk.dt;
                if self.timer < 0.0 {
                    return Ok(false);
                }
                self.timer = 0.0;
                Err(Unsupported("a hunt's set_state 11 after its wait"))
            }
            WALK => match self.move_to {
                Some(MoveTo::Start) => {
                    // case 3 after a wait: `+0x6c` goes from 1 to 2 as it asks again
                    if self.move_wait.state == 1 {
                        self.move_wait.state = 2;
                    }
                    self.move_to = Some(MoveTo::Moving);
                    let range = self.walk_range;
                    walk.me.action_state = self.state;
                    self.move_to = Some(start_walk(walk, Goal::Target(self.target), range, true)?);
                    Ok(())
                }
                Some(MoveTo::Moving) => {
                    if target_moved(&walk, self.target, true)? {
                        self.move_to = Some(MoveTo::Start);
                        return Ok(false);
                    }
                    walk.me.action_state = self.state;
                    match walk_or_give_up(walk)? {
                        Some(m) => self.move_to = Some(m),
                        None => self.give_up(strike)?,
                    }
                    Ok(())
                }
                // the failed MoveTo's update returns 3 (quiet: no notify): `FUN_140ac4670` with 8
                // and -1 looks for another target only for a class the unit AI's slot 0x1b0 gives
                // 0 (`FUN_140c1c4f0`: 15, trees; the class its unit AI targets, else the hunt's),
                // which is not modelled; otherwise the hunt waits 3 seconds and the unit's
                // animation stops (`FUN_141161850`)
                Some(MoveTo::Failed) => {
                    if let Some(class) = gather.ai_class {
                        let class = if class == -1 {
                            i32::from(self.target_class)
                        } else {
                            class
                        };
                        if class == 15 {
                            return Err(Unsupported(
                                "a hunt's search for another tree (FUN_140ac4670)",
                            ));
                        }
                    }
                    self.timer = -3.0;
                    *strike.anim_speed = Some(0.0);
                    Ok(())
                }
                // case 2: the timer runs by dt; at its end the MoveTo asks again (set_state 3)
                Some(MoveTo::Waiting) => {
                    self.move_wait.timer += walk.dt;
                    if self.move_wait.delay <= self.move_wait.timer {
                        self.move_wait.timer = 0.0;
                        self.move_to = Some(MoveTo::Start);
                    }
                    Ok(())
                }
                // its update in state 1 returns 1: the list deletes it, and its destructor
                // (`0x140aa1f30`) stops the unit (slot 0x5d0)
                Some(MoveTo::Arrived) => {
                    self.move_to = None;
                    walk.mover
                        .as_deref_mut()
                        .ok_or(Unsupported("a hunter without movement state"))?
                        .stop()?;
                    self.decay_check(gather, strike.victim)?;
                    self.set_face(walk, strike, gather, herd)
                }
                None => Err(Unsupported("a Hunt in state 4 without its MoveTo")),
            },
            FACE => {
                // case 5: FUN_141162db0 turns the unit to face its target; a unit without a turn
                // time turns at once, by the facets between (its angle `+0x7e8`, which the slice
                // does not hold)
                let turns_at_once = walk
                    .mover
                    .as_deref()
                    .ok_or(Unsupported("a hunter without movement state"))?
                    .stats
                    .turns_at_once;
                if !turns_at_once {
                    return Err(Unsupported(
                        "a hunter that turns over time to face its target",
                    ));
                }
                if 0.0 < strike.victim.amount {
                    self.set_gather(strike, gather);
                    Ok(())
                } else {
                    Err(Unsupported(
                        "a hunt's target empty when faced (set_state 3)",
                    ))
                }
            }
            GATHER => {
                full = self.gather(walk.dt, strike.victim, gather)?;
                Ok(())
            }
            _ => Err(Unsupported(
                "a Hunt action in a state the slice does not model",
            )),
        };
        r.map(|()| full)
    }

    /// `FUN_140ac44b0`: a task whose `attribute_type4` is 0x124, with its player's attribute 292
    /// at 1, stops its target's decay: a target whose master decays (`+0x620` above 0) gets a
    /// master of its own (`FUN_141186d10`) with the decay negated, which the slice does not
    /// model. Otherwise it does nothing.
    fn decay_check(&self, gather: &Gatherer<'_>, victim: &Victim) -> Result<(), Unsupported> {
        if self.task.kind4 == DECAY_TASK && gather.stops_decay && 0.0 < victim.decay {
            return Err(Unsupported(
                "a hunt that stops its target's decay (its own master, FUN_141186d10)",
            ));
        }
        Ok(())
    }

    /// The MoveTo's answer to the motion step giving up (it returns 1, `0x140aa2a70` in state 4): a
    /// human player's villager that has not waited yet waits (set_state 2, `+0x6c` 1) a second
    /// with a stance below 2 (or without a unit AI), else two, then asks again; any other MoveTo
    /// fails (set_state 13, quiet).
    fn give_up(&mut self, strike: &Strike<'_>) -> Result<(), Unsupported> {
        if self.move_wait.state != 0 || strike.own.class != VILLAGER_CLASS {
            self.move_to = Some(MoveTo::Failed);
            return Ok(());
        }
        self.move_wait.delay = if strike.stance < 2 { 1.0 } else { 2.0 };
        self.move_wait.state = 1;
        self.move_to = Some(MoveTo::Waiting);
        Ok(())
    }

    /// set_state(4) (`0x140ac60e0` case 4) after the target died: the sub-actions go, the target's
    /// place becomes the point, and a quiet MoveTo to the target within the task's range starts
    /// (`FUN_140aa0df0`, its start `0x140aa2100`: a unit within 0.01 of the target's footprint is
    /// there already, state 1; else state 3).
    fn set_walk(&mut self, walk: &mut Walk<'_>, victim: &Victim) -> Result<(), Unsupported> {
        self.tear_down(walk.mover.as_deref_mut())?;
        self.state = WALK;
        if victim.state <= 2 {
            return Err(Unsupported(
                "a hunt's set_state 4 on a live target after its attack",
            ));
        }
        let t = walk
            .world
            .get_known(self.target)?
            .ok_or(Unsupported("a hunt's target without a place"))?;
        self.point = t.position;
        self.walk_range = self.task.range;
        self.move_wait = MoveWait::default();
        let mover = walk
            .mover
            .as_deref()
            .ok_or(Unsupported("a hunter without movement state"))?;
        let ground = [mover.stats.radius[0], mover.stats.radius[1]];
        let gap = footprint_gap(walk.me.position, ground, t.position, t.radius);
        self.move_to = Some(if ARRIVED < gap {
            if mover.stats.speed == 0.0 {
                return Err(Unsupported("a MoveTo of a unit without speed (state 13)"));
            }
            MoveTo::Start
        } else {
            MoveTo::Arrived
        });
        Ok(())
    }

    /// set_state(6) (case 6): the hunt faces its target. Live livestock of its own player is led
    /// to its drop site first when it stands more than 3 tiles from it (`HERD_REACH` squared,
    /// set_state 10, not modelled); then live livestock is attacked (set_state 12, with an Attack
    /// of the master's ranges: [`Herd`]). Dead livestock out of the task's range plus `GAP` is
    /// walked to again (set_state 4, [`HuntAction::set_walk`]). Otherwise the unit takes the hunt's sprite
    /// (`FUN_140ac51d0`: the carrying one for a load of the task's resource of 1 or more, else the
    /// moving one; slot 0x68 takes the standing sprite for none). A target of class 49 (its
    /// owner's change, `FUN_141185360`) is not modelled.
    fn set_face(
        &mut self,
        walk: Walk<'_>,
        strike: &mut Strike<'_>,
        gather: &Gatherer<'_>,
        herd: Option<Herd>,
    ) -> Result<(), Unsupported> {
        if gather.target_class == CLASS_49 {
            return Err(Unsupported(
                "a hunt facing a target of class 49 (set_state 6)",
            ));
        }
        if gather.target_class == LIVESTOCK {
            if strike.victim.state <= 2 {
                let herd = herd.ok_or(Unsupported("a hunt facing live livestock unprepared"))?;
                if let (true, Some((site, gap_squared))) = (herd.own, herd.drop_site) {
                    self.drop_site = site;
                    if HERD_REACH < gap_squared {
                        return Err(Unsupported(
                            "a hunt that leads its livestock to a drop site (set_state 10)",
                        ));
                    }
                }
                let mover = walk
                    .mover
                    .ok_or(Unsupported("a hunter without movement state"))?;
                self.move_to = None;
                self.state = ATTACK;
                self.attack = Some(Attack::start(
                    self.target,
                    &herd.engagement,
                    herd.arms,
                    mover,
                )?);
                return Ok(());
            }
            let t = walk
                .world
                .get_known(self.target)?
                .ok_or(Unsupported("a hunt's target without a place"))?;
            let me = walk.me.position;
            let ground = walk
                .mover
                .as_deref()
                .map(|m| [m.stats.radius[0], m.stats.radius[1]])
                .ok_or(Unsupported("a hunter without movement state"))?;
            if self.task.range + GAP < footprint_gap(me, ground, t.position, t.radius) {
                let mut walk = walk;
                return self.set_walk(&mut walk, strike.victim);
            }
        }
        self.move_to = None;
        self.state = FACE;
        let carrying = self.task.resource != -1
            && *gather.held_kind == self.task.resource
            && 1 <= *gather.held as i32;
        let g = match (carrying, gather.sprites.carrying) {
            (true, Some(g)) => Some(g),
            _ => gather.sprites.moving,
        }
        .or(gather.sprites.standing);
        if let Some(g) = g {
            set_sprite(strike.sprite, g, strike.sprite_rng);
        }
        Ok(())
    }

    /// set_state(7) (case 7): a food task keeps a load of food (`FUN_14119d310` keeps the
    /// amount), any other load is dropped; the unit takes the working sprite (slot 0x68; the
    /// standing one for none); the gathering sound is not held.
    fn set_gather(&mut self, strike: &mut Strike<'_>, gather: &mut Gatherer<'_>) {
        self.state = GATHER;
        let kind = self.task.resource;
        let keeps = food(kind) && food(*gather.held_kind);
        if *gather.held_kind != kind {
            *gather.held = if keeps { *gather.held } else { 0.0 };
            *gather.held_kind = kind;
        }
        if let Some(g) = gather.sprites.working.or(gather.sprites.standing) {
            set_sprite(strike.sprite, g, strike.sprite_rng);
        }
    }

    /// State 7 (case 6): the work, `(work rate) × work_val1 × dt`, at most what the target holds,
    /// times the productivity, while the load stays within the capacity: the target loses the
    /// work, its last bit below 0.1 going with a thousandth of its master's first storage amount;
    /// the unit gains, a remainder below 0.1 of the capacity rounding the load up. A full load or
    /// an empty target ends the gathering: true asks the caller for set_state(8) (its drop site
    /// search, [`HuntAction::set_drop`]). The game's `+0xc14`
    /// (`FUN_140c884c0`), 0 as the tick oracle holds it, would take the whole target at once.
    fn gather(
        &mut self,
        dt: f32,
        victim: &mut Victim,
        g: &mut Gatherer<'_>,
    ) -> Result<bool, Unsupported> {
        let work = (g.work_rate * self.task.work_value) * dt;
        let take = if work <= victim.amount {
            work
        } else {
            victim.amount
        };
        let mut gain = g.productivity * take;
        let capacity = g.capacity;
        // `<=` in the game: a NaN goes to the drop-off too
        let room = capacity - *g.held;
        if !matches!(
            gain.partial_cmp(&room),
            Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
        ) {
            return Ok(true);
        }
        victim.amount -= take;
        if victim.amount < REMAINDER {
            gain = gain + g.target_storage * LAST_BIT + victim.amount;
            victim.amount = 0.0;
        }
        *g.held += gain;
        let over = (*g.held - capacity).abs();
        if *g.held < capacity && over < REMAINDER && over <= victim.amount {
            victim.amount -= over;
            *g.held += over;
        }
        if matches!(g.target_class, 9 | 10) && *g.held_kind == 3 && self.task.resource == 3 {
            return Err(Unsupported(
                "a gold load of class 9 or 10 (its player's attribute 3)",
            ));
        }
        Ok(!(REMAINDER <= over && 0.0 < victim.amount))
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
                fast_turn: false,
                walk_speed: 1.0,
                run_speed: 1.0,
            },
            waiting: 1,
            wait_delays: 0,
            attack_moving: false,
            last_move: 0,
            substitute_count: 0,
            ignore_units: Vec::new(),
            move_to_point: None,
            speed_stopped: false,
            pauses: Vec::new(),
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
            top_action: None,
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
        idle_attack_sprite: None,
        missile: -1,
        fire_frame: 0,
        interrupt_frame: -1,
    };

    const CHOP: HuntTask = HuntTask {
        id: 0,
        range: 0.0,
        resource: 15,
        sprites: [-1, 1239, 1961, 1960],
        work_value: 1.0,
        productivity: -1,
        kind4: -1,
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

    fn victim(amount: f32) -> Victim {
        Victim {
            id: 9357,
            amount,
            state: 3,
            kind: 10,
            class: 15,
            storage: 100.0,
            ..Victim::default()
        }
    }

    #[test]
    fn a_villager_gathers_work_rate_times_work_value_times_dt_until_its_load_is_full() {
        let mut hunt = HuntAction {
            target: 9357,
            state: GATHER,
            point: [24.5, 94.5, 0.0],
            target_class: 15,
            task: HuntTask {
                work_value: 0.39,
                resource: 1,
                ..CHOP
            },
            attack: None,
            move_to: None,
            walk_range: 0.0,
            drop_site: -1,
            carry_to: -1,
            timer: 0.0,
            move_wait: MoveWait::default(),
        };
        let (mut kind, mut held) = (1, 0.0f32);
        let mut gather = |held: &mut f32, kind: &mut i16, tree: &mut Victim| {
            let mut g = Gatherer {
                held_kind: kind,
                held,
                work_rate: 1.0,
                capacity: 10.0,
                productivity: 1.0,
                target_class: 15,
                target_storage: 100.0,
                stops_decay: false,
                ai_class: None,
                sprites: HuntSprites::default(),
            };
            hunt.gather(0.02, tree, &mut g)
        };
        let mut tree = victim(100.0);
        assert_eq!(gather(&mut held, &mut kind, &mut tree), Ok(false));
        let w = (1.0f32 * 0.39) * 0.02;
        assert_eq!((held, tree.amount), (w, 100.0 - w));
        // a load 0.1 short of the capacity rounds up, and the full load asks for set_state(8)
        held = 9.95 - w;
        let mut tree = victim(50.0);
        assert_eq!(gather(&mut held, &mut kind, &mut tree), Ok(true));
        assert_eq!(held, 10.0);
        // a tree's last bit goes with a thousandth of its master's storage, the empty target
        // asking for set_state(8)
        held = 0.0;
        let mut tree = victim(0.05);
        assert_eq!(gather(&mut held, &mut kind, &mut tree), Ok(true));
        assert_eq!((held, tree.amount), (0.05 + 100.0 * 0.001, 0.0));
    }

    /// A live allied target fails the hunt at once (set_state 13, its timer cleared), as house
    /// 9158 did MarvinBlue/426048731's 9181 after its drop-off in update 1897.
    #[test]
    fn a_hunt_on_a_live_ally_fails() {
        let ally = Quarry {
            allied: true,
            ..tree()
        };
        let mut held = (-1, 0.0);
        let hunt = HuntAction::start(
            CHOP,
            &ally,
            2,
            false,
            false,
            &mut held,
            &engagement(),
            ARMS,
            &mut walker(),
        )
        .expect("a hunt");
        assert_eq!(
            (hunt.state, hunt.timer, hunt.target),
            (FAILED, 0.0, ally.id)
        );
    }

    /// set_state(4) on a dead target, or on the hunter's own livestock, walks to it: a quiet
    /// MoveTo within the task's range, or 1.5 to the livestock (MarvinBlue/426048731's 9171 to its
    /// own sheep 9520 in update 1116).
    #[test]
    fn a_hunt_walks_to_a_dead_target_or_its_own_livestock() {
        let dead = Quarry { state: 3, ..tree() };
        let sheep = Quarry {
            class: LIVESTOCK,
            player: 2,
            ..tree()
        };
        for (q, range) in [(dead, CHOP.range), (sheep, 1.5)] {
            let mut held = (-1, 0.0);
            let hunt = HuntAction::start(
                CHOP,
                &q,
                2,
                false,
                false,
                &mut held,
                &engagement(),
                ARMS,
                &mut walker(),
            )
            .unwrap();
            assert_eq!(
                (hunt.state, hunt.move_to, hunt.walk_range, hunt.attack),
                (WALK, Some(MoveTo::Start), range, None)
            );
        }
    }

    /// MarvinBlue/426048731's lumberjack 9181 in update 1897: its MoveTo to the TC 9156 ended
    /// (state 1); the TC takes wood within a tile, so the load goes to player 2 as wood (1) and
    /// the unit carries nothing after; the hunt has no target to go back to.
    #[test]
    fn a_drop_off_gives_the_load_to_the_player() {
        let mut hunt = HuntAction {
            target: -1,
            state: DROP,
            point: [-1.0; 3],
            target_class: 3,
            task: CHOP,
            attack: None,
            move_to: Some(MoveTo::Arrived),
            walk_range: CHOP.range,
            drop_site: -1,
            carry_to: 9156,
            timer: 0.0,
            move_wait: MoveWait::default(),
        };
        let mut mover = walker();
        let mut world = crate::spatial::Spatial::default();
        fn walk<'a>(mover: &'a mut Mover, world: &'a mut crate::spatial::Spatial) -> Walk<'a> {
            Walk {
                mover: Some(mover),
                me: crate::movement::Me {
                    id: 9181,
                    position: [26.820372, 94.718056, 0.0],
                    action_state: DROP,
                },
                dt: 0.028,
                world,
                plan: None,
            }
        }
        let (mut kind, mut held) = (1, 7.057_053_6);
        let site = DropOff {
            takes: true,
            gap_squared: 0.5,
        };
        let got = hunt
            .update_drop(
                walk(&mut mover, &mut world),
                &mut kind,
                &mut held,
                230,
                Some(site),
            )
            .unwrap();
        assert_eq!(got, Some((1, 7.057_053_6)));
        assert_eq!((kind, held, hunt.move_to), (-1, 0.0, None));
        // carried food of a carcass (15) goes as food; a site that does not take it stops
        let mut kind = 15;
        let mut held = 2.0;
        hunt.move_to = Some(MoveTo::Arrived);
        let got = hunt
            .update_drop(
                walk(&mut mover, &mut world),
                &mut kind,
                &mut held,
                230,
                Some(site),
            )
            .unwrap();
        assert_eq!(got, Some((0, 2.0)));
        hunt.move_to = Some(MoveTo::Arrived);
        let refused = DropOff {
            takes: false,
            ..site
        };
        assert!(
            hunt.update_drop(
                walk(&mut mover, &mut world),
                &mut kind,
                &mut held,
                230,
                Some(refused)
            )
            .is_err()
        );
    }
}

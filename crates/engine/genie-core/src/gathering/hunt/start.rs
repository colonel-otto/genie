//! The Hunt's start, its chase and retarget, a move's target and the walk to a point (states
//! 11 and 15).

use crate::GenieError;
use crate::combat::attack::{Arms, Attack, Engagement};
use crate::gathering::hunt::{
    ARRIVED, ATTACK, BUILDING, CLASS_49, DROP, FAILED, FARM_AT_WORK, HuntAction, HuntTask, IDLE,
    LIVESTOCK, Lead, OWN_LIVESTOCK_RANGE, Quarry, SEARCH, Shown, WALK_POINT, WALK_SEARCH, food,
};
use crate::move_to::{Goal, MoveTo, MoveWait, Walk, start_walk_or_give_up, walk_or_give_up};
use crate::movement::Mover;

/// Who walks to a hunt's point ([`HuntAction::update_point_walk`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointWalker {
    /// The owner is a computer player (`FUN_141659cf0`).
    pub computer: bool,
    /// The unit is a villager (class 4): its `MoveTo` waits once when the motion step gives up.
    pub villager: bool,
    /// Its unit AI's stance (`+0x1a9`), 0 without one: the wait is a second below 2, else two.
    pub stance: u8,
}

/// What a hunt's walk to its point did in an update ([`HuntAction::update_point_walk`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointWalk {
    /// It walks on, or waits to ask again.
    Walking,
    /// State 15's walk ended or failed: the hunt is done ([`HuntAction::set_done`]), its target
    /// and second target to release.
    Done {
        /// The target it let go, −1 for none.
        target: i32,
        /// The second target it let go, −1 for none.
        second: i32,
    },
    /// State 11's walk ended or failed: the hunt searches (`set_state(3)`; the caller tells the
    /// unit AI).
    Search,
}

impl HuntAction {
    /// The factory's constructor (`FUN_140ac2dd0`; its target's add, slot 0x178, is the caller's)
    /// and the install's start (slot 5 with 1, `0x140ac49b0`): a hunter carrying another food
    /// keeps the amount as the task's kind (`FUN_14119d310` on `held`); a loaded hunter sent to
    /// one of its own drop sites carries the load there (`set_state` 8); else `set_state(4)`. A
    /// target of another, unallied player (or of class 49) that is not dead yet is attacked first
    /// (`set_state` 12). `computer` is the owner's type test (`FUN_141659cf0`); `carry_rule` the
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
    ) -> Result<(Self, bool), GenieError> {
        let mut hunt = Self {
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
            lead: Lead::NEW,
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
        // the start (slot 5 with 1): an own target among the hunter's master's drop sites
        // (`+0x7a0`, FUN_14109d4f0; a building annex's main building) takes a load there: it
        // becomes the second target (FUN_140a7a190), the target goes (slot 0x68 with none; the
        // caller takes and releases it), the point and `+0x68` clear, `set_state(8)`; a hunter
        // carrying nothing does not get there
        if target.player == player && target.drop_site && 0.0 < held.1 {
            hunt.target = -1;
            hunt.point = [-1.0; 3];
            hunt.target_class = -1;
            hunt.carry_to = target.id;
            hunt.set_drop(attack.gap, mover)?;
            return Ok((hunt, false));
        }
        let arrived = hunt.chase(target, player, attack, arms, mover)?;
        Ok((hunt, arrived))
    }

    /// `set_state(4)` (`0x140ac60e0` case 4): the sub-actions go (an Attack's `MoveTo` stops the
    /// unit), the timer clears and the target's place becomes the point. A dead target, or the hunter's own
    /// player's livestock, is walked to: a quiet `MoveTo` within the task's range, or
    /// `OWN_LIVESTOCK_RANGE` to the livestock (`FUN_140aa0df0`; its start `0x140aa2100`: a unit
    /// within 0.01 of the target's footprint is there already). A target not dead yet of another,
    /// unallied player (or of class 49) is attacked first: `set_state(12)` (case 0xc) with an
    /// Attack of the master's ranges. A live allied one fails the hunt (`set_state` 13).
    pub(crate) fn chase(
        &mut self,
        target: &Quarry,
        player: usize,
        attack: &Engagement,
        arms: Arms,
        mover: &mut Mover,
    ) -> Result<bool, GenieError> {
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
                    return Err(GenieError::unsupported(
                        "a MoveTo of a unit without speed (state 13)",
                    ));
                }
                MoveTo::Start
            } else {
                MoveTo::Arrived
            });
            return Ok(false);
        }
        if target.class != CLASS_49 && target.allied {
            // LAB_140ac679d: set_state(13): the timer clears; the unit's sprite (slot 0x68 with
            // FUN_140ac5210's) is the caller's
            self.state = FAILED;
            self.timer = 0.0;
            return Ok(false);
        }
        self.state = ATTACK;
        let (a, arrived) = Attack::start(target.id, attack, arms, mover)?;
        self.attack = Some(a);
        Ok(arrived)
    }

    /// Slot 9 (`0x140ac97c0`), the work command's new target for the hunt it keeps: true when the
    /// hunt takes it. A building at work as a farm (top action `0x7d`) takes it to `set_state` 15,
    /// not modelled. Its own player's drop site (one of its master's, `drops_here`,
    /// `FUN_14109d4f0`) while it carries something (`held`) takes the load there: the target goes
    /// (slot 0x68 with none; the caller releases it), the timer and the point clear, the drop site
    /// becomes the second target (`FUN_140a7a190`) with its class, and `set_state(8)`. Another target
    /// becomes the hunt's (slot 0x68, `FUN_140ac7980`; the caller releases the old target and takes
    /// the new) and the hunt starts over at `set_state(4)`. Its own target changes nothing. Some
    /// when it took the target, true when its Attack's `MoveTo` starts arrived (the caller shows
    /// the idle sprite, [`crate::combat::attack::show_arrived`]).
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
    ) -> Result<Option<bool>, GenieError> {
        if target.kind == BUILDING && target.top_action == Some(FARM_AT_WORK) {
            return Err(GenieError::unsupported(
                "a hunt sent to a farm at work (set_state 15)",
            ));
        }
        if target.player == player && drops_here && 0.0 < held {
            self.target = -1;
            self.timer = 0.0;
            self.point = [-1.0; 3];
            self.carry_to = target.id;
            self.target_class = target.class;
            self.set_drop(attack.gap, mover)?;
            return Ok(Some(false));
        }
        if target.id == self.target {
            return Ok(None);
        }
        self.target = target.id;
        self.target_class = target.class;
        self.carry_to = -1;
        self.chase(target, player, attack, arms, mover).map(Some)
    }

    /// Slot 0x40 (`FUN_140ac52c0`): the hunt takes a move's `target` (None for a point) and
    /// `point`; true when it does. A point with a task of a work range it does not take. A unit
    /// on a pasture (`mMyPastureId` +0x794) of another target would leave it (`FUN_141189ca0`),
    /// not modelled. Its own player's drop site (`drops_here`) while it carries something
    /// (`held`) takes the load there: unless it walks there already (state 8), the target goes
    /// (slot 0x68 with none; the caller releases it), the point clears, `+0x68` takes the drop
    /// site's class, which becomes the second target (`FUN_140a7a190`; the caller takes it) and
    /// `set_state(8)`. Any other target, or a point: the point is the move's, `+0x68` −1, the
    /// target goes and `set_state(15)`: a quiet `MoveTo` to the point within the task's range
    /// (`FUN_140aa0d30`).
    #[allow(clippy::too_many_arguments)]
    pub fn take_move(
        &mut self,
        target: Option<&Quarry>,
        point: [f32; 3],
        player: usize,
        held: f32,
        drops_here: bool,
        my_pasture: i32,
        gap: f32,
        mover: &mut Mover,
    ) -> Result<bool, GenieError> {
        if target.is_none() && self.task.range != 0.0 {
            return Ok(false);
        }
        if my_pasture != -1 && target.is_none_or(|t| t.id != my_pasture) {
            return Err(GenieError::unsupported(
                "a hunter leaving its pasture (FUN_141189ca0)",
            ));
        }
        if let Some(t) = target.filter(|t| t.player == player && drops_here && 0.0 < held) {
            if t.id == self.carry_to && self.state == DROP {
                return Ok(true);
            }
            self.target = -1;
            self.point = [-1.0; 3];
            self.target_class = t.class;
            self.carry_to = t.id;
            self.set_drop(gap, mover)?;
            return Ok(true);
        }
        self.point = point;
        self.target_class = -1;
        self.target = -1;
        self.set_point_walk(WALK_POINT, mover)?;
        Ok(true)
    }

    /// `set_state(11)` after a drop-off that found no next target: the walk to the hunt's point
    /// ([`HuntAction::set_point_walk`]), whose end searches again.
    ///
    /// # Errors
    ///
    /// A unit without speed.
    pub fn search_walk(&mut self, mover: &mut Mover) -> Result<(), GenieError> {
        self.set_point_walk(WALK_SEARCH, mover)
    }

    /// `set_state` 11 and 15 (`0x140ac60e0` case 0xb): the sub-actions go, the timer clears and a
    /// quiet `MoveTo` to the point within the task's range starts (`FUN_140aa0d30`, showing the
    /// walking sprite of [`walk_sprite`]).
    ///
    /// [`walk_sprite`]: crate::gathering::hunt::sprites::walk_sprite
    pub(super) fn set_point_walk(
        &mut self,
        state: u8,
        mover: &mut Mover,
    ) -> Result<(), GenieError> {
        self.tear_down(Some(mover))?;
        self.state = state;
        self.timer = 0.0;
        self.walk_range = self.task.range;
        self.move_wait = MoveWait::default();
        if mover.stats.speed == 0.0 {
            return Err(GenieError::unsupported(
                "a MoveTo of a unit without speed (state 13)",
            ));
        }
        self.move_to = Some(MoveTo::Start);
        Ok(())
    }

    /// State 4's wait after its `MoveTo` failed (`0x140ac7a00` case 3, its timer below 0): the
    /// timer runs by `dt`; at its end it clears and the hunt walks to its point (`set_state(11)`,
    /// the target kept), whose end searches.
    ///
    /// # Errors
    ///
    /// A unit that cannot move.
    pub fn wait_then_walk(&mut self, dt: f32, mover: &mut Mover) -> Result<(), GenieError> {
        self.timer += dt;
        if self.timer < 0.0 {
            return Ok(());
        }
        self.timer = 0.0;
        self.set_point_walk(WALK_SEARCH, mover)
    }

    /// State 15's and 11's update (`0x140ac7a00` cases 0xe and 0xa): the `MoveTo` to the point
    /// walks; the motion step giving up turns it to its wait or its failure
    /// ([`HuntAction::give_up_walk`]); a waiting one asks again after its wait (its case 2, its
    /// update returns 0). Once it ended (its update returns 1, 2 or 5) or failed (3, quiet: no
    /// notify), the `set_state` that follows tears it down (`FUN_140a98620`; its destructor stops
    /// the unit, slot 0x5d0): a human player's hunt in state 11 searches (`set_state(3)`: the
    /// state here, the rest the caller's), one in state 15 is done (`set_state(2)`: the target
    /// goes, the point clears and the timer; the unit takes its standing sprite and the second
    /// target goes, the caller's). A computer player's ended walk in state 15 (`set_state` 1) or
    /// failed one (`set_state` 13) is not modelled; a point walk has no target to lose (code 4).
    pub fn update_point_walk(
        &mut self,
        mut walk: Walk<'_>,
        who: PointWalker,
        mut shown: Shown<'_>,
    ) -> Result<PointWalk, GenieError> {
        let ended = match self.move_to {
            Some(MoveTo::Start) => {
                // case 3 after a wait: `+0x6c` goes from 1 to 2 as it asks again
                if self.move_wait.state == 1 {
                    self.move_wait.state = 2;
                }
                self.move_to = Some(MoveTo::Moving);
                let range = self.walk_range;
                walk.me.action_state = self.state;
                let walked = start_walk_or_give_up(walk, Goal::Point(self.point), range, true)?;
                shown.walking(walked.unwrap_or(MoveTo::Moving));
                match walked {
                    Some(state) => self.move_to = Some(state),
                    None => self.give_up_walk(who),
                }
                return Ok(PointWalk::Walking);
            }
            Some(MoveTo::Moving) => {
                walk.me.action_state = self.state;
                match walk_or_give_up(walk)? {
                    Some(state) => self.move_to = Some(state),
                    None => self.give_up_walk(who),
                }
                return Ok(PointWalk::Walking);
            }
            Some(MoveTo::Waiting) => {
                self.move_wait.timer += walk.dt;
                if self.move_wait.delay <= self.move_wait.timer {
                    self.move_wait.timer = 0.0;
                    self.move_to = Some(MoveTo::Start);
                }
                return Ok(PointWalk::Walking);
            }
            Some(MoveTo::Arrived) => true,
            Some(MoveTo::Failed) => false,
            None => {
                return Err(GenieError::unsupported(
                    "a Hunt's walk to a point without its MoveTo",
                ));
            }
        };
        if who.computer {
            return Err(GenieError::unsupported(if ended {
                "a computer player's hunt after its walk (set_state 1)"
            } else {
                "a computer player's hunt whose walk to a point failed (set_state 13)"
            }));
        }
        self.move_to = None;
        walk.mover
            .as_deref_mut()
            .ok_or(crate::turn::HUNTER_UNMOVING)?
            .stop()?;
        if self.state == WALK_SEARCH {
            self.state = SEARCH;
            return Ok(PointWalk::Search);
        }
        let [target, second] = self.set_done();
        Ok(PointWalk::Done { target, second })
    }

    /// `set_state(2)` (`0x140ac60e0` case 2): the hunt is done. The unit takes the sprite it
    /// stands with (slot 0x68 with `FUN_140ac5210`'s, the caller's), the target and the second
    /// target go (slot 0x68 and `FUN_140a7a190` with none: returned in that order for the caller
    /// to release), the point clears and the timer.
    pub const fn set_done(&mut self) -> [i32; 2] {
        self.state = IDLE;
        let target = std::mem::replace(&mut self.target, -1);
        let second = std::mem::replace(&mut self.carry_to, -1);
        self.point = [-1.0; 3];
        self.timer = 0.0;
        [target, second]
    }

    /// The point walk's `MoveTo` answer to the motion step giving up ([`MoveWait::give_up`]: a
    /// human player's villager waits once; quiet).
    pub const fn give_up_walk(&mut self, who: PointWalker) {
        let waits = !who.computer && who.villager;
        self.move_to = Some(self.move_wait.give_up(waits, who.stance));
    }
}

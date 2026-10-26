//! The Gather's constructor and start, its retarget, the walk, the drop and a move's target,
//! the idle and the halt; a farm's stops and their productivity.

use crate::GenieError;
use crate::combat::attack::set_sprite;
use crate::gather::{
    ARRIVED, DEFAULT_PRODUCTIVITY, DROP, FACE, FARM_CLASS, FARM_STOPS, GatherAction, Gatherer,
    IDLE, MOVED, Resource, SPECIAL_TARGETS, STOP_CENTRE, STOP_DRAWS, STOP_FARMS, WALK, WORK, food,
};
use crate::hunt::HuntTask;
use crate::math::rng::Well512;
use crate::move_to::{MoveTo, MoveWait};
use crate::movement::Mover;
use crate::sprite::ActiveSprite;

/// What the constructor reads to time a farm's stops: the gatherer's work rate (its master's
/// `+0x798` plus its powerup, slot 0x6e8), its capacity (`+0x61c`) and the player's attribute the
/// task's `attribute_type2` names (none: 1.0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StopRate {
    /// The work rate.
    pub work_rate: f32,
    /// The capacity.
    pub capacity: f32,
    /// The productivity.
    pub productivity: f32,
}

impl StopRate {
    /// The time per stop (`+0x70`): the capacity times the productivity over the work a second,
    /// shared out over the stops.
    #[must_use]
    pub fn time_per_stop(self, work_value: f32, stops: u8) -> f32 {
        ((self.capacity * self.productivity) / (self.work_rate * work_value)) / f32::from(stops)
    }
}

/// What slot 0x40 reads of a move's target object.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoveTarget {
    /// The object.
    pub id: i32,
    /// Its owner.
    pub player: usize,
    /// The gatherer's master drops its loads there (its main building's master for an annex,
    /// `+0xbb8`; `FUN_14109d4f0`).
    pub drop_site: bool,
    /// The footprint gap from the gatherer to it.
    pub gap: f32,
}

/// The productivity the time per stop reads (the constructor's and the update's): the player's
/// attribute the task's `attribute_type2` names, as it is, or 1.0 for none (`DAT_14330eb8c`).
#[must_use]
pub fn productivity_of(attribute_type2: i16, attributes: &[f32]) -> f32 {
    if attribute_type2 == -1 {
        return DEFAULT_PRODUCTIVITY;
    }
    usize::try_from(attribute_type2)
        .ok()
        .and_then(|i| attributes.get(i))
        .copied()
        .unwrap_or(0.0)
}

/// `set_state(4)`'s point of a farm's next stop (`0x140a8cd40` case 4): a tile of the farm's
/// footprint (its place `position` less its radii `radius`, plus a draw of the Action stream
/// modulo twice each radius, its centre 0.5 in), drawn again while it is the last stop's, at
/// most ten times.
#[must_use]
pub fn stop_point(
    position: [f32; 3],
    radius: [f32; 2],
    last: [f32; 2],
    rng: &mut Well512,
) -> [f32; 2] {
    let (mut ix, mut iy) = (
        (radius[0] + radius[0]) as i32,
        (radius[1] + radius[1]) as i32,
    );
    if ix == 0 || iy == 0 {
        (ix, iy) = (1, 1);
    }
    let (left, bottom) = (
        (position[0] - radius[0]) as i32,
        (position[1] - radius[1]) as i32,
    );
    let [mut x, mut y] = last;
    let mut draws = 0;
    while x == last[0] && y == last[1] && draws < STOP_DRAWS {
        x = ((rng.rand15() as i32) % ix + left) as f32 + STOP_CENTRE;
        y = ((rng.rand15() as i32) % iy + bottom) as f32 + STOP_CENTRE;
        draws += 1;
    }
    [x, y]
}

impl GatherAction {
    /// The factory's constructor with a target (`FUN_140a88310`; the target's add, slot 0x178, is
    /// the caller's): the target's place is the point and its class the target type; a farm (class
    /// 49) or masters 357 and 1188 keep their place as the farm's. One stop, `mGatherX` and
    /// `mGatherY` at −1. A farm of master 50 or 1187 with the game's farm setting has four stops,
    /// each [`StopRate::time_per_stop`] long.
    ///
    /// # Errors
    ///
    /// None yet.
    pub fn new(
        task: HuntTask,
        target: &Resource,
        farm_stops: bool,
        rate: StopRate,
    ) -> Result<Self, GenieError> {
        let farm = if target.class == FARM_CLASS || SPECIAL_TARGETS.contains(&target.master) {
            [target.position[0], target.position[1]]
        } else {
            [0.0; 2]
        };
        let (stops, time_per_stop) =
            if farm_stops && target.class == FARM_CLASS && STOP_FARMS.contains(&target.master) {
                (FARM_STOPS, rate.time_per_stop(task.work_value, FARM_STOPS))
            } else {
                (1, 0.0)
            };
        Ok(Self {
            target: target.id,
            carry_to: -1,
            state: 0,
            point: target.position,
            timer: 0.0,
            task,
            target_type: i32::from(target.class),
            stops,
            current_stop: 0,
            time_per_stop,
            time_at_stop: 0.0,
            last_stop: [0.0; 2],
            worked: false,
            farm,
            gather_at: [-1.0; 2],
            move_to: None,
            walk_to: None,
            walk_range: task.range,
            move_wait: MoveWait::default(),
        })
    }

    /// The install's start (slot 5 with 1, `0x140a8b3a0`): a gatherer carrying another food keeps
    /// the amount as the task's kind (`FUN_14119d310`); one carrying another resource drops it
    /// when the game settings' test `FUN_141302150` (`carry_rule`) says so or it is a computer
    /// player's carrying nothing. A loaded gatherer sent to one of its own drop sites carries the
    /// load there: the target becomes the second target (the caller takes it, then releases it as
    /// the first), the point clears and `set_state(8)` walks there. Else `set_state(4)` walks to
    /// the target. A farm another farmer works (with this one, two or more) is walked to only as
    /// a point (`set_state(11)`, [`GatherAction::set_walk_back`]) and let go.
    ///
    /// # Errors
    ///
    /// What the start does not model.
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        &mut self,
        target: &Resource,
        player: usize,
        computer: bool,
        carry_rule: bool,
        held: &mut (i16, f32),
        mover: &mut Mover,
        rng: &mut Well512,
    ) -> Result<(), GenieError> {
        let kind = self.task.resource;
        let both_food = food(kind) && food(held.0);
        let drops = computer && held.1 == 0.0;
        if (!carry_rule && !drops) || held.0 == kind || both_food {
            if held.0 != kind && both_food {
                held.0 = kind;
            }
        } else {
            *held = (kind, 0.0);
        }
        // a farm another farmer works (`+0x644` above 1 with this one, which the constructor
        // took): set_state(11) to its point, then the farm goes (slot 0x68 with none; the caller
        // lets it go)
        if self.target_type == i32::from(FARM_CLASS) && 1 < target.workers + 1 {
            let radius = if STOP_FARMS.contains(&target.master) {
                0.0
            } else {
                target.radius[0]
            };
            self.set_walk_back(radius, mover)?;
            self.target = -1;
            return Ok(());
        }
        if target.player == player && target.drop_site && 0.0 < held.1 {
            self.target = -1;
            self.point = [-1.0; 3];
            self.carry_to = target.id;
            return self.set_drop(target.gap, mover);
        }
        self.set_walk(target.position, target.radius, target.gap, mover, rng)
    }

    /// Slot 0x48 (`0x140a903a0`) on the work command's new `target` (one the world holds that is
    /// no building, or a farm; the caller's tests): the gathering is not started any more
    /// (`+0x80` cleared first, whatever follows). Its present target in state 6, 7 or 15, or in
    /// 4 or 11 for a computer player, it keeps as it is. A loaded gatherer (`held`) sent to one of
    /// its player's drop sites carries its load there: the timer clears, the target goes (slot
    /// 0x68 with none) with the point, the drop site becomes the second target (`FUN_140a7a190`)
    /// and `set_state(8)` walks there. Any other target becomes the target (slot 0x68) with its
    /// place as the point and its class as the target type, the second target goes and
    /// `set_state(4)` walks to it; the caller takes and lets go the targets that changed. With
    /// the game's farm setting a fish trap (master 199) keeps its place as the farm's, and a stop
    /// farm (masters 50, 1187) too, with its four stops from the first and the time per stop of
    /// `rate`, its time at the stop cleared. A farm another farmer works (with this one, two or
    /// more) is walked to as a point (`set_state(11)`) and let go again (slot 0x68 with none, then
    /// `FUN_140a7a190` with none): the caller takes it and lets it go. Always true: the gather
    /// took the target.
    ///
    /// # Errors
    ///
    /// What it does not model; a unit that cannot move.
    #[allow(clippy::too_many_arguments)]
    pub fn retarget(
        &mut self,
        target: &Resource,
        player: usize,
        computer: bool,
        held: f32,
        (farm_stops, rate): (bool, StopRate),
        mover: &mut Mover,
        rng: &mut Well512,
    ) -> Result<bool, GenieError> {
        self.worked = false;
        if target.id == self.target {
            match self.state {
                FACE | WORK | 15 => return Ok(true),
                WALK | 11 if computer => return Ok(true),
                _ => {}
            }
        }
        if target.player == player && target.drop_site && 0.0 < held {
            self.timer = 0.0;
            self.target = -1;
            self.point = [-1.0; 3];
            self.carry_to = target.id;
            self.set_drop(target.gap, mover)?;
            return Ok(true);
        }
        // slot 0x68 takes the target: its workers count it once it is new
        let workers = target.workers + i32::from(target.id != self.target);
        self.target = target.id;
        self.point = target.position;
        self.target_type = i32::from(target.class);
        if farm_stops {
            if STOP_FARMS.contains(&target.master) {
                self.farm = [target.position[0], target.position[1]];
                self.stops = FARM_STOPS;
                self.current_stop = 0;
                self.time_per_stop = rate.time_per_stop(self.task.work_value, FARM_STOPS);
                self.time_at_stop = 0.0;
            } else if target.master == 199 {
                self.farm = [target.position[0], target.position[1]];
            }
        }
        if target.class == FARM_CLASS && 1 < workers {
            let radius = if STOP_FARMS.contains(&target.master) {
                0.0
            } else {
                target.radius[0]
            };
            self.set_walk_back(radius, mover)?;
            self.target = -1;
            self.carry_to = -1;
            return Ok(true);
        }
        self.carry_to = -1;
        self.set_walk(target.position, target.radius, target.gap, mover, rng)?;
        Ok(true)
    }

    /// `set_state(4)` (`0x140a8cd40` case 4) with its target at `position` (its master's radii
    /// `radius`), `gap` from the unit: the sub-actions go (a `MoveTo`'s teardown stops the unit),
    /// the timer clears and the target's place becomes the point. With one stop a quiet `MoveTo`
    /// to the target within the task's range starts (`FUN_140aa0df0`, its start `0x140aa2100`: a
    /// unit within 0.01 of the target's footprint is there already, state 1; else state 3). A
    /// farm's stops (the game's `+0xc14` being 0) walk to a point of the farm instead
    /// ([`stop_point`]), which becomes the last stop: a quiet `MoveTo` to it within the task's
    /// range (`FUN_140aa0d30`, state 3).
    ///
    /// # Errors
    ///
    /// A gather without its target (the `MoveTo`'s state 13); a unit that cannot move.
    pub fn set_walk(
        &mut self,
        position: [f32; 3],
        radius: [f32; 2],
        gap: f32,
        mover: &mut Mover,
        rng: &mut Well512,
    ) -> Result<(), GenieError> {
        self.tear_down(Some(mover))?;
        self.state = WALK;
        self.timer = 0.0;
        if self.target == -1 {
            return Err(GenieError::unsupported(
                "a gather's walk without its target (set_state 13)",
            ));
        }
        self.point = position;
        if 1 < self.stops {
            let [x, y] = stop_point(position, radius, self.last_stop, rng);
            self.last_stop = [x, y];
            self.move_wait = MoveWait::default();
            if mover.stats.speed == 0.0 {
                return Err(GenieError::unsupported(
                    "a MoveTo of a unit without speed (state 13)",
                ));
            }
            self.walk_to = Some([x, y, position[2]]);
            self.walk_range = self.task.range;
            self.move_to = Some(MoveTo::Start);
            return Ok(());
        }
        self.start_move(gap, mover)
    }

    /// `set_state(8)` (`0x140a8cd40` case 8) with a load and its drop site (`carry_to`, the
    /// caller's search, `FUN_1411671e0`): the current stop clears, the sub-actions go and a quiet
    /// `MoveTo` to the drop site within the task's range starts (`FUN_140aa0df0`).
    ///
    /// # Errors
    ///
    /// No drop site (the unit AI's part); a unit that cannot move.
    pub fn set_drop(&mut self, gap: f32, mover: &mut Mover) -> Result<(), GenieError> {
        self.tear_down(Some(mover))?;
        self.state = DROP;
        self.current_stop = 0;
        if self.carry_to == -1 {
            return Err(GenieError::unsupported(
                "a gather's drop-off without a drop site (notify 0x203)",
            ));
        }
        self.start_move(gap, mover)
    }

    /// A new quiet `MoveTo` within the task's range: arrived within 0.01 of its goal's footprint.
    pub(super) fn start_move(&mut self, gap: f32, mover: &Mover) -> Result<(), GenieError> {
        self.move_wait = MoveWait::default();
        self.walk_to = None;
        self.walk_range = self.task.range;
        self.move_to = Some(if ARRIVED < gap {
            if mover.stats.speed == 0.0 {
                return Err(GenieError::unsupported(
                    "a MoveTo of a unit without speed (state 13)",
                ));
            }
            MoveTo::Start
        } else {
            MoveTo::Arrived
        });
        Ok(())
    }

    /// Slot 0x40 (`0x140a8b740`) on a move to `point`, `target` the object moved to, if any: a
    /// loaded gatherer moved to one of its player's drop sites carries its load there (nothing
    /// changes when that is already the drop site it walks to in state 8): the target goes (slot
    /// 0x68 with none) with the point, the site becomes the second target (`FUN_140a7a190`) and
    /// `set_state(8)` walks there. Any other move becomes the point: the target goes and
    /// `set_state(15)` walks there ([`GatherAction::set_moved`]). Always taken (it returns 1); the
    /// caller takes and lets go the targets that changed.
    ///
    /// # Errors
    ///
    /// A unit that cannot move.
    pub fn take_move(
        &mut self,
        target: Option<MoveTarget>,
        point: [f32; 3],
        player: usize,
        held: f32,
        mover: &mut Mover,
    ) -> Result<(), GenieError> {
        match target.filter(|t| t.player == player && t.drop_site && 0.0 < held) {
            Some(t) if t.id == self.carry_to && self.state == DROP => Ok(()),
            Some(t) => {
                self.target = -1;
                self.point = [-1.0; 3];
                self.carry_to = t.id;
                self.set_drop(t.gap, mover)
            }
            None => {
                self.point = point;
                self.target = -1;
                self.set_moved(mover)
            }
        }
    }

    /// `set_state(15)` (`0x140a8cd40` case 0xf, as 11): the sub-actions go, the timer clears and
    /// a `MoveTo` to the point (`FUN_140aa0d30`, quiet: `+0x60`) within the target's radius plus
    /// the task's range starts; with no target (a move lets it go) the radius is 0.
    ///
    /// # Errors
    ///
    /// A unit that cannot move.
    pub fn set_moved(&mut self, mover: &mut Mover) -> Result<(), GenieError> {
        self.tear_down(Some(mover))?;
        self.state = MOVED;
        self.timer = 0.0;
        if self.target != -1 {
            return Err(GenieError::unsupported(
                "a gather's walk to a point with a target (its radius)",
            ));
        }
        if mover.stats.speed == 0.0 {
            return Err(GenieError::unsupported(
                "a MoveTo of a unit without speed (state 13)",
            ));
        }
        self.move_wait = MoveWait::default();
        self.walk_to = Some(self.point);
        self.walk_range = self.task.range;
        self.move_to = Some(MoveTo::Start);
        Ok(())
    }

    /// `set_state(2)` (case 2): the timer clears, the unit stands ([`GatherAction::stand_sprite`]),
    /// the target and the second target go (slot 0x68, `FUN_140a7a190` with none; the caller
    /// lets them go) and the point is −1.
    pub(super) fn set_idle(&mut self, g: &mut Gatherer<'_>) {
        self.state = IDLE;
        self.timer = 0.0;
        if let Some(s) = self.stand_sprite(g) {
            set_sprite(g.sprite, s, g.sprite_rng);
        }
        self.target = -1;
        self.carry_to = -1;
        self.point = [-1.0; 3];
    }

    /// Slot 0x38 (`FUN_140a8e1b0`, as `TRIBE_Action_Gather`'s `FUN_140abf3c0`) as the unit's
    /// stop calls it: `set_state(2)` (`0x140a8cd40` case 2: the sub-actions go first, a
    /// `MoveTo`'s teardown stopping the unit; then [`GatherAction::set_idle`] with `stand`, the
    /// sprite `FUN_140a8b6b0` gives), true. Returns the target and the second target it let go
    /// (slot 0x68 and `FUN_140a7a190` with none), which the caller releases.
    ///
    /// # Errors
    ///
    /// The unit's stop that the motion step does not model.
    pub(crate) fn halt(
        &mut self,
        mover: Option<&mut Mover>,
        stand: Option<crate::sprite::Graphic>,
        sprite: &mut Option<ActiveSprite>,
        rng: &mut Well512,
    ) -> Result<[i32; 2], GenieError> {
        self.tear_down(mover)?;
        self.state = IDLE;
        self.timer = 0.0;
        if let Some(s) = stand {
            set_sprite(sprite, s, rng);
        }
        let released = [self.target, self.carry_to];
        self.target = -1;
        self.carry_to = -1;
        self.point = [-1.0; 3];
        Ok(released)
    }
}

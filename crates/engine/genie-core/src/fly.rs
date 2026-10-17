//! A bird's Fly action (type 10, vtable `0x142ed4c88`; update `0x140a84b40`, set_state
//! `0x140a84a50`): the bird rests (state 2), picks a point to fly to from the Action stream
//! (state 3) and flies there with a MoveTo sub-action (state 4) until its flight time runs out or
//! the MoveTo ends; then it rests for its task's wait time, or picks the next flight at once.

use crate::fmath::{cos, sin};
use crate::masters::Task;
use crate::move_to::{Goal, MoveTo, Walk, start_walk, walk_on};
use crate::rng::Well512;
use crate::unit_ai::Unsupported;

/// The Fly action's type, which word 9 adds.
pub const FLY: i16 = 10;

const RESTING: u8 = 2;
const CHOOSING: u8 = 3;
const FLYING: u8 = 4;

/// Below this (`DAT_142eb0c60`, 1e-7) a task value counts as none.
const EPSILON: f32 = crate::image::fly::EPSILON;
/// Degrees to radians (`DAT_142eb568c`).
const DEGREE: f32 = crate::image::fly::DEGREE;
/// The flight angle's step between tries, in degrees, and the tries (`FUN_140a84b40`, state 3).
const ANGLE_STEP: i32 = 80;
const TRIES: i32 = 12;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fly {
    /// `+0x18`.
    pub state: u8,
    /// `+0x44`, in seconds: the flight's time left, or the rest's.
    pub timer: f32,
    /// The MoveTo sub-action's state and its point.
    pub move_to: Option<MoveTo>,
    pub goal: Option<[f32; 3]>,
    /// `+0x38`, `+0x3c`: the point the bird keeps within its task's leash.
    pub home: [f32; 2],
    /// The action's task (`+0x48`): its leash (`+0x19`, tiles), rest (`+0x1c`), distance factor
    /// (`+0x28`), avoided class and range (`+0x2c`, `+0x30`), terrain (`+0x16`) and sprites.
    pub task: Task,
}

/// What a flight's choice reads of the bird and the world.
pub struct Sky<'a> {
    /// The bird's top speed (slot 0x208) and place.
    pub speed: f32,
    pub position: [f32; 3],
    /// Its master's radii, which its distance to the leash's centre leaves out
    /// (`FUN_140a847c0`).
    pub radius: [f32; 2],
    /// The map's width and height (`world+0x98`, `+0x18`, `+0x1c`).
    pub map: [i32; 2],
    /// The Action stream.
    pub action: &'a mut Well512,
}

impl Fly {
    /// set_state (`0x140a84a50`): 2 and 3 drop the MoveTo (its teardown stops the bird, the
    /// caller's: the returned flag); 2 gives the bird its master's sprite (slot 0x68, not held
    /// for a bird); 3 draws the flight's time, `r * 10 / 0x7fff + 5` seconds, from the Action
    /// stream.
    fn set_state(&mut self, state: u8, action: &mut Well512) -> bool {
        self.state = state;
        if state == FLYING {
            return false;
        }
        let walked = self.move_to.take().is_some();
        self.goal = None;
        if state == CHOOSING {
            let r = action.rand15() as i32;
            self.timer = ((r * 10) / 0x7fff + 5) as f32;
        }
        walked
    }

    /// One update (`0x140a84b40`): the timer counts down by dt; resting, a bird whose timer ran
    /// out chooses (set_state 3); choosing, it picks its flight ([`Fly::choose`]); flying, a timer
    /// run out rests it (set_state 2, the rest time set), and once the MoveTo is done or gone
    /// (its list's update returns 1) it rests for the rest time more, or chooses at once.
    /// Returns whether the bird stopped (a dropped MoveTo's teardown, slot 0x5d0).
    pub fn update(&mut self, mut walk: Walk<'_>, sky: Sky<'_>) -> Result<bool, Unsupported> {
        if 0.0 < self.timer {
            self.timer -= walk.dt;
        }
        let rest = self.task.wait_time;
        match self.state {
            RESTING => {
                if 0.0 < self.timer {
                    return Ok(false);
                }
                Ok(self.set_state(CHOOSING, sky.action))
            }
            CHOOSING => self.choose(sky),
            FLYING => {
                let mut stopped = false;
                if self.timer <= 0.0 {
                    stopped |= self.set_state(RESTING, sky.action);
                    if EPSILON < rest {
                        self.timer = rest;
                    }
                }
                // the sub-action list's update: none left, or the MoveTo's state 1, is 1
                let done = match self.move_to {
                    None | Some(MoveTo::Arrived) => true,
                    Some(MoveTo::Start) => {
                        let goal = self.goal.ok_or(Unsupported("a flight without its point"))?;
                        self.move_to = Some(MoveTo::Moving);
                        self.move_to =
                            Some(start_walk(walk.reborrow(), Goal::Point(goal), 0.0, false)?);
                        false
                    }
                    Some(MoveTo::Moving) => {
                        self.move_to = Some(walk_on(walk)?);
                        false
                    }
                };
                if !done {
                    return Ok(stopped);
                }
                if EPSILON < rest {
                    self.timer += rest;
                    stopped |= self.set_state(RESTING, sky.action);
                    return Ok(stopped);
                }
                stopped |= self.set_state(CHOOSING, sky.action);
                Ok(stopped)
            }
            _ => Err(Unsupported(
                "a Fly action in a state the slice does not model",
            )),
        }
    }

    /// State 3: an angle drawn from the Action stream (`0..360`, `FUN_140810fc0`), a distance of
    /// speed × time left × the task's distance factor (1 for none); from the bird the point that
    /// far along the angle (the y axis turned by it), tried at most 12 times, 80° on each time:
    /// it must lie inside the map and, with a leash, the bird must be within it of its centre or
    /// the point nearer the centre than the bird. A terrain the point must have and objects of a
    /// class to keep away from are not modelled. A task with sprites draws which one the flight
    /// takes (`r % 3`, not held). The MoveTo to the point at the bird's height within 0 starts
    /// (state 3) and the bird flies (set_state 4); with no distance or no point it rests.
    fn choose(&mut self, sky: Sky<'_>) -> Result<bool, Unsupported> {
        let t = self.task;
        let leash = f32::from(t.holding_attr) * f32::from(t.holding_attr);
        let mut angle = sky.action.below(360) as i32;
        let factor = if t.work_val1 < EPSILON {
            1.0
        } else {
            t.work_val1
        };
        let distance = sky.speed * self.timer * factor;
        let [x, y, z] = sky.position;
        let [w, h] = sky.map;
        let mut tries = 0;
        let point = loop {
            if distance < EPSILON {
                return Ok(self.set_state(RESTING, sky.action));
            }
            let theta = angle as f32 * DEGREE;
            let (c, s) = (cos(theta), sin(theta));
            let dx = c * 0.0 - s;
            let dy = s * 0.0 + c;
            let py = dy * distance + y;
            let px = dx * distance + x;
            angle += ANGLE_STEP;
            tries += 1;
            let inside = 0.0 < px && px <= (w - 1) as f32 && 0.0 < py && py <= (h - 1) as f32;
            if inside
                && (leash <= EPSILON || self.near_home(sky.position, sky.radius, [px, py], leash))
            {
                break [px, py];
            }
            if TRIES - 1 < tries {
                return Ok(self.set_state(RESTING, sky.action));
            }
        };
        if t.terrain != -1 {
            return Err(Unsupported("a flight to a terrain of its task's"));
        }
        if t.work_val2 as i32 as i16 != -1 && 0.0 < t.work_range {
            return Err(Unsupported("a flight keeping away from a class"));
        }
        if EPSILON < t.work_val1 {
            return Err(Unsupported("a flight's animation speed (+0x7b8)"));
        }
        let [moving, proceeding, working, carrying] = t.sprites;
        if moving != -1 || carrying != -1 || proceeding != -1 {
            // the moving, working or proceeding sprite, the first there from r % 3 on
            sky.action.rand15();
            if moving == -1 && working == -1 && proceeding == -1 {
                return Err(Unsupported(
                    "a flight whose task has only a carrying sprite",
                ));
            }
        }
        self.goal = Some([point[0], point[1], z]);
        self.move_to = Some(MoveTo::Start);
        self.set_state(FLYING, sky.action);
        Ok(false)
    }

    /// The leash test of state 3: the bird within it of the centre (the squared footprint
    /// distance, `FUN_140a847c0`), or the point within it, or no farther than the bird.
    fn near_home(&self, at: [f32; 3], radius: [f32; 2], point: [f32; 2], leash: f32) -> bool {
        let [hx, hy] = self.home;
        let gap = |d: f32, r: f32| {
            let g = d.abs() - r;
            if 0.0 <= g { g } else { 0.0 }
        };
        let gx = gap(hx - at[0], radius[0]);
        let gy = gap(hy - at[1], radius[1]);
        let bird = gy * gy + gx * gx;
        if bird <= leash {
            return true;
        }
        let ey = point[1] - hy;
        let ex = point[0] - hx;
        let d = ex * ex + ey * ey;
        d < leash || d <= bird
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::movement::Me;
    use crate::spatial::Spatial;

    fn walk(world: &mut Spatial) -> Walk<'_> {
        Walk {
            mover: None,
            me: Me {
                id: 9925,
                position: [57.845688, 104.499275, 9.0],
            },
            dt: 0.021,
            world,
            plan: None,
        }
    }

    /// MarvinBlue/426048731's hawk 9925 (master 96, HAWKX): its task has no leash, rest or
    /// distance factor, and three sprites.
    fn hawk(state: u8, timer: f32) -> Fly {
        Fly {
            state,
            timer,
            move_to: None,
            goal: None,
            home: [78.5, 103.5],
            task: Task {
                action_type: FLY,
                object_group: -1,
                work_val2: 0.0,
                sprites: [846, 1595, 846, -1],
                ..Task::default()
            },
        }
    }

    fn sky(action: &mut Well512) -> Sky<'_> {
        Sky {
            speed: 1.32,
            position: [57.845688, 104.499275, 9.0],
            radius: [0.2, 0.2],
            map: [120, 120],
            action,
        }
    }

    #[test]
    fn a_flight_whose_time_runs_out_chooses_the_next_at_once_without_a_rest() {
        // update 717: the timer of 0.016 runs out: set_state(2) drops the MoveTo (the bird
        // stops), the empty list's update is 1 and the task has no rest: set_state(3) draws the
        // next flight's time
        let mut world = Spatial::default();
        let mut fly = Fly {
            move_to: Some(MoveTo::Moving),
            goal: Some([57.844986, 104.49953, 9.0]),
            ..hawk(FLYING, 0.015977083)
        };
        let mut r = Well512::default();
        r.set_seed(1);
        let mut e = r.clone();
        let stopped = fly.update(walk(&mut world), sky(&mut r)).unwrap();
        assert!(stopped);
        let t = ((e.rand15() as i32 * 10) / 0x7fff + 5) as f32;
        assert_eq!((fly.state, fly.timer, fly.move_to), (CHOOSING, t, None));
        assert_eq!(r.draws, 1);
    }

    #[test]
    fn a_choice_flies_speed_times_time_along_a_drawn_angle() {
        // update 718: 10 s left less dt, 1.32 a second: 13.17 tiles at the drawn angle, then
        // the sprite draw; the MoveTo starts at the bird's height
        let mut world = Spatial::default();
        let mut fly = hawk(CHOOSING, 10.0);
        let mut r = Well512::default();
        r.set_seed(7);
        let mut e = r.clone();
        fly.update(walk(&mut world), sky(&mut r)).unwrap();
        let angle = e.below(360) as i32;
        let d = 1.32 * (10.0f32 - 0.021) * 1.0;
        let theta = angle as f32 * DEGREE;
        let (c, s) = (cos(theta), sin(theta));
        let point = [
            (c * 0.0 - s) * d + 57.845688,
            (s * 0.0 + c) * d + 104.499275,
        ];
        let inside = |p: [f32; 2]| 0.0 < p[0] && p[0] <= 119.0 && 0.0 < p[1] && p[1] <= 119.0;
        assert!(inside(point), "the seed's first angle lands on the map");
        assert_eq!(fly.goal, Some([point[0], point[1], 9.0]));
        assert_eq!((fly.state, fly.move_to), (FLYING, Some(MoveTo::Start)));
        assert_eq!(r.draws, 2);
    }

    #[test]
    fn a_resting_bird_chooses_once_its_rest_ran_out() {
        let mut world = Spatial::default();
        let mut fly = hawk(RESTING, 0.5);
        let mut r = Well512::default();
        assert_eq!(fly.update(walk(&mut world), sky(&mut r)), Ok(false));
        assert_eq!((fly.state, r.draws), (RESTING, 0));
        fly.timer = 0.01;
        fly.update(walk(&mut world), sky(&mut r)).unwrap();
        assert_eq!((fly.state, r.draws), (CHOOSING, 1));
    }

    #[test]
    fn a_leash_keeps_the_point_near_home() {
        let fly = Fly {
            home: [10.0, 10.0],
            ..hawk(CHOOSING, 1.0)
        };
        // the bird 5 tiles off with a leash of 3 (9 squared): a point nearer home passes, one
        // farther does not
        assert!(fly.near_home([15.0, 10.0, 0.0], [0.0; 2], [12.0, 10.0], 9.0));
        assert!(!fly.near_home([15.0, 10.0, 0.0], [0.0; 2], [16.0, 10.0], 9.0));
        assert!(fly.near_home([12.0, 10.0, 0.0], [0.0; 2], [40.0, 10.0], 9.0));
    }
}

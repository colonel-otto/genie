//! The missile (`RGE_Missile_Object`, type 60) and its action (`RGE_Action_Missile`, type 8): a
//! ranged attacker's swing makes one at its weapon offset (`FUN_141081020`, [`spawn_point`]), its
//! action aims at the target (`FUN_140a9c600`, [`aim`]; set_state 3, `FUN_140a9de70`,
//! [`MissileAction::launch`]) and flies it each update (`FUN_140a9e6e0`,
//! [`MissileAction::fly`]) until it hits the target (the shooter's hit, slot 0x860) or the ground.
//!
//! 158041: `RGE_Action_Missile` vtable `0x142ed5d68` (start slot 5 `0x140a9c5b0`, update slot 6
//! `0x140a9e6e0`, set_state slot 16 `0x140a9de70`); `RGE_Missile_Object` vtable `0x142f57fe8`
//! (update `0x14113e520`).

use crate::fmath;
use crate::unit_ai::Unsupported;

/// The missile action's type, which word 9 adds when it is the top action.
pub const MISSILE: i16 = 8;

/// A missile's states: 4 flies, 1 ends (set_state 1: the object's slot 0xd8, its hit points to 0).
pub const FLYING: u8 = 4;
pub const DONE: u8 = 1;

/// A target's height counts a third of its master's height radius (`FUN_140a9c600`,
/// `DAT_142eb1c98`).
const HEIGHT_SHARE: f32 = crate::image::missile::HEIGHT_SHARE;
/// The hit chance's draw scale (`DAT_142eb69cc`) and a missed missile's half spread
/// (`DAT_142e7f384`).
const DRAW_SCALE: f32 = crate::image::missile::DRAW_SCALE;
const HALF: f32 = crate::image::missile::HALF;
/// A missile past its reach ends this far beyond its point (`DAT_142eb5be8`).
const BEYOND: f32 = crate::image::missile::BEYOND;
/// A missile within this of its point may touch the ground (`DAT_142eb5690`, `+0x994`).
pub const LANDING: f32 = crate::image::missile::LANDING;
/// The hit query holds its ends this far inside the map's far edges (`FUN_1411570c0`,
/// `DAT_142eb5690`).
pub const QUERY_EDGE: f32 = crate::image::missile::QUERY_EDGE;
/// The reach adds this to the shooter's radius and range (`FUN_141081020`, `DAT_142e8c150`).
const REACH_MARGIN: f32 = crate::image::missile::REACH_MARGIN;

/// What a missile's master holds for the flight: the `.dat`'s `projectile_type` (`+0x880`, not
/// saved), `smart_mode` (`+0x881`, the save's `targetting_type`), `hit_mode` (`+0x882`,
/// `missile_hit_info`), `vanish_mode` (`+0x883`, `missile_die_info`) and `projectile_arc`
/// (`+0x888`, `ballistics_ratio`); its master's speed (slot 0xe8, `+0x740`), radii
/// (`+0x5d8`..`+0x5e0`) and the `.dat`'s `fly_mode` (`+0x608`: 0 holds it on the ground, 1 above
/// it, `FUN_141193a10`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MissileStats {
    pub projectile_type: u8,
    pub smart_mode: u8,
    pub hit_mode: u8,
    pub vanish_mode: u8,
    pub arc: f32,
    pub speed: f32,
    pub radius: [f32; 3],
    pub fly_mode: u8,
}

/// Where a missile came from (`+0x9c`, `mProjectileData`, `FUN_140aa86e0`): the firing object's
/// place, the missile master's radii on the ground, the firing object's master and owner (−1
/// both without one, `FUN_140aa8720`). The hit's elevation reads its height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Origin {
    pub position: [f32; 3],
    pub radius: [f32; 2],
    pub master: i32,
    pub owner: i32,
}

impl Origin {
    /// `FUN_140aa8720`: no origin.
    pub const NONE: Origin = Origin {
        position: [0.0; 3],
        radius: [0.0; 2],
        master: -1,
        owner: -1,
    };

    /// The save's 28 bytes: seven 32-bit words.
    pub fn from_bytes(b: &[u8; 28]) -> Self {
        let w = |i: usize| u32::from_le_bytes([b[4 * i], b[4 * i + 1], b[4 * i + 2], b[4 * i + 3]]);
        let f = |i: usize| f32::from_bits(w(i));
        Origin {
            position: [f(0), f(1), f(2)],
            radius: [f(3), f(4)],
            master: w(5) as i32,
            owner: w(6) as i32,
        }
    }
}

/// The missile action (type 8).
#[derive(Clone, Debug, PartialEq)]
pub struct MissileAction {
    /// `+0x18`.
    pub state: u8,
    /// The target (`+0x20`, its id `+0x30`) and the object it was fired from (`+0x28`, `+0x34`),
    /// −1 for none.
    pub target: i32,
    pub fired_from: i32,
    /// `+0x38`..`+0x40`: the target's place when fired.
    pub point: [f32; 3],
    /// `+0x44`: the way flown so far (a straight missile's).
    pub timer: f32,
    /// `+0x68`..`+0x70`: the direction, a unit vector for a straight missile.
    pub direction: [f32; 3],
    /// `+0x74`, `+0x78`: the arc's vertical speed and acceleration.
    pub ballistic_velocity: f32,
    pub ballistic_acceleration: f32,
    /// `+0x7c`: the way left to the aimed point.
    pub distance: f32,
    /// `+0x80`: the master's hit mode, 1 after a miss.
    pub hit_mode: u8,
    /// `+0x98`: the last hit's result.
    pub result: u8,
    /// `+0x9c`.
    pub origin: Origin,
    /// `+0x88`: the objects the last step met (not saved: none after a load).
    pub met: Vec<i32>,
}

/// A missile object's own part.
#[derive(Clone, Debug, PartialEq)]
pub struct Missile {
    /// `+0x990`: how far it flies at most (`max_range`).
    pub max_range: f32,
    /// `+0x998` (`fired_from_id`) and `+0x99c` (`mIsChargeProjectile`).
    pub fired_from: i32,
    pub charge: bool,
    /// `+0x994`: farther than [`LANDING`] from its point; the ground does not stop it.
    pub flying: bool,
    /// `+0x7dc`..`+0x7e4`: this update's step (the moving object's `velocity_x`..`_z`).
    pub step: [f32; 3],
    /// `+0x900`: the firing object's attack count when fired.
    pub attack_count: i32,
    /// Its action, until the action list goes (a dead missile's, or its destruction's).
    pub action: Option<MissileAction>,
}

/// `FUN_141081020`'s place for a new missile: the firing object's place plus its master's weapon
/// offset (`+0x844`..`+0x84c`) turned by its facing (`+0x7e8`): x by cos·oy − sin·ox, y by
/// cos·ox + sin·oy, the height by oz.
pub fn spawn_point(at: [f32; 3], angle: f32, offset: [f32; 3]) -> [f32; 3] {
    let (c, s) = (fmath::cos(angle), fmath::sin(angle));
    let [ox, oy, oz] = offset;
    let y = c * ox + s * oy + at[1];
    let x = (c * oy - s * ox) + at[0];
    [x, y, at[2] + oz]
}

/// `FUN_141081020`'s reach: the firing master's radii's length plus its range plus 2.
pub fn reach(radius: [f32; 2], range: f32) -> f32 {
    fmath::sqrt(radius[0] * radius[0] + radius[1] * radius[1]) + range + REACH_MARGIN
}

/// What the aim reads of the firing object (`+0x28`, else the missile): its master's
/// `base_hit_chance` (`+0x83e`) and `missed_missile_spread` (`+0x860`), its class and the
/// target's master's radius on x and class for the class-54 rule.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aimer {
    pub hit_chance: i16,
    pub spread: f32,
    pub class: i16,
}

/// `FUN_140a9c600`: where a missile at `from` aims for its `point`: a target's height rises by a
/// third of its master's height radius (`target`: its radii x and z and class; None without a
/// target); the Action stream's draw ("intercept") over 100 against the hit chance (80 for a
/// class-54 shooter at a target narrower than a tile of class 27, 52 or 3) misses above it: the
/// hit mode becomes 1 and the point strays by up to the ground distance times the spread, two
/// more draws, half each way. A missile master of id 527 or 528 with an attack count is not
/// modelled.
#[allow(clippy::too_many_arguments)]
pub fn aim(
    point: [f32; 3],
    from: [f32; 3],
    target: Option<([f32; 3], i16)>,
    aimer: Aimer,
    master_id: i16,
    attack_count: i32,
    hit_mode: &mut u8,
    draw: &mut dyn FnMut() -> u32,
) -> Result<[f32; 3], Unsupported> {
    let [mut x, mut y, mut z] = point;
    if let Some((radius, _)) = target {
        z += radius[2] / HEIGHT_SHARE;
    }
    let dx = x - from[0];
    let mut gap = fmath::sqrt((y - from[1]) * (y - from[1]) + dx * dx);
    let mut chance = aimer.hit_chance;
    if let Some((radius, class)) = target
        && aimer.class == 54
        && radius[0] < 1.0
        && matches!(class, 27 | 52 | 3)
    {
        chance = 0x50;
    }
    let r = draw();
    if i32::from(chance) < (r as i32 * 100) / 0x7fff {
        if *hit_mode == 0 {
            *hit_mode = 1;
        }
        gap *= aimer.spread;
        if 0 < attack_count && matches!(master_id, 527 | 528) {
            return Err(Unsupported("a missed missile of master 527 or 528"));
        }
        let gap = gap + 0.0;
        let half = gap * HALF;
        x += (draw() as i32 as f32 * gap) / DRAW_SCALE - half;
        y += (draw() as i32 as f32 * gap) / DRAW_SCALE - half;
    }
    Ok([x, y, z])
}

impl MissileAction {
    /// set_state(3) (`FUN_140a9de70` case 3) from `from` toward the aimed point: the way left
    /// (its length), a straight missile's unit direction (projectile type 0; type 3 keeps the
    /// way itself) with the object's angle on it (slot 0x158: `atan2f` of y over x, returned),
    /// the timer back to 0, the arc's acceleration and vertical speed (`arc` not 0: twice the
    /// arc's rise, `−arc × way × way` or `−arc × way` above 0, over the half flight time squared;
    /// the speed the opposite of the acceleration times that time); then set_state(4). A smart
    /// missile leading a moving target and the projectile types 1 and 2 are not modelled; the
    /// sprite's pitch facet is not held.
    pub fn launch(
        &mut self,
        from: [f32; 3],
        aimed: [f32; 3],
        stats: &MissileStats,
    ) -> Result<Option<f32>, Unsupported> {
        let dx = aimed[0] - from[0];
        let dy = aimed[1] - from[1];
        let dz = aimed[2] - from[2];
        let dz2 = dz * dz;
        let way = fmath::sqrt(dy * dy + dx * dx + dz2);
        self.distance = way;
        if stats.smart_mode & 1 != 0 && self.target != -1 {
            return Err(Unsupported("a smart missile leading its target"));
        }
        let angle = match stats.projectile_type {
            0 => {
                self.direction = if way <= 0.0 {
                    [0.0; 3]
                } else {
                    [dx / way, dy / way, dz / way]
                };
                self.timer = 0.0;
                Some(fmath::atan2(self.direction[1], self.direction[0]))
            }
            3 => {
                self.direction = [dx, dy, dz];
                Some(fmath::atan2(dy, dx))
            }
            _ => return Err(Unsupported("a missile of projectile type 1 or 2")),
        };
        let (acceleration, velocity) = if way <= 0.0 || stats.arc == 0.0 {
            (0.0, 0.0)
        } else {
            let t = if stats.speed <= 0.0 {
                0.0
            } else {
                way / stats.speed
            };
            let t = t * HALF;
            let rise = if stats.arc <= 0.0 {
                way * stats.arc * way
            } else {
                -(way * stats.arc)
            };
            let a = (rise + rise) / (t * t);
            (a, -a * t)
        };
        self.ballistic_acceleration = acceleration;
        self.ballistic_velocity = velocity;
        self.state = FLYING;
        Ok(angle)
    }

    /// One update's flight in state 4 (`FUN_140a9e6e0`), before the step's hits: a straight
    /// missile steps its speed times the world's dt along its direction, its timer counting the
    /// way and its distance shrinking; past its reach and 2 beyond its point it ends (set_state
    /// 1). An arc rises by `(dt × acceleration × 0.5 + vertical speed) × dt`, its vertical speed
    /// gaining `dt × acceleration`. Returns the step and whether the missile flies on (farther
    /// than 0.1 from its point, `+0x994`).
    pub fn fly(
        &mut self,
        dt: f32,
        max_range: f32,
        stats: &MissileStats,
    ) -> Result<([f32; 3], bool), Unsupported> {
        let mut step = match stats.projectile_type {
            0 => {
                let way = stats.speed * dt;
                let d = [
                    way * self.direction[0],
                    way * self.direction[1],
                    way * self.direction[2],
                ];
                self.timer += way;
                self.distance -= way;
                if max_range <= self.timer && self.distance < BEYOND {
                    self.state = DONE;
                }
                d
            }
            3 => {
                self.distance = 0.0;
                self.direction
            }
            _ => return Err(Unsupported("a missile of projectile type 1 or 2")),
        };
        if stats.arc != 0.0 {
            step[2] += (dt * self.ballistic_acceleration * HALF + self.ballistic_velocity) * dt;
            self.ballistic_velocity += dt * self.ballistic_acceleration;
        }
        Ok((step, LANDING < self.distance))
    }
}

/// `FUN_14113f740`'s height test: an object at height `z` whose master's height radius is
/// `height` meets a missile at `missile_z` whose master's is `missile_height` when it lies at
/// most `height` below the missile and at most `missile_height` above.
pub fn meets_height(z: f32, height: f32, missile_z: f32, missile_height: f32) -> bool {
    let d = z - missile_z;
    -height <= d && d <= missile_height
}

/// The ground hit's box test (`FUN_140a9e6e0`): a missile at `at` with radii `radius` lies on
/// the target at `target` with radii `target_radius` when it is nearer on each axis than the
/// two radii together.
pub fn on_target(
    at: [f32; 3],
    radius: [f32; 3],
    target: [f32; 3],
    target_radius: [f32; 2],
) -> bool {
    let dx = (at[0] - target[0]).abs();
    let dy = (at[1] - target[1]).abs();
    dx < target_radius[0] + radius[0] && dy < target_radius[1] + radius[1]
}

/// The height a missile's place takes on the ground (slot 0x60, `FUN_14113eb60`, with the
/// ground's height, from the static new_position): a missile still flying (`+0x994`) keeps its
/// height; else `FUN_141193a10` by its master's fly mode: 0 puts it on the ground, 1 lifts it to
/// the ground when it is below, others keep it.
pub fn grounded(z: f32, ground: f32, flying: bool, fly_mode: u8) -> f32 {
    match fly_mode {
        _ if flying => z,
        0 => ground,
        1 if z < ground => ground,
        _ => z,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MarvinBlue/426048731's spear -28 (master 509: speed 7, arc −0.06), fired at the deer 9655
    /// in the update that made the save after 3137: 2.80657 to go when set_state(3) ran, and its
    /// arc's acceleration −23.52 and vertical speed 4.71503 as the game held them.
    #[test]
    fn a_spear_arcs_to_its_point() {
        let stats = MissileStats {
            projectile_type: 0,
            arc: -0.06,
            speed: 7.0,
            radius: [0.1, 0.1, 0.0],
            ..MissileStats::default()
        };
        let mut a = MissileAction {
            state: 3,
            target: 9655,
            fired_from: 9171,
            point: [0.0; 3],
            timer: 0.0,
            direction: [0.0; 3],
            ballistic_velocity: 0.0,
            ballistic_acceleration: 0.0,
            distance: 0.0,
            hit_mode: 0,
            result: 0,
            origin: Origin::NONE,
            met: Vec::new(),
        };
        // a way of 2.80657 along x
        let angle = a
            .launch([0.0, 0.0, 0.0], [2.80657, 0.0, 0.0], &stats)
            .unwrap();
        assert_eq!(angle, Some(0.0));
        assert_eq!(a.state, FLYING);
        assert!((a.ballistic_acceleration - -23.52).abs() < 1e-3);
        assert!((a.ballistic_velocity - 4.71503).abs() < 1e-4);
        // one update of 52 ms: 0.364 of the way
        let (step, flying) = a.fly(0.052, 5.2828426, &stats).unwrap();
        assert!((a.timer - 0.364).abs() < 1e-6);
        assert!((a.distance - 2.44257).abs() < 1e-5);
        assert!((step[0] - 0.364).abs() < 1e-6);
        assert!(flying);
    }

    #[test]
    fn a_shot_turns_its_offset_by_the_facing() {
        // facing along x: the offset's y is ahead, its x to the side
        let p = spawn_point([10.0, 20.0, 0.0], 0.0, [0.0, 0.5, 1.5]);
        assert_eq!(p, [10.5, 20.0, 1.5]);
        assert_eq!(reach([0.2, 0.2], 3.0), 0.28284273 + 3.0 + 2.0);
    }

    /// MarvinBlue/426048731's spears (fly mode 1) in the update that made the save after 3148:
    /// -30, 0.27 past its point, keeps its height 0.3496 above the flat ground; -29 would sink to
    /// −0.3602 and is lifted onto it (the ground hit: its height is the ground's).
    #[test]
    fn a_spear_past_its_point_takes_the_ground_by_its_fly_mode() {
        assert_eq!(grounded(0.349_624_8, 0.0, false, 1), 0.349_624_8);
        assert_eq!(grounded(-0.360_166_4, 0.0, false, 1), 0.0);
        // still flying it keeps any height; fly mode 0 always takes the ground
        assert_eq!(grounded(-0.360_166_4, 0.0, true, 1), -0.360_166_4);
        assert_eq!(grounded(0.349_624_8, 0.0, false, 0), 0.0);
    }

    #[test]
    fn a_missile_meets_an_object_at_its_height_and_lands_on_its_box() {
        // an object half a tile high meets a missile of no height from its foot to its top
        assert!(meets_height(0.0, 0.5, 0.3, 0.0));
        assert!(!meets_height(0.0, 0.5, 0.6, 0.0));
        assert!(!meets_height(0.2, 0.5, 0.0, 0.0));
        // the box test: nearer than the radii together on both axes
        assert!(on_target(
            [10.0, 10.0, 0.0],
            [0.1, 0.1, 0.0],
            [10.25, 9.8, 0.0],
            [0.2, 0.2]
        ));
        assert!(!on_target(
            [10.0, 10.0, 0.0],
            [0.1, 0.1, 0.0],
            [10.3, 9.8, 0.0],
            [0.2, 0.2]
        ));
    }
}

//! Where an object faces: its angle (`+0x7e8`, radians on the map, 0 to 2π) and its facet
//! (`+0xe`), counted in its current sprite's facets (`+0x140` -> `+0xa`). A walking unit's yaw
//! takes the facet of its forward vector and the angle of that facet (`FUN_141163140`); a unit
//! facing a target turns its angle by the facets between (`FUN_141162db0`); a new object draws
//! its angle (`FUN_141161450`). A missile's spawn point turns by the angle (`FUN_141081020`).
//!
//! The routines read the current sprite's facet count and its sequence flags' bit 1
//! ([`SpriteFacets`]); the slice takes them from the unit's master when every sprite of the
//! master has the same ([`crate::movement::MoveStats::facets`]).

use crate::fmath::{self, Vec3};

const TWO_PI: f32 = crate::image::facing::TWO_PI;
const MINUS_TWO_PI: f32 = crate::image::facing::MINUS_TWO_PI;
const PI: f32 = crate::image::facing::PI;
const MINUS_PI: f32 = crate::image::facing::MINUS_PI;
/// A facet's angle starts a quarter turn of π in (`DAT_142eb6e44`).
const QUARTER_PI: f32 = crate::image::facing::QUARTER_PI;
/// A facing within half a facet (`DAT_142e7f384`, `DAT_142eb6e58`) faces.
const HALF: f32 = crate::image::facing::HALF;
const MINUS_HALF: f32 = crate::image::facing::MINUS_HALF;
/// No offset below this (`DAT_142eb0c60`).
const EPSILON: f32 = crate::image::facing::EPSILON;
/// The cosine's bounds (`DAT_14330eb8c`, `DAT_14330ec20`) and the diagonal's square (`DAT_142e8c150`)
/// of the forward vector's facet (`FUN_14119d0c0`).
const ONE: f32 = crate::image::facing::ONE;
const MINUS_ONE: f32 = crate::image::facing::MINUS_ONE;
const TWO: f32 = crate::image::facing::TWO;

/// What the facing routines read of the current sprite: its facets and whether its sequence
/// flags have bit 1 (`+0x1a`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpriteFacets {
    pub count: i16,
    pub flag_1: bool,
}

/// The facets most routines count: the sprite's, at least 8 (`FUN_14114d780`, `FUN_14119d0c0`).
fn at_least_eight(s: SpriteFacets) -> i32 {
    i32::from(s.count).max(8)
}

/// An angle held in 0 to 2π by one turn either way (the routines' tail).
fn wrapped(a: f32) -> f32 {
    if a < 0.0 {
        a + TWO_PI
    } else if TWO_PI < a {
        a - TWO_PI
    } else {
        a
    }
}

/// `FUN_141158f20`: `x` modulo `m` into 0 to `m`.
#[must_use]
pub fn modulo(x: f32, m: f32) -> f32 {
    if x < 0.0 {
        let r = libm::fmodf(-x, m);
        if r == 0.0 { r } else { m - r }
    } else {
        libm::fmodf(x, m)
    }
}

/// `FUN_14119d0c0`: the facet of a forward vector on the ground: its angle to the diagonal
/// (1, 0, 1) (`acosf`), in facets of 2π over the count, rounded half up, counted the other way
/// when the cross product's height is below 0.
#[must_use]
pub fn facet_of_forward(forward: Vec3, s: SpriteFacets) -> u8 {
    let [f0, f1, f2] = forward;
    let length = fmath::sqrt(f0 * f0 + 0.0 + f2 * f2);
    let u = [f0 / length, f1 / length, f2 / length];
    let root = fmath::sqrt(TWO);
    let d = [ONE / root, 0.0 / root, ONE / root];
    let dot = d[0] * u[0] + d[1] * u[1] + d[2] * u[2];
    let angle = if dot < ONE {
        if MINUS_ONE < dot {
            fmath::acos(dot)
        } else {
            PI
        }
    } else {
        0.0
    };
    let side = fmath::cross(u, d)[1];
    let n = at_least_eight(s);
    let q = angle / (TWO_PI / n as f32);
    let floor = libm::floorf(q);
    let low = floor as i32 as u8;
    let facet = if q < (floor as i32) as f32 + HALF {
        low
    } else {
        low.wrapping_add(1)
    };
    if 0.0 <= side {
        facet
    } else {
        (n as u8).wrapping_sub(facet)
    }
}

/// `FUN_14114d780`: the angle of a facet: its share of a turn plus a quarter of π, held in 0 to
/// 2π.
#[must_use]
pub fn angle_of_facet(facet: u8, s: SpriteFacets) -> f32 {
    let n = at_least_eight(s);
    wrapped((TWO_PI / n as f32) * f32::from(facet) + QUARTER_PI)
}

/// `FUN_14114d6c0`: the facet of an angle and its unrounded share (`+0x804`), for a sprite with
/// facets (at least 8 counted); none without (0, 0).
#[must_use]
pub fn facet_of_angle(angle: f32, s: Option<SpriteFacets>) -> (u8, f32) {
    let Some(s) = s.filter(|s| 0 < s.count) else {
        return (0, 0.0);
    };
    let n = at_least_eight(s) as f32;
    let q = ((angle - QUARTER_PI) * n) / TWO_PI;
    let share = modulo(q, n);
    let mut q = q + HALF;
    if q < 0.0 {
        q += n;
    }
    (q as i32 as u8, share)
}

/// `FUN_14114eaa0`: how far `to` lies from where an object at `at` facing `angle` faces, in
/// facets (at least 8): the angle to it (`atan2f`, 0 to 2π) less the facing angle, wrapped into
/// −π to π, times the facets over 2π; 0 on the same point (within 1e-7) or without facets.
#[must_use]
pub fn facets_to(at: [f32; 2], angle: f32, to: [f32; 2], s: Option<SpriteFacets>) -> f32 {
    let Some(s) = s.filter(|s| 0 < s.count) else {
        return 0.0;
    };
    let n = at_least_eight(s);
    let dx = to[0] - at[0];
    let dy = to[1] - at[1];
    if !(EPSILON < dx.abs() || EPSILON < dy.abs()) {
        return 0.0;
    }
    let mut a = fmath::atan2(dy, dx);
    if a < 0.0 {
        a += TWO_PI;
    }
    a -= angle;
    if PI < a {
        a += MINUS_TWO_PI;
    }
    if a < MINUS_PI {
        a += TWO_PI;
    }
    (n as f32 / TWO_PI) * a
}

/// `FUN_141162db0` for a unit without a turn time (its master's `+0x758` 0): facing `to` from `at`
/// within half a facet it faces; else its angle turns by the facets between at once (held in 0
/// to 2π) and it faces. Returns the angle.
#[must_use]
pub fn face(at: [f32; 2], angle: f32, to: [f32; 2], s: Option<SpriteFacets>) -> f32 {
    let n = s.map_or(8, |s| if s.count < 8 { 8 } else { i32::from(s.count) });
    let off = facets_to(at, angle, to, s);
    if (MINUS_HALF..=HALF).contains(&off) {
        return angle;
    }
    wrapped((off * TWO_PI) / n as f32 + angle)
}

/// `FUN_141161450`'s draw: a new object whose sprite turns (sequence flag 1) with more than one
/// facet faces a facet drawn from the Object stream (`r`, 0 to 0x7fff): `(r × facets) / 0x7fff`
/// facets of 2π over the facets.
#[must_use]
pub fn drawn_angle(r: u32, s: SpriteFacets) -> f32 {
    let n = i32::from(s.count);
    let k = ((r as i32).wrapping_mul(n)) / 0x7fff;
    wrapped(k as f32 * (TWO_PI / n as f32))
}

/// A new object's sprite turns and has more than one facet: [`drawn_angle`] draws.
#[must_use]
pub fn draws_angle(s: Option<SpriteFacets>) -> bool {
    s.is_some_and(|s| s.flag_1 && 1 < s.count)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VILLAGER: SpriteFacets = SpriteFacets {
        count: 16,
        flag_1: true,
    };

    #[test]
    fn a_forward_vector_takes_its_facet_and_the_facet_its_angle() {
        // facing the diagonal (1, 0, 1): facet 0, a quarter of π
        let f = facet_of_forward([1.0, 0.0, 1.0], VILLAGER);
        assert_eq!(f, 0);
        assert_eq!(angle_of_facet(f, VILLAGER), QUARTER_PI);
        // facing x: an eighth of a turn round, two facets of sixteen, the other way
        let f = facet_of_forward([1.0, 0.0, 0.0], VILLAGER);
        assert_eq!(f, 14);
        assert_eq!(
            angle_of_facet(14, VILLAGER),
            wrapped(TWO_PI / 16.0 * 14.0 + QUARTER_PI)
        );
    }

    #[test]
    fn a_unit_faces_within_half_a_facet_or_turns_at_once() {
        let at = [100.0, 40.0];
        let angle = 0.0;
        // straight along its angle: it faces already
        assert_eq!(face(at, angle, [105.0, 40.0], Some(VILLAGER)), angle);
        // a quarter turn away: it turns by four facets of sixteen
        let turned = face(at, angle, [100.0, 45.0], Some(VILLAGER));
        assert!((turned - TWO_PI / 4.0).abs() < 1e-6);
    }

    #[test]
    fn the_modulo_of_a_negative_value_counts_back_from_the_divisor() {
        assert_eq!(modulo(-1.0, 16.0), 15.0);
        assert_eq!(modulo(-16.0, 16.0), 0.0);
        assert_eq!(modulo(17.0, 16.0), 1.0);
    }
}

//! The game's f32 math, bit for bit (checked by `tests/crt_math.rs` against build 158041's own
//! routines on recorded inputs).
//!
//! The game links software routines from FreeBSD's msun: acosf `0x14099dc60`, cosf `0x14099e4d0`,
//! sinf `0x14099ed60`. The `libm` crate ports the same sources. Its sqrtf `0x14099ef70` is an
//! integer Newton iteration that rounds correctly, so IEEE's square root gives the same bits.

/// The square root, as the game's (correctly rounded).
#[must_use]
pub fn sqrt(x: f32) -> f32 {
    x.sqrt()
}

/// The arc cosine, as the game's acosf (msun's) computes it.
#[must_use]
pub fn acos(x: f32) -> f32 {
    libm::acosf(x)
}

/// The cosine, as the game's cosf (msun's) computes it.
#[must_use]
pub fn cos(x: f32) -> f32 {
    libm::cosf(x)
}

/// The sine, as the game's sinf (msun's) computes it.
#[must_use]
pub fn sin(x: f32) -> f32 {
    libm::sinf(x)
}

/// A vector in the game's order (`BVector`): x, height, y. The map's y is the third component.
pub type Vec3 = [f32; 3];

/// `a - b` (`FUN_14087e560`).
#[must_use]
pub fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// `a + b` (`FUN_14087e5e0`).
#[must_use]
pub fn add(a: Vec3, b: Vec3) -> Vec3 {
    [b[0] + a[0], b[1] + a[1], b[2] + a[2]]
}

/// `s * v` (`FUN_14087e4a0`).
#[must_use]
pub fn scale(s: f32, v: Vec3) -> Vec3 {
    [s * v[0], s * v[1], s * v[2]]
}

/// `a · b`, summed left to right.
#[must_use]
pub fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// `a × b` (`FUN_14087e4d0`), each component in the game's operand order.
#[must_use]
pub fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        b[0] * a[2] - a[0] * b[2],
        a[0] * b[1] - b[0] * a[1],
    ]
}

/// `|v|`, the squares summed x, height, y.
#[must_use]
pub fn length(v: Vec3) -> f32 {
    sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2])
}

/// `v / |v|` (`FUN_141027d00`).
#[must_use]
pub fn normalize(v: Vec3) -> Vec3 {
    let l = length(v);
    [v[0] / l, v[1] / l, v[2] / l]
}

/// The angle between two unit vectors (`FUN_14087e650`): π/2 when their dot product is below
/// 1e-7 in size, 0 from 1 up, π from -1 down, else its arc cosine (a NaN dot included).
#[must_use]
pub fn angle_between(a: Vec3, b: Vec3) -> f32 {
    let d = dot(a, b);
    if d.abs() < 1e-7 {
        std::f32::consts::FRAC_PI_2
    } else if d >= 1.0 {
        0.0
    } else if d <= -1.0 {
        std::f32::consts::PI
    } else {
        // COMISS -1, d; JC at 0x14087e6ed: an unordered dot goes on to acosf
        acos(d)
    }
}

/// The distance of `p` from the line through `a` along `dir` (`FUN_14087e710`): the cross
/// product of `p - a` and `dir`, its length over `dir`'s, each sum in the game's order.
#[must_use]
pub fn line_distance(p: Vec3, a: Vec3, dir: Vec3) -> f32 {
    let x = (p[1] - a[1]) * dir[2] - (p[2] - a[2]) * dir[1];
    let z = (p[0] - a[0]) * dir[1] - (p[1] - a[1]) * dir[0];
    let y = (p[2] - a[2]) * dir[0] - (p[0] - a[0]) * dir[2];
    sqrt((y * y + x * x + z * z) / (dir[1] * dir[1] + dir[0] * dir[0] + dir[2] * dir[2]))
}

/// The rotation by `angle` about the unit `axis` (`FUN_14087e350`): the 3×3 part of the game's
/// 3×4 matrix, rows as it stores them.
#[must_use]
pub fn rotation(angle: f32, axis: Vec3) -> [Vec3; 3] {
    let (c, s) = (cos(angle), sin(angle));
    let t = 1.0 - c;
    let [x, y, z] = axis;
    let xy = y * x * t;
    let xz = z * x * t;
    let yz = z * y * t;
    [
        [x * x * t + c, xy - z * s, y * s + xz],
        [z * s + xy, y * y * t + c, yz - x * s],
        [xz - y * s, x * s + yz, z * z * t + c],
    ]
}

/// `m · v` (`FUN_14087e2c0`), each row's products summed in the game's order.
#[must_use]
pub fn rotate(m: &[Vec3; 3], v: Vec3) -> Vec3 {
    [
        v[1] * m[0][1] + v[0] * m[0][0] + v[2] * m[0][2],
        v[0] * m[1][0] + v[1] * m[1][1] + v[2] * m[1][2],
        v[0] * m[2][0] + v[1] * m[2][1] + v[2] * m[2][2],
    ]
}

/// `atan2f(y, x)` (`0x14099e020`).
#[must_use]
pub fn atan2(y: f32, x: f32) -> f32 {
    libm::atan2f(y, x)
}

/// `COMISS a, b` then `JC`: taken when `a` is less than `b` or either is NaN (unordered).
#[must_use]
pub fn below_or_unordered(a: f32, b: f32) -> bool {
    a < b || a.is_nan() || b.is_nan()
}

/// `MINSS a, b`: `a` when it is less than `b`, else `b` (also when either is NaN).
#[must_use]
pub fn minss(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

/// `MAXSS a, b`: `a` when it is greater than `b`, else `b` (also when either is NaN).
#[must_use]
pub fn maxss(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_jc_after_comiss_is_taken_below_and_unordered() {
        assert!(below_or_unordered(1.0, 2.0));
        assert!(!below_or_unordered(2.0, 2.0) && !below_or_unordered(3.0, 2.0));
        assert!(below_or_unordered(f32::NAN, 2.0) && below_or_unordered(2.0, f32::NAN));
    }

    #[test]
    fn minss_and_maxss_take_the_second_operand_when_either_is_nan() {
        assert_eq!((minss(1.0, 2.0), minss(2.0, 1.0)), (1.0, 1.0));
        assert_eq!((maxss(1.0, 2.0), maxss(2.0, 1.0)), (2.0, 2.0));
        assert_eq!(minss(f32::NAN, 2.0), 2.0);
        assert!(minss(2.0, f32::NAN).is_nan());
        assert_eq!(maxss(f32::NAN, 2.0), 2.0);
        assert!(maxss(2.0, f32::NAN).is_nan());
    }

    #[test]
    fn the_angle_between_unit_vectors_follows_the_games_compares() {
        // FUN_14087e650: |d| below 1e-7 gives π/2, 1 and up 0, -1 and down π, else acosf; a NaN
        // dot falls through all three unordered compares (JBE, JC, JC) to acosf(NaN)
        let x = [1.0, 0.0, 0.0];
        assert_eq!(
            angle_between(x, [0.0, 0.0, 1.0]),
            std::f32::consts::FRAC_PI_2
        );
        assert_eq!(angle_between(x, x), 0.0);
        assert_eq!(angle_between(x, [-1.0, 0.0, 0.0]), std::f32::consts::PI);
        assert_eq!(angle_between(x, [0.5, 0.0, 0.0]), acos(0.5));
        assert!(angle_between(x, [f32::NAN, 0.0, 0.0]).is_nan());
    }

    /// `FUN_14087e710`: (3, 4, 0) lies 4 from the x axis, whatever the direction's length.
    #[test]
    fn a_points_distance_from_a_line() {
        assert_eq!(
            line_distance([3.0, 4.0, 0.0], [0.0; 3], [2.0, 0.0, 0.0]),
            4.0
        );
        assert_eq!(
            line_distance([3.0, 0.0, 0.0], [0.0; 3], [1.0, 0.0, 0.0]),
            0.0
        );
    }
}

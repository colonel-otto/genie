//! The game's 3-vectors (`BVector`): sums, products, length, angle, line distance and
//! rotation.

use crate::math::fmath::{acos, cos, sin, sqrt};

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

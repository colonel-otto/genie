//! The game's f32 math, bit for bit (checked by `tests/crt_math.rs` against build 158041's own
//! routines on recorded inputs).
//!
//! The game links musl's math routines, which take FreeBSD msun's sources: acosf `0x14099dc60`,
//! atan2f `0x14099e020`, cosf `0x14099e4d0`, sinf `0x14099ed60` and the kernels and argument
//! reduction they call. Each is ported here from its machine code: its constants (read from the
//! image), its operation order, its branches and its integer bit tests. Its sqrtf `0x14099ef70` is
//! an integer Newton iteration that rounds correctly, so IEEE's square root gives the same bits.

/// 1 (`DAT_14330eb8c`).
const ONE: f32 = f32::from_bits(0x3f80_0000);
/// 1/2 (`DAT_142e7f384`).
const HALF: f32 = f32::from_bits(0x3f00_0000);

/// The square root, as the game's (correctly rounded).
#[must_use]
pub fn sqrt(x: f32) -> f32 {
    x.sqrt()
}

/// π/2's leading bits (`DAT_142ec89f0`): acosf near 0, also atanf's bound.
const PIO2_HI: f32 = f32::from_bits(0x3fc9_0fda);
/// π/2's next bits (`DAT_142ec89e0`).
const PIO2_LO: f32 = f32::from_bits(0x33a2_2168);
/// acosf(-1), twice [`PIO2_HI`] (`DAT_142ec89f4`).
const ACOS_OF_MINUS_ONE: f32 = f32::from_bits(0x4049_0fda);
/// acosf's rational R(z) = z (P0 + z (P1 − z P2)) / (1 − z Q1): P0 (`DAT_142ec89e8`).
const ACOS_P0: f32 = f32::from_bits(0x3e2a_aa75);
/// R's P1 (`DAT_142ec89f8`).
const ACOS_P1: f32 = f32::from_bits(0xbd2f_13ba);
/// R's P2, subtracted: msun's pS2 negated (`DAT_142ec89e4`).
const ACOS_P2: f32 = f32::from_bits(0x3c0d_d36b);
/// R's Q1, subtracted: msun's qS1 negated (`DAT_142ec89ec`).
const ACOS_Q1: f32 = f32::from_bits(0x3f34_e5ae);

/// acosf's R(z), each product and sum in the game's order (inlined three times in acosf).
fn acos_r(z: f32) -> f32 {
    ((ACOS_P1 - z * ACOS_P2) * z + ACOS_P0) * z / (ONE - z * ACOS_Q1)
}

/// The arc cosine: the game's acosf `0x14099dc60` (msun's `e_acosf.c`). |x| from 1 up and NaN
/// take the bit test's branch: ±1 give 0 and π, the rest the invalid `0 / (x − x)`. Below 1/2 it
/// is π/2 less x's series, above 1/2 twice the arc sine of `sqrt((1 ∓ x) / 2)`.
#[must_use]
pub fn acos(x: f32) -> f32 {
    let hx = x.to_bits();
    let ix = hx & 0x7fff_ffff;
    if ix >= 0x3f80_0000 {
        if ix != 0x3f80_0000 {
            // SUBSS x, x; DIVSS 0, (x − x): NaN for |x| above 1, infinities and NaN
            #[allow(clippy::eq_op)]
            return 0.0 / (x - x);
        }
        return if hx >> 31 != 0 {
            ACOS_OF_MINUS_ONE
        } else {
            0.0
        };
    }
    if ix < 0x3f00_0000 {
        if ix <= 0x3280_0000 {
            return PIO2_HI;
        }
        return PIO2_HI - (x - (PIO2_LO - acos_r(x * x) * x));
    }
    if hx >> 31 != 0 {
        let z = (x + ONE) * HALF;
        let s = sqrt(z);
        let h = PIO2_HI - (acos_r(z) * s - PIO2_LO + s);
        return h + h;
    }
    let z = (ONE - x) * HALF;
    let s = sqrt(z);
    let df = f32::from_bits(s.to_bits() & 0xffff_f000);
    let c = (z - df * df) / (df + s);
    let h = acos_r(z) * s + c + df;
    h + h
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

/// fabsf `0x14099e7b0`: `x` with its sign bit cleared (an `AND` on its bits).
const fn fabs(x: f32) -> f32 {
    f32::from_bits(x.to_bits() & 0x7fff_ffff)
}

/// `0x14099e870`, msun's isnan as the game compiles it: `x` widened to a double (`CVTSS2SD`), the
/// high word without its sign plus 1 for a non-zero low word, above the infinity's `0x7ff00000`.
fn is_nan(x: f32) -> bool {
    let d = f64::from(x).to_bits();
    ((d >> 32) as u32 & 0x7fff_ffff) + u32::from(d as u32 != 0) > 0x7ff0_0000
}

/// atanf of a negative x from 2^26 down: −π/2's leading bits (`DAT_142ec8a74`).
const ATAN_OF_MINUS_HUGE: f32 = f32::from_bits(0xbfc9_0fda);
/// 2 (`DAT_142e8c150`).
const TWO: f32 = f32::from_bits(0x4000_0000);
/// 3/2 (`DAT_142eb4838`).
const ONE_AND_A_HALF: f32 = f32::from_bits(0x3fc0_0000);
/// −1 (`DAT_14330ec20`).
const MINUS_ONE: f32 = f32::from_bits(0xbf80_0000);
/// atanf's series in z = t², w = z²: (T1 − w T3) w + ((w T4 + T2) w + T0) z. T0 (`DAT_142ec8a6c`).
const ATAN_T0: f32 = f32::from_bits(0x3eaa_aaa9);
/// The series' T1 (`DAT_142ec8a70`).
const ATAN_T1: f32 = f32::from_bits(0xbe4c_ca98);
/// The series' T2 (`DAT_142ec8a68`).
const ATAN_T2: f32 = f32::from_bits(0x3e11_f50d);
/// The series' T3, subtracted: msun's aT3 negated (`DAT_142ec8a64`).
const ATAN_T3: f32 = f32::from_bits(0x3dda_1247);
/// The series' T4 (`DAT_142ec8a60`).
const ATAN_T4: f32 = f32::from_bits(0x3d7c_ac25);
/// atan(1/2)'s leading bits (`DAT_142ec8a40`).
const ATAN_HI_0: f32 = f32::from_bits(0x3eed_6338);
/// atan(1)'s leading bits (`DAT_142ec8a44`).
const ATAN_HI_1: f32 = f32::from_bits(0x3f49_0fda);
/// atan(3/2)'s leading bits (`DAT_142ec8a48`).
const ATAN_HI_2: f32 = f32::from_bits(0x3f7b_985e);
/// atan(∞)'s leading bits (`DAT_142ec8a4c`).
const ATAN_HI_3: f32 = f32::from_bits(0x3fc9_0fda);
/// atan(1/2)'s next bits (`DAT_142ec8a50`).
const ATAN_LO_0: f32 = f32::from_bits(0x31ac_3769);
/// atan(1)'s next bits (`DAT_142ec8a54`).
const ATAN_LO_1: f32 = f32::from_bits(0x3322_2168);
/// atan(3/2)'s next bits (`DAT_142ec8a58`).
const ATAN_LO_2: f32 = f32::from_bits(0x3314_0fb4);
/// atan(∞)'s next bits (`DAT_142ec8a5c`).
const ATAN_LO_3: f32 = f32::from_bits(0x33a2_2168);
/// The breakpoints' angles, the image's table at `0x142ec8a40`.
const ATAN_HI: [f32; 4] = [ATAN_HI_0, ATAN_HI_1, ATAN_HI_2, ATAN_HI_3];
/// Their next bits, the table at `0x142ec8a50`.
const ATAN_LO: [f32; 4] = [ATAN_LO_0, ATAN_LO_1, ATAN_LO_2, ATAN_LO_3];

/// The arc tangent: the game's atanf `0x14099e2f0` (msun's `s_atanf.c`). |x| from 2^26 up is
/// ±π/2 (a NaN comes back as it is), below 2^-12 it is x. Below 7/16 the series runs on x; above,
/// |x| is moved next to a breakpoint (1/2, 1, 3/2, ∞) and the series added to its angle.
#[must_use]
pub fn atan(x: f32) -> f32 {
    let hx = x.to_bits();
    let negative = hx >> 31 != 0;
    let ix = hx & 0x7fff_ffff;
    if ix >= 0x4c80_0000 {
        if is_nan(x) {
            return x;
        }
        return if negative {
            ATAN_OF_MINUS_HUGE
        } else {
            PIO2_HI
        };
    }
    let (id, t) = if ix < 0x3ee0_0000 {
        if ix < 0x3980_0000 {
            // a subnormal x is squared into a dead store (msun raises underflow): x it is
            return x;
        }
        (None, x)
    } else {
        let a = fabs(x);
        if ix < 0x3f98_0000 {
            if ix < 0x3f30_0000 {
                (Some(0), (a * TWO - ONE) / (a + TWO))
            } else {
                (Some(1), (a - ONE) / (a + ONE))
            }
        } else if ix < 0x401c_0000 {
            (Some(2), (a - ONE_AND_A_HALF) / (a * ONE_AND_A_HALF + ONE))
        } else {
            (Some(3), MINUS_ONE / a)
        }
    };
    let z = t * t;
    let w = z * z;
    let s2 = (ATAN_T1 - w * ATAN_T3) * w;
    let s1 = ((w * ATAN_T4 + ATAN_T2) * w + ATAN_T0) * z;
    let series = (s2 + s1) * t;
    let Some(id) = id else {
        return t - series;
    };
    let r = ATAN_HI[id] - ((series - ATAN_LO[id]) - t);
    if negative { -r } else { r }
}

/// π (`DAT_142eb0c68`).
const PI: f32 = f32::from_bits(0x4049_0fdb);
/// −π (`DAT_142ec8a3c`).
const MINUS_PI: f32 = f32::from_bits(0xc049_0fdb);
/// π/2 (`DAT_142eb0c64`).
const FRAC_PI_2: f32 = f32::from_bits(0x3fc9_0fdb);
/// −π/2 (`DAT_142ec8a34`).
const MINUS_FRAC_PI_2: f32 = f32::from_bits(0xbfc9_0fdb);
/// π/4 (`DAT_142eb6e44`).
const FRAC_PI_4: f32 = f32::from_bits(0x3f49_0fdb);
/// −π/4 (`DAT_142ec8a30`).
const MINUS_FRAC_PI_4: f32 = f32::from_bits(0xbf49_0fdb);
/// 3π/4 (`DAT_142ec8a24`).
const THREE_FRAC_PI_4: f32 = f32::from_bits(0x4016_cbe4);
/// −3π/4 (`DAT_142ec8a38`).
const MINUS_THREE_FRAC_PI_4: f32 = f32::from_bits(0xc016_cbe4);
/// −0 (`DAT_142ec8a28`).
const MINUS_ZERO: f32 = f32::from_bits(0x8000_0000);
/// π's next bits below [`PI`] (`DAT_142ec8a2c`).
const PI_LO: f32 = f32::from_bits(0xb3bb_bd2e);

/// `atan2f(y, x)`: the game's atan2f `0x14099e020` (msun's `e_atan2f.c`), the angle of the point
/// (x, y). A NaN in either gives `y + x`; x = 1 is atanf(y). `m` holds x's sign (bit 1) and y's
/// (bit 0): it picks the quadrant of the zeros, the infinities and of atanf(|y / x|).
#[must_use]
pub fn atan2(y: f32, x: f32) -> f32 {
    if is_nan(x) || is_nan(y) {
        return y + x;
    }
    let hx = x.to_bits();
    if hx == 0x3f80_0000 {
        return atan(y);
    }
    let hy = y.to_bits();
    let m = ((hx & 0xbfff_ffff) | (hy >> 1)) >> 30;
    let ix = hx & 0x7fff_ffff;
    let iy = hy & 0x7fff_ffff;
    if iy == 0 {
        return match m {
            0 | 1 => y,
            2 => PI,
            _ => MINUS_PI,
        };
    }
    let up_or_down = if m & 1 == 0 {
        FRAC_PI_2
    } else {
        MINUS_FRAC_PI_2
    };
    if ix == 0 {
        return up_or_down;
    }
    if ix == 0x7f80_0000 {
        return if iy == 0x7f80_0000 {
            match m {
                0 => FRAC_PI_4,
                1 => MINUS_FRAC_PI_4,
                2 => THREE_FRAC_PI_4,
                _ => MINUS_THREE_FRAC_PI_4,
            }
        } else {
            match m {
                0 => 0.0,
                1 => MINUS_ZERO,
                2 => PI,
                _ => MINUS_PI,
            }
        };
    }
    // |y / x| above 2^26, or y infinite
    if ix + 0x0d00_0000 < iy || iy == 0x7f80_0000 {
        return up_or_down;
    }
    // |y / x| below 2^-26 with x negative: 0, else atanf(|y / x|)
    let z = if m & 2 != 0 && iy + 0x0d00_0000 < ix {
        0.0
    } else {
        atan(fabs(y / x))
    };
    match m {
        0 => z,
        1 => -z,
        2 => PI - (z - PI_LO),
        _ => (z - PI_LO) - PI,
    }
}

/// `COMISS a, b` then `JC`: taken when `a` is less than `b` or either is NaN (unordered).
#[must_use]
pub fn below_or_unordered(a: f32, b: f32) -> bool {
    a < b || a.is_nan() || b.is_nan()
}

/// `COMISS a, b` then `JBE`: taken when `a` is at most `b` or either is NaN (unordered).
#[must_use]
pub fn at_most_or_unordered(a: f32, b: f32) -> bool {
    a <= b || a.is_nan() || b.is_nan()
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

    /// `f` gives the game's bits on each `(input, output)` pair, a NaN's payload included. The
    /// outputs are build 158041's own routines, run on these inputs in unicorn.
    fn assert_bits(name: &str, f: impl Fn(f32) -> f32, cases: &[(u32, u32)]) {
        for &(x, game) in cases {
            let got = f(f32::from_bits(x)).to_bits();
            assert_eq!(got, game, "{name}({x:08x}): {got:08x}, game {game:08x}");
        }
    }

    /// acosf `0x14099dc60` at its branches' bounds (2^-26, 0.5, 1), zeros, subnormals, infinities
    /// and NaNs: |x| above 1 is the invalid 0/0, a NaN comes back quieted.
    #[test]
    fn acos_is_the_games_acosf_at_its_bounds() {
        assert_bits(
            "acos",
            acos,
            &[
                (0x0000_0000, 0x3fc9_0fda),
                (0x8000_0000, 0x3fc9_0fda),
                (0x0000_0001, 0x3fc9_0fda),
                (0x8000_0001, 0x3fc9_0fda),
                (0x007f_ffff, 0x3fc9_0fda),
                (0x807f_ffff, 0x3fc9_0fda),
                (0x0080_0000, 0x3fc9_0fda),
                (0x8080_0000, 0x3fc9_0fda),
                (0x7f7f_ffff, 0xffc0_0000),
                (0xff7f_ffff, 0xffc0_0000),
                (0x7f80_0000, 0xffc0_0000),
                (0xff80_0000, 0xffc0_0000),
                (0x7fc0_0000, 0x7fc0_0000),
                (0xffc0_0000, 0xffc0_0000),
                (0x7f80_0001, 0x7fc0_0001),
                (0xff81_2345, 0xffc1_2345),
                (0x3280_0000, 0x3fc9_0fda),
                (0x3280_0001, 0x3fc9_0fdb),
                (0xb280_0000, 0x3fc9_0fda),
                (0xb280_0001, 0x3fc9_0fdb),
                (0x3eff_ffff, 0x3f86_0a92),
                (0x3f00_0000, 0x3f86_0a92),
                (0xbeff_ffff, 0x4006_0a92),
                (0xbf00_0000, 0x4006_0a92),
                (0x3f7f_ffff, 0x39b5_04f3),
                (0x3f80_0000, 0x0000_0000),
                (0x3f80_0001, 0xffc0_0000),
                (0xbf7f_ffff, 0x4049_0a32),
                (0xbf80_0000, 0x4049_0fda),
                (0xbf80_0001, 0xffc0_0000),
                (0x4000_0000, 0xffc0_0000),
                (0x3f35_04f3, 0x3f49_0fdb),
                (0xbf35_04f3, 0x4016_cbe4),
            ],
        );
    }

    /// atanf `0x14099e2f0` at its branches' bounds (2^-12, 7/16, 11/16, 19/16, 39/16, 2^26),
    /// zeros, subnormals, infinities and NaNs: a NaN comes back as it is, a signalling one too.
    #[test]
    fn atan_is_the_games_atanf_at_its_bounds() {
        assert_bits(
            "atan",
            atan,
            &[
                (0x0000_0000, 0x0000_0000),
                (0x8000_0000, 0x8000_0000),
                (0x0000_0001, 0x0000_0001),
                (0x8000_0001, 0x8000_0001),
                (0x007f_ffff, 0x007f_ffff),
                (0x807f_ffff, 0x807f_ffff),
                (0x0080_0000, 0x0080_0000),
                (0x8080_0000, 0x8080_0000),
                (0x7f7f_ffff, 0x3fc9_0fda),
                (0xff7f_ffff, 0xbfc9_0fda),
                (0x7f80_0000, 0x3fc9_0fda),
                (0xff80_0000, 0xbfc9_0fda),
                (0x7fc0_0000, 0x7fc0_0000),
                (0xffc0_0000, 0xffc0_0000),
                (0x7f80_0001, 0x7f80_0001),
                (0xff81_2345, 0xff81_2345),
                (0x397f_ffff, 0x397f_ffff),
                (0x3980_0000, 0x3980_0000),
                (0xb980_0000, 0xb980_0000),
                (0x3edf_ffff, 0x3ed3_2775),
                (0x3ee0_0000, 0x3ed3_2776),
                (0xbee0_0000, 0xbed3_2776),
                (0x3f2f_ffff, 0x3f1a_2f80),
                (0x3f30_0000, 0x3f1a_2f81),
                (0x3f97_ffff, 0x3f5e_f386),
                (0x3f98_0000, 0x3f5e_f387),
                (0xbf98_0000, 0xbf5e_f387),
                (0x401b_ffff, 0x3f97_3ab9),
                (0x401c_0000, 0x3f97_3ab9),
                (0xc01c_0000, 0xbf97_3ab9),
                (0x4c7f_ffff, 0x3fc9_0fdb),
                (0x4c80_0000, 0x3fc9_0fda),
                (0xcc80_0000, 0xbfc9_0fda),
                (0x3f80_0000, 0x3f49_0fdb),
                (0xbf80_0000, 0xbf49_0fdb),
                (0x3f00_0000, 0x3eed_6338),
                (0x4049_0fdb, 0x3fa1_9dc5),
            ],
        );
    }

    /// atan2f `0x14099e020` on every pair of ±0, ±1, 1/2, −3, the smallest subnormal, 1e20, ±∞
    /// and a NaN: the game's outputs (unicorn).
    #[test]
    fn atan2_is_the_games_atan2f_on_special_pairs() {
        for (y, x, game) in [
            (0x0000_0000, 0x0000_0000, 0x0000_0000),
            (0x0000_0000, 0x8000_0000, 0x4049_0fdb),
            (0x0000_0000, 0x3f80_0000, 0x0000_0000),
            (0x0000_0000, 0xbf80_0000, 0x4049_0fdb),
            (0x0000_0000, 0x3f00_0000, 0x0000_0000),
            (0x0000_0000, 0xc040_0000, 0x4049_0fdb),
            (0x0000_0000, 0x0000_0001, 0x0000_0000),
            (0x0000_0000, 0x60ad_78ec, 0x0000_0000),
            (0x0000_0000, 0x7f80_0000, 0x0000_0000),
            (0x0000_0000, 0xff80_0000, 0x4049_0fdb),
            (0x0000_0000, 0x7fc0_0001, 0x7fc0_0001),
            (0x8000_0000, 0x0000_0000, 0x8000_0000),
            (0x8000_0000, 0x8000_0000, 0xc049_0fdb),
            (0x8000_0000, 0x3f80_0000, 0x8000_0000),
            (0x8000_0000, 0xbf80_0000, 0xc049_0fdb),
            (0x8000_0000, 0x3f00_0000, 0x8000_0000),
            (0x8000_0000, 0xc040_0000, 0xc049_0fdb),
            (0x8000_0000, 0x0000_0001, 0x8000_0000),
            (0x8000_0000, 0x60ad_78ec, 0x8000_0000),
            (0x8000_0000, 0x7f80_0000, 0x8000_0000),
            (0x8000_0000, 0xff80_0000, 0xc049_0fdb),
            (0x8000_0000, 0x7fc0_0001, 0x7fc0_0001),
            (0x3f80_0000, 0x0000_0000, 0x3fc9_0fdb),
            (0x3f80_0000, 0x8000_0000, 0x3fc9_0fdb),
            (0x3f80_0000, 0x3f80_0000, 0x3f49_0fdb),
            (0x3f80_0000, 0xbf80_0000, 0x4016_cbe4),
            (0x3f80_0000, 0x3f00_0000, 0x3f8d_b70d),
            (0x3f80_0000, 0xc040_0000, 0x4034_784b),
            (0x3f80_0000, 0x0000_0001, 0x3fc9_0fdb),
            (0x3f80_0000, 0x60ad_78ec, 0x1e3c_e508),
            (0x3f80_0000, 0x7f80_0000, 0x0000_0000),
            (0x3f80_0000, 0xff80_0000, 0x4049_0fdb),
            (0x3f80_0000, 0x7fc0_0001, 0x7fc0_0001),
            (0xbf80_0000, 0x0000_0000, 0xbfc9_0fdb),
            (0xbf80_0000, 0x8000_0000, 0xbfc9_0fdb),
            (0xbf80_0000, 0x3f80_0000, 0xbf49_0fdb),
            (0xbf80_0000, 0xbf80_0000, 0xc016_cbe4),
            (0xbf80_0000, 0x3f00_0000, 0xbf8d_b70d),
            (0xbf80_0000, 0xc040_0000, 0xc034_784b),
            (0xbf80_0000, 0x0000_0001, 0xbfc9_0fdb),
            (0xbf80_0000, 0x60ad_78ec, 0x9e3c_e508),
            (0xbf80_0000, 0x7f80_0000, 0x8000_0000),
            (0xbf80_0000, 0xff80_0000, 0xc049_0fdb),
            (0xbf80_0000, 0x7fc0_0001, 0x7fc0_0001),
            (0x3f00_0000, 0x0000_0000, 0x3fc9_0fdb),
            (0x3f00_0000, 0x8000_0000, 0x3fc9_0fdb),
            (0x3f00_0000, 0x3f80_0000, 0x3eed_6338),
            (0x3f00_0000, 0xbf80_0000, 0x402b_6374),
            (0x3f00_0000, 0x3f00_0000, 0x3f49_0fdb),
            (0x3f00_0000, 0xc040_0000, 0x403e_7e0f),
            (0x3f00_0000, 0x0000_0001, 0x3fc9_0fdb),
            (0x3f00_0000, 0x60ad_78ec, 0x1dbc_e508),
            (0x3f00_0000, 0x7f80_0000, 0x0000_0000),
            (0x3f00_0000, 0xff80_0000, 0x4049_0fdb),
            (0x3f00_0000, 0x7fc0_0001, 0x7fc0_0001),
            (0xc040_0000, 0x0000_0000, 0xbfc9_0fdb),
            (0xc040_0000, 0x8000_0000, 0xbfc9_0fdb),
            (0xc040_0000, 0x3f80_0000, 0xbf9f_e0bb),
            (0xc040_0000, 0xbf80_0000, 0xbff2_3efa),
            (0xc040_0000, 0x3f00_0000, 0xbfb3_ec43),
            (0xc040_0000, 0xc040_0000, 0xc016_cbe4),
            (0xc040_0000, 0x0000_0001, 0xbfc9_0fdb),
            (0xc040_0000, 0x60ad_78ec, 0x9f0d_abc6),
            (0xc040_0000, 0x7f80_0000, 0x8000_0000),
            (0xc040_0000, 0xff80_0000, 0xc049_0fdb),
            (0xc040_0000, 0x7fc0_0001, 0x7fc0_0001),
            (0x0000_0001, 0x0000_0000, 0x3fc9_0fdb),
            (0x0000_0001, 0x8000_0000, 0x3fc9_0fdb),
            (0x0000_0001, 0x3f80_0000, 0x0000_0001),
            (0x0000_0001, 0xbf80_0000, 0x4049_0fdb),
            (0x0000_0001, 0x3f00_0000, 0x0000_0002),
            (0x0000_0001, 0xc040_0000, 0x4049_0fdb),
            (0x0000_0001, 0x0000_0001, 0x3f49_0fdb),
            (0x0000_0001, 0x60ad_78ec, 0x0000_0000),
            (0x0000_0001, 0x7f80_0000, 0x0000_0000),
            (0x0000_0001, 0xff80_0000, 0x4049_0fdb),
            (0x0000_0001, 0x7fc0_0001, 0x7fc0_0001),
            (0x60ad_78ec, 0x0000_0000, 0x3fc9_0fdb),
            (0x60ad_78ec, 0x8000_0000, 0x3fc9_0fdb),
            (0x60ad_78ec, 0x3f80_0000, 0x3fc9_0fda),
            (0x60ad_78ec, 0xbf80_0000, 0x3fc9_0fdb),
            (0x60ad_78ec, 0x3f00_0000, 0x3fc9_0fdb),
            (0x60ad_78ec, 0xc040_0000, 0x3fc9_0fdb),
            (0x60ad_78ec, 0x0000_0001, 0x3fc9_0fdb),
            (0x60ad_78ec, 0x60ad_78ec, 0x3f49_0fdb),
            (0x60ad_78ec, 0x7f80_0000, 0x0000_0000),
            (0x60ad_78ec, 0xff80_0000, 0x4049_0fdb),
            (0x60ad_78ec, 0x7fc0_0001, 0x7fc0_0001),
            (0x7f80_0000, 0x0000_0000, 0x3fc9_0fdb),
            (0x7f80_0000, 0x8000_0000, 0x3fc9_0fdb),
            (0x7f80_0000, 0x3f80_0000, 0x3fc9_0fda),
            (0x7f80_0000, 0xbf80_0000, 0x3fc9_0fdb),
            (0x7f80_0000, 0x3f00_0000, 0x3fc9_0fdb),
            (0x7f80_0000, 0xc040_0000, 0x3fc9_0fdb),
            (0x7f80_0000, 0x0000_0001, 0x3fc9_0fdb),
            (0x7f80_0000, 0x60ad_78ec, 0x3fc9_0fdb),
            (0x7f80_0000, 0x7f80_0000, 0x3f49_0fdb),
            (0x7f80_0000, 0xff80_0000, 0x4016_cbe4),
            (0x7f80_0000, 0x7fc0_0001, 0x7fc0_0001),
            (0xff80_0000, 0x0000_0000, 0xbfc9_0fdb),
            (0xff80_0000, 0x8000_0000, 0xbfc9_0fdb),
            (0xff80_0000, 0x3f80_0000, 0xbfc9_0fda),
            (0xff80_0000, 0xbf80_0000, 0xbfc9_0fdb),
            (0xff80_0000, 0x3f00_0000, 0xbfc9_0fdb),
            (0xff80_0000, 0xc040_0000, 0xbfc9_0fdb),
            (0xff80_0000, 0x0000_0001, 0xbfc9_0fdb),
            (0xff80_0000, 0x60ad_78ec, 0xbfc9_0fdb),
            (0xff80_0000, 0x7f80_0000, 0xbf49_0fdb),
            (0xff80_0000, 0xff80_0000, 0xc016_cbe4),
            (0xff80_0000, 0x7fc0_0001, 0x7fc0_0001),
            (0x7fc0_0001, 0x0000_0000, 0x7fc0_0001),
            (0x7fc0_0001, 0x8000_0000, 0x7fc0_0001),
            (0x7fc0_0001, 0x3f80_0000, 0x7fc0_0001),
            (0x7fc0_0001, 0xbf80_0000, 0x7fc0_0001),
            (0x7fc0_0001, 0x3f00_0000, 0x7fc0_0001),
            (0x7fc0_0001, 0xc040_0000, 0x7fc0_0001),
            (0x7fc0_0001, 0x0000_0001, 0x7fc0_0001),
            (0x7fc0_0001, 0x60ad_78ec, 0x7fc0_0001),
            (0x7fc0_0001, 0x7f80_0000, 0x7fc0_0001),
            (0x7fc0_0001, 0xff80_0000, 0x7fc0_0001),
            (0x7fc0_0001, 0x7fc0_0001, 0x7fc0_0001),
        ] {
            let got = atan2(f32::from_bits(y), f32::from_bits(x)).to_bits();
            assert_eq!(
                got, game,
                "atan2({y:08x}, {x:08x}): {got:08x}, game {game:08x}"
            );
        }
    }

    /// A NaN in either argument gives `y + x` (`ADDSS y, x`): SSE returns the first operand's NaN
    /// (y's) when both are NaNs, quieted. (Unicorn's softfloat picks the larger payload there, as
    /// the x87 does, so these outputs follow Intel's rule, not the emulator.)
    #[test]
    fn atan2_of_a_nan_is_y_plus_x() {
        for (y, x, game) in [
            (0xffc0_0002, 0x7fc0_0003, 0xffc0_0002),
            (0x7f80_0001, 0xff80_0002, 0x7fc0_0001),
            (0x7f80_0001, 0x3f80_0000, 0x7fc0_0001),
            (0x3f80_0000, 0x7f80_0001, 0x7fc0_0001),
        ] {
            let got = atan2(f32::from_bits(y), f32::from_bits(x)).to_bits();
            assert_eq!(
                got, game,
                "atan2({y:08x}, {x:08x}): {got:08x}, game {game:08x}"
            );
        }
    }

    /// fabsf `0x14099e7b0` clears the sign bit and nothing else, a NaN's payload kept.
    #[test]
    fn fabs_clears_the_sign_bit() {
        assert_bits(
            "fabs",
            fabs,
            &[
                (0x0000_0000, 0x0000_0000),
                (0x8000_0000, 0x0000_0000),
                (0x8000_0001, 0x0000_0001),
                (0x807f_ffff, 0x007f_ffff),
                (0xff7f_ffff, 0x7f7f_ffff),
                (0xff80_0000, 0x7f80_0000),
                (0xffc0_0000, 0x7fc0_0000),
                (0x7f80_0001, 0x7f80_0001),
                (0xff81_2345, 0x7f81_2345),
            ],
        );
    }

    /// `0x14099e870` (isnan through the double): true for every NaN, quiet or signalling, of
    /// either sign; false for the infinities and the largest finite value.
    #[test]
    fn is_nan_is_true_for_every_nan_only() {
        for (x, game) in [
            (0x0000_0000, false),
            (0x8000_0001, false),
            (0x007f_ffff, false),
            (0x7f7f_ffff, false),
            (0xff7f_ffff, false),
            (0x7f80_0000, false),
            (0xff80_0000, false),
            (0x7fc0_0000, true),
            (0xffc0_0000, true),
            (0x7f80_0001, true),
            (0xff80_0001, true),
            (0xff81_2345, true),
            (0x7fff_ffff, true),
            (0xffff_ffff, true),
        ] {
            assert_eq!(is_nan(f32::from_bits(x)), game, "{x:08x}");
        }
    }

    #[test]
    fn a_jc_after_comiss_is_taken_below_and_unordered() {
        assert!(below_or_unordered(1.0, 2.0));
        assert!(!below_or_unordered(2.0, 2.0) && !below_or_unordered(3.0, 2.0));
        assert!(below_or_unordered(f32::NAN, 2.0) && below_or_unordered(2.0, f32::NAN));
        assert!(at_most_or_unordered(2.0, 2.0) && !at_most_or_unordered(3.0, 2.0));
        assert!(at_most_or_unordered(f32::NAN, 2.0) && at_most_or_unordered(2.0, f32::NAN));
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

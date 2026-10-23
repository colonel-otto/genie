//! The game's f32 math, bit for bit (checked by `tests/crt_math.rs` against build 158041's own
//! routines on recorded inputs).
//!
//! The game links musl's math routines, which take FreeBSD msun's sources: acosf `0x14099dc60`,
//! atan2f `0x14099e020`, cosf `0x14099e4d0`, sinf `0x14099ed60` and the kernels and argument
//! reduction they call. Each is ported here from its machine code: its constants (read from the
//! image), its operation order, its branches and its integer bit tests. Its sqrtf `0x14099ef70` is
//! an integer Newton iteration that rounds correctly, so IEEE's square root gives the same bits.
//! Its fmodf is the CRT's (an import): the exact remainder, ported as musl's ([`fmod`]).
//!
//! A NaN result's bits are the game's on every host: SSE gives an invalid operation on operands
//! that are not NaN its default NaN with the sign set (`0xffc00000`), where ARM gives it without
//! the sign and WebAssembly either way, and it hands on the first NaN operand quieted, where ARM
//! prefers a signalling one. A routine whose result is a NaN therefore makes its bits itself
//! ([`sse_nan`]).

/// 1 (`DAT_14330eb8c`).
const ONE: f32 = f32::from_bits(0x3f80_0000);
/// 1/2 (`DAT_142e7f384`).
const HALF: f32 = f32::from_bits(0x3f00_0000);

/// SSE's default NaN in f32 (Intel's quiet-NaN floating-point indefinite): an invalid operation's
/// result on operands that are not NaN.
const INDEFINITE: u32 = 0xffc0_0000;
/// SSE's default NaN in f64.
const INDEFINITE_F64: u64 = 0xfff8_0000_0000_0000;
/// The quiet bit of an f32 NaN.
const QUIET: u32 = 0x0040_0000;
/// The quiet bit of an f64 NaN.
const QUIET_F64: u64 = 0x0008_0000_0000_0000;

/// `result` as SSE gives it: a NaN result is the first NaN of `operands` (in the game's operand
/// order), quieted, else the default NaN ([`INDEFINITE`]).
fn sse_nan(result: f32, operands: &[f32]) -> f32 {
    if !result.is_nan() {
        return result;
    }
    let nan = operands.iter().find(|o| o.is_nan());
    f32::from_bits(nan.map_or(INDEFINITE, |o| o.to_bits() | QUIET))
}

/// [`sse_nan`] in f64.
fn sse_nan_f64(result: f64, operands: &[f64]) -> f64 {
    if !result.is_nan() {
        return result;
    }
    let nan = operands.iter().find(|o| o.is_nan());
    f64::from_bits(nan.map_or(INDEFINITE_F64, |o| o.to_bits() | QUIET_F64))
}

/// An f64 rounded to f32 as `CVTSD2SS` does: a NaN keeps its sign and the top 23 bits of its
/// payload, quieted.
const fn narrow(x: f64) -> f32 {
    if !x.is_nan() {
        return x as f32;
    }
    let b = x.to_bits();
    f32::from_bits(
        ((b >> 32) as u32 & 0x8000_0000) | 0x7f80_0000 | ((b >> 29) as u32 & 0x007f_ffff) | QUIET,
    )
}

/// An f32 widened to f64 as `CVTSS2SD` does: a NaN keeps its sign and payload, quieted.
fn widen(x: f32) -> f64 {
    if !x.is_nan() {
        return f64::from(x);
    }
    let b = u64::from(x.to_bits());
    f64::from_bits(
        ((b & 0x8000_0000) << 32) | 0x7ff0_0000_0000_0000 | ((b & 0x007f_ffff) << 29) | QUIET_F64,
    )
}

/// The square root, as the game's (correctly rounded); below 0 the default NaN.
#[must_use]
pub fn sqrt(x: f32) -> f32 {
    sse_nan(x.sqrt(), &[x])
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
            return sse_nan(0.0 / (x - x), &[x]);
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

/// 1 in double (`DAT_142ea7c58`).
const ONE_F64: f64 = f64::from_bits(0x3ff0_0000_0000_0000);
/// −1 in double (`DAT_14330ec28`).
const MINUS_ONE_F64: f64 = f64::from_bits(0xbff0_0000_0000_0000);
/// 1/2 in double (`DAT_142ea7c50`).
const HALF_F64: f64 = f64::from_bits(0x3fe0_0000_0000_0000);
/// π/2 in double (`DAT_142ec8a10`): cosf's and sinf's step to the next quadrant.
const FRAC_PI_2_F64: f64 = f64::from_bits(0x3ff9_21fb_5444_2d18);
/// π in double (`DAT_142ec8a78`).
const PI_F64: f64 = f64::from_bits(0x4009_21fb_5444_2d18);
/// 3π/2 in double (`DAT_142ec8a80`).
const THREE_FRAC_PI_2_F64: f64 = f64::from_bits(0x4012_d97c_7f33_21d2);
/// 2π in double (`DAT_142ec8a88`).
const TWO_PI_F64: f64 = f64::from_bits(0x4019_21fb_5444_2d18);

/// The sine kernel's series x + s (S2 z − S1) + s w (S4 z − S3) with z = x², w = z², s = z x:
/// S1, subtracted: msun's S1 negated (`DAT_142ec89a8`).
const SIN_S1: f64 = f64::from_bits(0x3fc5_5555_54cb_ac77);
/// The sine kernel's S2 (`DAT_142ec89a0`).
const SIN_S2: f64 = f64::from_bits(0x3f81_1110_896e_fbb2);
/// The sine kernel's S3, subtracted: msun's S3 negated (`DAT_142ec8998`).
const SIN_S3: f64 = f64::from_bits(0x3f2a_00f9_e2ca_e774);
/// The sine kernel's S4 (`DAT_142ec8990`).
const SIN_S4: f64 = f64::from_bits(0x3ec6_cd87_8c3b_46a7);
/// The cosine kernel's series (1 − C0 z) + C1 w + (C3 z − C2) w z with z = x², w = z²: C0,
/// subtracted: msun's C0 negated (`DAT_142ec87c8`).
const COS_C0: f64 = f64::from_bits(0x3fdf_ffff_fd0c_5e81);
/// The cosine kernel's C1 (`DAT_142ec87c0`).
const COS_C1: f64 = f64::from_bits(0x3fa5_5553_e105_3a42);
/// The cosine kernel's C2, subtracted: msun's C2 negated (`DAT_142ec87b8`).
const COS_C2: f64 = f64::from_bits(0x3f56_c087_e80f_1e27);
/// The cosine kernel's C3 (`DAT_142ec87b0`).
const COS_C3: f64 = f64::from_bits(0x3ef9_9342_e0ee_5069);

/// `0x14099db70` (msun's `__kernel_sindf`): the sine of a double up to about π/4 in size, rounded
/// to f32 (`CVTSD2SS`).
fn k_sin(x: f64) -> f32 {
    let z = x * x;
    let w = z * z;
    let r = z * SIN_S4 - SIN_S3;
    let s = z * x;
    narrow(sse_nan_f64(
        w * s * r + ((z * SIN_S2 - SIN_S1) * s + x),
        &[x],
    ))
}

/// `0x14099cad0` (msun's `__kernel_cosdf`): the cosine of a double up to about π/4 in size,
/// rounded to f32.
fn k_cos(x: f64) -> f32 {
    let z = x * x;
    let w = z * z;
    let r = z * COS_C3 - COS_C2;
    narrow(sse_nan_f64(
        r * (w * z) + ((ONE_F64 - z * COS_C0) + w * COS_C1),
        &[x],
    ))
}

/// floor's 2^52, added and taken off to round away the fraction (`DAT_142ec8bf0`).
const FLOOR_TO_INT: f64 = f64::from_bits(0x4330_0000_0000_0000);

/// floor `0x14099e7d0` (musl's): `x` from 2^52 up, zeros and the non-finite as they are; below 1
/// in size 0 or −1 by the sign; else `x` plus the distance to its rounded neighbour, less 1 when
/// that rounded up.
fn floor(x: f64) -> f64 {
    let u = x.to_bits();
    let e = (u >> 52) as u32 & 0x7ff;
    if e >= 0x433 || x == 0.0 {
        return x;
    }
    let negative = u >> 63 != 0;
    let y = if negative {
        x - FLOOR_TO_INT + FLOOR_TO_INT - x
    } else {
        x + FLOOR_TO_INT - FLOOR_TO_INT - x
    };
    if e <= 0x3fe {
        return if negative { -1.0 } else { 0.0 };
    }
    if y > 0.0 { y + x - ONE_F64 } else { y + x }
}

/// 2^1023 (`DAT_142ec8e70`).
const TWO_POW_1023: f64 = f64::from_bits(0x7fe0_0000_0000_0000);
/// 2^-969, scalbn's step down: 2^-1022 · 2^53 (`DAT_142ec8e68`).
const TWO_POW_MINUS_969: f64 = f64::from_bits(0x0360_0000_0000_0000);

/// scalbn `0x14099ecd0` (musl's): `x` · 2^n, the exponent brought into range by at most two steps
/// of 2^1023 or 2^-969 (so a subnormal result rounds once), then capped.
fn scalbn(x: f64, n: i32) -> f64 {
    let mut y = x;
    let mut n = n;
    if n > 1023 {
        y *= TWO_POW_1023;
        n -= 1023;
        if n > 1023 {
            y *= TWO_POW_1023;
            n -= 1023;
            if n > 1023 {
                n = 1023;
            }
        }
    } else if n < -1022 {
        y *= TWO_POW_MINUS_969;
        n += 969;
        if n < -1022 {
            y *= TWO_POW_MINUS_969;
            n += 969;
            if n < -1022 {
                n = -1022;
            }
        }
    }
    f64::from_bits(((0x3ff + n) as u64) << 52) * y
}

/// The chunks' 2^-24 (`DAT_142ec8940`).
const TWO_POW_MINUS_24: f64 = f64::from_bits(0x3e70_0000_0000_0000);
/// The chunks' 2^24 (`DAT_142ec8958`).
const TWO_POW_24: f64 = f64::from_bits(0x4170_0000_0000_0000);
/// 1/8 (`DAT_142ec8948`).
const EIGHTH: f64 = f64::from_bits(0x3fc0_0000_0000_0000);
/// 8 (`DAT_142ec8950`).
const EIGHT: f64 = f64::from_bits(0x4020_0000_0000_0000);
/// The terms per precision (`init_jk`, the table at `0x142ec87e0`).
const INIT_JK: [usize; 4] = [3, 4, 4, 6];
/// 2/π in 24-bit chunks (`ipio2`, the table at `0x142ec87f0`).
const IPIO2: [i32; 66] = [
    0xa2_f983, 0x6e_4e44, 0x15_29fc, 0x27_57d1, 0xf5_34dd, 0xc0_db62, 0x95_993c, 0x43_9041,
    0xfe_5163, 0xab_debb, 0xc5_61b7, 0x24_6e3a, 0x42_4dd2, 0xe0_0649, 0x2e_ea09, 0xd1_921c,
    0xfe_1deb, 0x1c_b129, 0xa7_3ee8, 0x82_35f5, 0x2e_bb44, 0x84_e99c, 0x70_26b4, 0x5f_7e41,
    0x39_91d6, 0x39_8353, 0x39_f49c, 0x84_5f8b, 0xbd_f928, 0x3b_1ff8, 0x97_ffde, 0x05_980f,
    0xef_2f11, 0x8b_5a0a, 0x6d_1f6d, 0x36_7ecf, 0x27_cb09, 0xb7_4f46, 0x3f_669e, 0x5f_ea2d,
    0x75_27ba, 0xc7_ebe5, 0xf1_7b3d, 0x07_39f7, 0x8a_5292, 0xea_6bfb, 0x5f_b11f, 0x8d_5d08,
    0x56_0330, 0x46_fc7b, 0x6b_abf0, 0xcf_bc20, 0x9a_f436, 0x1d_a9e3, 0x91_615e, 0xe6_1b08,
    0x65_9985, 0x5f_14a0, 0x68_408d, 0xff_d880, 0x4d_7327, 0x31_0606, 0x15_56ca, 0x73_a8c9,
    0x60_e27b, 0xc0_8c6b,
];
/// π/2's first 24 bits (`DAT_142ec8900`).
const PIO2_0: f64 = f64::from_bits(0x3ff9_21fb_4000_0000);
/// π/2's next 24 bits (`DAT_142ec8908`).
const PIO2_1: f64 = f64::from_bits(0x3e74_442d_0000_0000);
/// π/2's third 24 bits (`DAT_142ec8910`).
const PIO2_2: f64 = f64::from_bits(0x3cf8_4698_8000_0000);
/// π/2's fourth 24 bits (`DAT_142ec8918`).
const PIO2_3: f64 = f64::from_bits(0x3b78_cc51_6000_0000);
/// π/2's fifth 24 bits (`DAT_142ec8920`).
const PIO2_4: f64 = f64::from_bits(0x39f0_1b83_8000_0000);
/// π/2's sixth 24 bits (`DAT_142ec8928`).
const PIO2_5: f64 = f64::from_bits(0x387a_2520_4000_0000);
/// π/2's seventh 24 bits (`DAT_142ec8930`).
const PIO2_6: f64 = f64::from_bits(0x36e3_8222_8000_0000);
/// π/2's eighth 24 bits (`DAT_142ec8938`).
const PIO2_7: f64 = f64::from_bits(0x3569_f31d_0000_0000);
/// π/2 in 24-bit pieces (`PIo2`, the table at `0x142ec8900`).
const PIO2: [f64; 8] = [
    PIO2_0, PIO2_1, PIO2_2, PIO2_3, PIO2_4, PIO2_5, PIO2_6, PIO2_7,
];

/// `0x14099cbc0` (msun's `__kernel_rem_pio2`): `x` modulo π/2 for a huge argument, by its bits
/// against 2/π's (Payne and Hanek). `x` holds the argument's 24-bit chunks (at most 3, the first
/// scaled by 2^`e0`); `prec` (0 to 3) picks how many doubles carry the remainder. Returns the
/// quadrant count's low 3 bits and the remainder's pieces (the unused ones 0).
fn rem_pio2_large(x: &[f64], e0: i32, prec: usize) -> (i32, [f64; 3]) {
    let jk = INIT_JK[prec];
    let jp = jk;
    let jx = x.len() - 1;
    let jv = ((e0 - 3) / 24).max(0) as usize;
    let mut q0 = e0 - 24 * (jv as i32 + 1);
    let mut f = [0.0; 20];
    let mut q = [0.0; 20];
    let mut fq = [0.0; 20];
    let mut iq = [0_i32; 20];
    let mut y = [0.0; 3];

    // f[0..=jx + jk]: 2/π's chunks from jv − jx on, 0 before the table
    for (i, f) in f.iter_mut().enumerate().take(jx + jk + 1) {
        let j = (jv + i).checked_sub(jx);
        *f = j.map_or(0.0, |j| f64::from(IPIO2[j]));
    }
    // q[0..=jk]: the products x · f
    for i in 0..=jk {
        let mut fw = 0.0;
        for j in 0..=jx {
            fw += x[j] * f[jx + i - j];
        }
        q[i] = fw;
    }

    let mut jz = jk;
    let (mut n, mut ih, mut z);
    loop {
        // q[] into the integer chunks iq[], highest last
        z = q[jz];
        let mut j = jz;
        let mut i = 0;
        while j > 0 {
            let fw = f64::from((TWO_POW_MINUS_24 * z) as i32);
            iq[i] = (z - TWO_POW_24 * fw) as i32;
            z = q[j - 1] + fw;
            i += 1;
            j -= 1;
        }

        // the quadrant count n, z its fraction
        z = scalbn(z, q0);
        z -= EIGHT * floor(z * EIGHTH);
        n = z as i32;
        z -= f64::from(n);
        ih = 0;
        if q0 > 0 {
            // iq[jz − 1] holds n's low bits
            let i = iq[jz - 1] >> (24 - q0);
            n += i;
            iq[jz - 1] -= i << (24 - q0);
            ih = iq[jz - 1] >> (23 - q0);
        } else if q0 == 0 {
            ih = iq[jz - 1] >> 23;
        } else if z >= HALF_F64 {
            ih = 2;
        }

        if ih > 0 {
            // the fraction above 1/2: take 1 − q
            n += 1;
            let mut carry = false;
            for chunk in &mut iq[..jz] {
                if carry {
                    *chunk = 0xff_ffff - *chunk;
                } else if *chunk != 0 {
                    carry = true;
                    *chunk = 0x100_0000 - *chunk;
                }
            }
            match q0 {
                1 => iq[jz - 1] &= 0x7f_ffff,
                2 => iq[jz - 1] &= 0x3f_ffff,
                _ => {}
            }
            if ih == 2 {
                z = ONE_F64 - z;
                if carry {
                    z -= scalbn(ONE_F64, q0);
                }
            }
        }

        // all of the fraction's chunks 0: take more terms of 2/π and compute again
        if z == 0.0 && iq[jk..jz].iter().fold(0, |j, &chunk| j | chunk) == 0 {
            let mut k = 1;
            while iq[jk - k] == 0 {
                k += 1;
            }
            for i in jz + 1..=jz + k {
                f[jx + i] = f64::from(IPIO2[jv + i]);
                let mut fw = 0.0;
                for j in 0..=jx {
                    fw += x[j] * f[jx + i - j];
                }
                q[i] = fw;
            }
            jz += k;
            continue;
        }
        break;
    }

    // chop off the zero chunks, or split z into 24-bit ones
    if z == 0.0 {
        jz -= 1;
        q0 -= 24;
        while iq[jz] == 0 {
            jz -= 1;
            q0 -= 24;
        }
    } else {
        z = scalbn(z, -q0);
        if z >= TWO_POW_24 {
            let fw = f64::from((TWO_POW_MINUS_24 * z) as i32);
            iq[jz] = (z - TWO_POW_24 * fw) as i32;
            jz += 1;
            q0 += 24;
            iq[jz] = fw as i32;
        } else {
            iq[jz] = z as i32;
        }
    }

    // the chunks back to doubles
    let mut fw = scalbn(ONE_F64, q0);
    for i in (0..=jz).rev() {
        q[i] = fw * f64::from(iq[i]);
        fw *= TWO_POW_MINUS_24;
    }
    // fq[jz − i] = π/2 · q[i..], at most jp + 1 pieces of π/2
    for i in (0..=jz).rev() {
        let mut fw = 0.0;
        let mut k = 0;
        while k <= jp && k <= jz - i {
            fw += PIO2[k] * q[i + k];
            k += 1;
        }
        fq[jz - i] = fw;
    }

    // fq[] into y[]
    let sign = |v: f64| if ih == 0 { v } else { -v };
    match prec {
        0 => {
            let mut fw = 0.0;
            for i in (0..=jz).rev() {
                fw += fq[i];
            }
            y[0] = sign(fw);
        }
        1 | 2 => {
            let mut fw = 0.0;
            for i in (0..=jz).rev() {
                fw += fq[i];
            }
            y[0] = sign(fw);
            fw = fq[0] - fw;
            for v in &fq[1..=jz] {
                fw += v;
            }
            y[1] = sign(fw);
        }
        _ => {
            for i in (1..=jz).rev() {
                let fw = fq[i - 1] + fq[i];
                fq[i] += fq[i - 1] - fw;
                fq[i - 1] = fw;
            }
            for i in (2..=jz).rev() {
                let fw = fq[i - 1] + fq[i];
                fq[i] += fq[i - 1] - fw;
                fq[i - 1] = fw;
            }
            let mut fw = 0.0;
            for i in (2..=jz).rev() {
                fw += fq[i];
            }
            y = [sign(fq[0]), sign(fq[1]), sign(fw)];
        }
    }
    (n & 7, y)
}

/// 2/π (`DAT_142ec8968`).
const INV_PIO2: f64 = f64::from_bits(0x3fe4_5f30_6dc9_c883);
/// 1.5 · 2^52: added and taken off, it rounds to an integer (`DAT_142ec8980`).
const TO_INT: f64 = f64::from_bits(0x4338_0000_0000_0000);
/// π/2's first 33 bits (`DAT_142ec8978`).
const PIO2_HI_F64: f64 = f64::from_bits(0x3ff9_21fb_5000_0000);
/// π/2 less [`PIO2_HI_F64`] (`DAT_142ec8960`).
const PIO2_TAIL_F64: f64 = f64::from_bits(0x3e51_10b4_611a_6263);
/// π/4 as the float rounds it, in double (`DAT_142ec8970`).
const FRAC_PI_4_F64: f64 = f64::from_bits(0x3fe9_21fb_6000_0000);
/// −π/4 as the float rounds it, in double (`DAT_142ec8988`).
const MINUS_FRAC_PI_4_F64: f64 = f64::from_bits(0xbfe9_21fb_6000_0000);

/// `0x14099da00` (msun's `__rem_pio2f`): `x` less n · π/2, as the count n and a double remainder
/// within about π/4. Below 2^28 · π/2 n is x · 2/π rounded, the remainder taken with π/2 in two
/// parts (and n moved by one past ±π/4); above, [`rem_pio2_large`] reduces x's 24 bits scaled by
/// its exponent; infinities and NaN give `x − x` and 0.
fn rem_pio2(x: f32) -> (i32, f64) {
    let hx = x.to_bits();
    let ix = hx & 0x7fff_ffff;
    if ix < 0x4dc9_0fdb {
        let d = f64::from(x);
        let mut fw = d * INV_PIO2 + TO_INT - TO_INT;
        let mut n = fw as i32;
        let mut y = d - fw * PIO2_HI_F64 - fw * PIO2_TAIL_F64;
        // COMISD −π/4, y then JBE; COMISD y, π/4 then JBE
        if MINUS_FRAC_PI_4_F64 > y {
            fw += MINUS_ONE_F64;
            n -= 1;
            y = d - fw * PIO2_HI_F64 - fw * PIO2_TAIL_F64;
        } else if y > FRAC_PI_4_F64 {
            fw += ONE_F64;
            n += 1;
            y = d - fw * PIO2_HI_F64 - fw * PIO2_TAIL_F64;
        }
        return (n, y);
    }
    if ix >= 0x7f80_0000 {
        #[allow(clippy::eq_op)]
        return (0, widen(sse_nan(x - x, &[x])));
    }
    // x's 24 bits with the exponent 23, scaled by 2^e0
    let e0 = (ix >> 23) as i32 - 150;
    let chunk = f64::from(f32::from_bits(ix - ((e0 as u32) << 23)));
    let (n, y) = rem_pio2_large(&[chunk], e0, 0);
    if hx >> 31 != 0 {
        (-n, -y[0])
    } else {
        (n, y[0])
    }
}

/// The cosine: the game's cosf `0x14099e4d0` (msun's `s_cosf.c`). Up to 9π/4 in size x moves by
/// a multiple of π/2 in double to the sine or cosine kernel; beyond, [`rem_pio2`] reduces it.
/// Below 2^-12 it is 1; infinities and NaN give `x − x`.
#[must_use]
pub fn cos(x: f32) -> f32 {
    let hx = x.to_bits();
    let negative = hx >> 31 != 0;
    let ix = hx & 0x7fff_ffff;
    if ix <= 0x3f49_0fda {
        if ix < 0x3980_0000 {
            // x + 2^120 into a dead store (msun raises inexact): 1
            return ONE;
        }
        return k_cos(f64::from(x));
    }
    if ix <= 0x407b_53d1 {
        let d = f64::from(x);
        if ix > 0x4016_cbe3 {
            return -k_cos(if negative { d + PI_F64 } else { d - PI_F64 });
        }
        return if negative {
            k_sin(d + FRAC_PI_2_F64)
        } else {
            k_sin(FRAC_PI_2_F64 - d)
        };
    }
    if ix <= 0x40e2_31d5 {
        if ix > 0x40af_eddf {
            let d = f64::from(x);
            return k_cos(if negative {
                d + TWO_PI_F64
            } else {
                d - TWO_PI_F64
            });
        }
        return k_sin(f64::from(if negative { -x } else { x }) - THREE_FRAC_PI_2_F64);
    }
    if ix >= 0x7f80_0000 {
        #[allow(clippy::eq_op)]
        return sse_nan(x - x, &[x]);
    }
    let (n, y) = rem_pio2(x);
    match n & 3 {
        0 => k_cos(y),
        1 => k_sin(-y),
        2 => -k_cos(y),
        _ => k_sin(y),
    }
}

/// The sine: the game's sinf `0x14099ed60` (msun's `s_sinf.c`), its branches cosf's. Below 2^-12
/// it is x; infinities and NaN give `x − x`.
#[must_use]
pub fn sin(x: f32) -> f32 {
    let hx = x.to_bits();
    let negative = hx >> 31 != 0;
    let ix = hx & 0x7fff_ffff;
    if ix <= 0x3f49_0fda {
        if ix < 0x3980_0000 {
            // x · 2^-120 or x + 2^120 into a dead store (msun raises underflow or inexact): x
            return x;
        }
        return k_sin(f64::from(x));
    }
    if ix <= 0x407b_53d1 {
        let d = f64::from(x);
        if ix <= 0x4016_cbe3 {
            return if negative {
                -k_cos(d + FRAC_PI_2_F64)
            } else {
                k_cos(d - FRAC_PI_2_F64)
            };
        }
        return k_sin(-(if negative { d + PI_F64 } else { d - PI_F64 }));
    }
    if ix <= 0x40e2_31d5 {
        let d = f64::from(x);
        if ix <= 0x40af_eddf {
            return if negative {
                k_cos(d + THREE_FRAC_PI_2_F64)
            } else {
                -k_cos(d - THREE_FRAC_PI_2_F64)
            };
        }
        return k_sin(if negative {
            d + TWO_PI_F64
        } else {
            d - TWO_PI_F64
        });
    }
    if ix >= 0x7f80_0000 {
        #[allow(clippy::eq_op)]
        return sse_nan(x - x, &[x]);
    }
    let (n, y) = rem_pio2(x);
    match n & 3 {
        0 => k_sin(y),
        1 => k_cos(y),
        2 => k_sin(-y),
        _ => -k_cos(y),
    }
}

/// fmodf: the remainder of x / y with x's sign, exact. The game calls the CRT's
/// (`api-ms-win-crt-math-l1-1-0.dll!fmodf`, through the import thunk `0x142a8e7fa`), whose body is
/// not in the image; an exact remainder has one value, so musl's `fmodf` is ported: a shift and
/// subtract on the significands. A zero or NaN y and a non-finite x give `(x · y) / (x · y)`.
#[must_use]
pub fn fmod(x: f32, y: f32) -> f32 {
    let mut uxi = x.to_bits();
    let mut uyi = y.to_bits();
    let mut ex = (uxi >> 23 & 0xff) as i32;
    let mut ey = (uyi >> 23 & 0xff) as i32;
    let sx = uxi & 0x8000_0000;
    if uyi << 1 == 0 || y.is_nan() || ex == 0xff {
        // the invalid operation's default NaN, or the NaN argument quieted
        let p = x * y;
        #[allow(clippy::eq_op)]
        return sse_nan(p / p, &[x, y]);
    }
    if uxi << 1 <= uyi << 1 {
        // |x| at most |y|: a signed zero when equal, else x
        return if uxi << 1 == uyi << 1 {
            f32::from_bits(sx)
        } else {
            x
        };
    }
    // the significands with their leading bit, a subnormal's shifted up
    if ex == 0 {
        let mut i = uxi << 9;
        while i >> 31 == 0 {
            ex -= 1;
            i <<= 1;
        }
        uxi <<= 1 - ex;
    } else {
        uxi &= u32::MAX >> 9;
        uxi |= 1 << 23;
    }
    if ey == 0 {
        let mut i = uyi << 9;
        while i >> 31 == 0 {
            ey -= 1;
            i <<= 1;
        }
        uyi <<= 1 - ey;
    } else {
        uyi &= u32::MAX >> 9;
        uyi |= 1 << 23;
    }
    // x mod y, one exponent step at a time
    while ex > ey {
        let i = uxi.wrapping_sub(uyi);
        if i >> 31 == 0 {
            if i == 0 {
                return f32::from_bits(sx);
            }
            uxi = i;
        }
        uxi <<= 1;
        ex -= 1;
    }
    let i = uxi.wrapping_sub(uyi);
    if i >> 31 == 0 {
        if i == 0 {
            return f32::from_bits(sx);
        }
        uxi = i;
    }
    while uxi >> 23 == 0 {
        uxi <<= 1;
        ex -= 1;
    }
    // back to a float: normal, or shifted down into a subnormal
    if ex > 0 {
        uxi -= 1 << 23;
        uxi |= (ex as u32) << 23;
    } else {
        uxi >>= 1 - ex;
    }
    f32::from_bits(uxi | sx)
}

/// ln 2's leading bits, whose products with an exponent are exact (fdlibm's `ln2_hi`).
const LN2_HI: f64 = f64::from_bits(0x3fe6_2e42_fee0_0000);
/// ln 2's next bits (fdlibm's `ln2_lo`).
const LN2_LO: f64 = f64::from_bits(0x3dea_39ef_3579_3c76);
/// 2^54, which scales a subnormal up.
const TWO_POW_54: f64 = f64::from_bits(0x4350_0000_0000_0000);
/// The series log(1 + f) = 2s + s·R(s²) with s = f / (2 + f): R's coefficients Lg1 to Lg7.
const LOG_LG: [f64; 7] = [
    f64::from_bits(0x3fe5_5555_5555_5593),
    f64::from_bits(0x3fd9_9999_9997_fa04),
    f64::from_bits(0x3fd2_4924_9422_9359),
    f64::from_bits(0x3fcc_71c5_1d8e_78af),
    f64::from_bits(0x3fc7_4664_96cb_03de),
    f64::from_bits(0x3fc3_9a09_d078_c69f),
    f64::from_bits(0x3fc2_f112_df3e_5244),
];
/// 1/3, the short series' cubic term (`f·f·(1/2 − f/3)`).
const THIRD_F64: f64 = f64::from_bits(0x3fd5_5555_5555_5555);

/// The natural logarithm of a double, fdlibm's `__ieee754_log` (FreeBSD msun's `e_log.c`, below
/// 1 ulp): x = 2^k · (1 + f) with 1 + f in [√2/2, √2), then k·ln 2 plus the series of
/// log(1 + f), each operation in fdlibm's order. Zero gives −∞, a negative x the invalid
/// operation's NaN, +∞ and a NaN themselves (quieted).
fn log_f64(x: f64) -> f64 {
    let bits = x.to_bits();
    let (mut hx, lx) = ((bits >> 32) as i32, bits as u32);
    let mut x = x;
    let mut k = 0;
    if hx < 0x0010_0000 {
        if (hx & 0x7fff_ffff) as u32 | lx == 0 {
            return f64::NEG_INFINITY;
        }
        if hx < 0 {
            #[allow(clippy::eq_op)]
            return sse_nan_f64((x - x) / 0.0, &[x]);
        }
        // a subnormal, scaled up
        k -= 54;
        x *= TWO_POW_54;
        hx = (x.to_bits() >> 32) as i32;
    }
    if hx >= 0x7ff0_0000 {
        return sse_nan_f64(x + x, &[x]);
    }
    k += (hx >> 20) - 1023;
    hx &= 0x000f_ffff;
    let i = (hx + 0x95f64) & 0x0010_0000;
    // x or x/2 normalised into [√2/2, √2)
    let high = (hx | (i ^ 0x3ff0_0000)) as u32;
    x = f64::from_bits((u64::from(high) << 32) | (x.to_bits() & 0xffff_ffff));
    k += i >> 20;
    let f = x - 1.0;
    let dk = f64::from(k);
    if (0x000f_ffff & (2 + hx)) < 3 {
        // |f| below 2^-20
        if f == 0.0 {
            return if k == 0 {
                0.0
            } else {
                dk * LN2_HI + dk * LN2_LO
            };
        }
        let r = f * f * (0.5 - THIRD_F64 * f);
        return if k == 0 {
            f - r
        } else {
            dk * LN2_HI - ((r - dk * LN2_LO) - f)
        };
    }
    let s = f / (2.0 + f);
    let z = s * s;
    let mut i = hx - 0x6147a;
    let w = z * z;
    let j = 0x6b851 - hx;
    let t1 = w * (LOG_LG[1] + w * (LOG_LG[3] + w * LOG_LG[5]));
    let t2 = z * (LOG_LG[0] + w * (LOG_LG[2] + w * (LOG_LG[4] + w * LOG_LG[6])));
    i |= j;
    let r = t2 + t1;
    if i > 0 {
        let hfsq = 0.5 * f * f;
        if k == 0 {
            f - (hfsq - s * (hfsq + r))
        } else {
            dk * LN2_HI - ((hfsq - (s * (hfsq + r) + dk * LN2_LO)) - f)
        }
    } else if k == 0 {
        f - s * (f - r)
    } else {
        dk * LN2_HI - ((s * (f - r) - dk * LN2_LO) - f)
    }
}

/// logf: the natural logarithm. The game calls the CRT's (`api-ms-win-crt-math-l1-1-0.dll!logf`,
/// through the import thunk `0x142a8e85a`), whose body is not in the image; the tick oracle
/// stands in for it with the double logarithm rounded to f32. This is the same: x widened
/// (`CVTSS2SD`), fdlibm's double logarithm (below 1 ulp, [`log_f64`]) and the rounding
/// (`CVTSD2SS`), which gives the correctly rounded logarithm but within 2^-28 or so of a halfway
/// point. Zero gives −∞, a negative x the default NaN, a NaN itself (quieted).
#[must_use]
pub fn log(x: f32) -> f32 {
    narrow(log_f64(widen(x)))
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
        // ADDSS y, x: the first NaN operand, quieted
        return sse_nan(y + x, &[y, x]);
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

/// `checkint` (`FUN_14099e9a0`) of powf's exponent bits: 0 not an integer, 1 an odd one, 2 an
/// even one.
const fn powf_checkint(iy: u32) -> u32 {
    let e = iy >> 23 & 0xff;
    if e < 0x7f {
        return 0;
    }
    if e > 0x7f + 23 {
        return 2;
    }
    let bit = 1u32 << (0x7f + 23 - e);
    if iy & (bit - 1) != 0 {
        return 0;
    }
    if iy & bit != 0 {
        return 1;
    }
    2
}

/// powf's log2 table (`0x142ec8d30`): 16 pairs of `invc` and `logc` (f64 bits).
const POWF_LOG2: [(u64, u64); 16] = [
    (0x3ff6_61ec_79f8_f3be, 0xbfde_fec6_5b96_3019),
    (0x3ff5_71ed_4aaf_883d, 0xbfdb_0b68_32d4_fca4),
    (0x3ff4_9539_f0f0_10b0, 0xbfd7_418b_0a1f_b77b),
    (0x3ff3_c995_b0b8_0385, 0xbfd3_9de9_1a6d_cf7b),
    (0x3ff3_0d19_0c88_64a5, 0xbfd0_1d9b_f3f2_b631),
    (0x3ff2_5e22_7b0b_8ea0, 0xbfc9_7c1d_1b3b_7af0),
    (0x3ff1_bb4a_4a1a_343f, 0xbfc2_f9e3_93af_3c9f),
    (0x3ff1_2358_f08a_e5ba, 0xbfb9_60cb_bf78_8d5c),
    (0x3ff0_953f_4199_00a7, 0xbfaa_6f9d_b647_5fce),
    (0x3ff0_0000_0000_0000, 0x0000_0000_0000_0000),
    (0x3fee_608c_fd9a_47ac, 0x3fb3_38ca_9f24_f53d),
    (0x3fec_a4b3_1f02_6aa0, 0x3fc4_76a9_5438_91ba),
    (0x3feb_2036_576a_fce6, 0x3fce_840b_4ac4_e4d2),
    (0x3fe9_c2d1_63a1_aa2d, 0x3fd4_0645_f0c6_651c),
    (0x3fe8_86e6_0378_41ed, 0x3fd8_8e9c_2c1b_9ff8),
    (0x3fe7_67dc_f553_4862, 0x3fdc_e0a4_4eb1_7bcc),
];

/// powf's log2 polynomial (`0x142ec8e30`..`0x142ec8e50`, f64 bits).
const POWF_LOG2_POLY: [u64; 5] = [
    0x3fd2_7616_c949_6e0b,
    0xbfd7_1969_a075_c67a,
    0x3fde_c70a_6ca7_badd,
    0xbfe7_1547_48be_f6c8,
    0x3ff7_1547_652a_b82b,
];

/// powf's exp2 table (`0x142ec8aa0`): 2^(i/32) less i << 47, as f64 bits.
const POWF_EXP2: [u64; 32] = [
    0x3ff0_0000_0000_0000,
    0x3fef_d9b0_d315_8574,
    0x3fef_b558_6cf9_890f,
    0x3fef_9301_d012_5b51,
    0x3fef_72b8_3c7d_517b,
    0x3fef_5487_3168_b9aa,
    0x3fef_387a_6e75_6238,
    0x3fef_1e9d_f51f_dee1,
    0x3fef_06fe_0a31_b715,
    0x3fee_f1a7_373a_a9cb,
    0x3fee_dea6_4c12_3422,
    0x3fee_ce08_6061_892d,
    0x3fee_bfda_d536_2a27,
    0x3fee_b42b_569d_4f82,
    0x3fee_ab07_dd48_5429,
    0x3fee_a47e_b03a_5585,
    0x3fee_a09e_667f_3bcd,
    0x3fee_9f75_e8ec_5f74,
    0x3fee_a114_73eb_0187,
    0x3fee_a589_994c_ce13,
    0x3fee_ace5_422a_a0db,
    0x3fee_b737_b0cd_c5e5,
    0x3fee_c491_82a3_f090,
    0x3fee_d503_b23e_255d,
    0x3fee_e89f_995a_d3ad,
    0x3fee_ff76_f2fb_5e47,
    0x3fef_199b_dd85_529c,
    0x3fef_3720_dcef_9069,
    0x3fef_5818_dcfb_a487,
    0x3fef_7c97_337b_9b5f,
    0x3fef_a4af_a2a4_90da,
    0x3fef_d076_5b6e_4540,
];

/// powf's exp2 shift (`0x142ec8ba0`, 0x1.8p+52 / 32) and polynomial (`0x142ec8ba8`..`0x142ec8bb8`).
const POWF_SHIFT: u64 = 0x42e8_0000_0000_0000;
const POWF_EXP2_POLY: [u64; 3] = [
    0x3fac_6af8_4b91_2394,
    0x3fce_bfce_50fa_c4f3,
    0x3fe6_2e42_ff0c_52d6,
];
/// The sign bias of a negative result (1 << 16), which the exp2 table index carries into the
/// sign bit.
const POWF_SIGN_BIAS: u64 = 0x1_0000;

/// powf's overflow and underflow (`FUN_14099cb80`, `FUN_14099cb90` through `FUN_14099cba0`):
/// `x` (2^97, 2^-95) times itself, negated with a sign bias.
fn powf_xflow(sign_bias: u64, x: f32) -> f32 {
    let s = if sign_bias == 0 { x } else { -x };
    x * s
}

/// powf (`FUN_14099e9e0`): musl's (Arm's optimized-routines) power, its log2 and exp2 in f64 over
/// the image's tables, in its machine code's operation order: the special cases of an exponent
/// that is 0, infinite or NaN and of a base that is 0, subnormal, infinite, NaN or negative
/// (`checkint`), the log2 of the base times the exponent, the overflow and underflow beyond
/// 126 (`0x142ec8e58`, −150), then its exp2 rounded to f32 (`CVTPD2PS`).
#[must_use]
pub fn powf(x: f32, y: f32) -> f32 {
    let mut ix = x.to_bits();
    let iy = y.to_bits();
    let mut sign_bias = 0u64;
    let y_special = iy.wrapping_mul(2).wrapping_sub(1) >= 0xfeff_ffff;
    if ix.wrapping_sub(0x0080_0000) >= 0x7f00_0000 || y_special {
        if y_special {
            let y2 = iy.wrapping_mul(2);
            if y2 == 0 || ix == 0x3f80_0000 {
                return ONE;
            }
            let x2 = ix.wrapping_mul(2);
            if x2 > 0xff00_0000 || y2 > 0xff00_0000 {
                return sse_nan(x + y, &[x, y]);
            }
            if x2 == 0x7f00_0000 {
                return ONE;
            }
            // |x| < 1 against a positive y: 0; else y * y
            if (x2 < 0x7f00_0000) == (iy >> 31 == 0) {
                return 0.0;
            }
            return y * y;
        }
        if ix.wrapping_mul(2).wrapping_sub(1) >= 0xfeff_ffff {
            let mut x2 = x * x;
            if ix >> 31 != 0 && powf_checkint(iy) == 1 {
                x2 = -x2;
            }
            if iy >> 31 != 0 {
                return ONE / x2;
            }
            return x2;
        }
        if ix >> 31 != 0 {
            let yint = powf_checkint(iy);
            if yint == 0 {
                return f32::from_bits(INDEFINITE);
            }
            if yint == 1 {
                sign_bias = POWF_SIGN_BIAS;
            }
            ix &= 0x7fff_ffff;
        }
        if ix < 0x0080_0000 {
            // a subnormal base, normalised (0x1p23, `0x142ec8d20`)
            ix = (x * f32::from_bits(0x4b00_0000)).to_bits() & 0x7fff_ffff;
            ix = ix.wrapping_sub(23 << 23);
        }
    }
    // log2(x) = log1p(z / c - 1) / ln 2 + log2(c) + k
    let tmp = ix.wrapping_sub(0x3f33_0000);
    let i = (tmp >> 19 & 0xf) as usize;
    let top = tmp & 0xff80_0000;
    let iz = ix.wrapping_sub(top);
    let k = (top as i32) >> 23;
    let (invc, logc) = POWF_LOG2[i];
    let a = POWF_LOG2_POLY.map(f64::from_bits);
    let r = f64::from(f32::from_bits(iz)) * f64::from_bits(invc) - 1.0;
    let y0 = f64::from(k) + f64::from_bits(logc);
    let q = y0 + r * a[4];
    let p = r * a[2] + a[3];
    let poly = r * a[0] + a[1];
    let r2 = r * r;
    let q = q + p * r2;
    let ylogx = (q + poly * (r2 * r2)) * f64::from(y);
    if (ylogx.to_bits() >> 47 & 0xffff) as u16 >= 0x80bf {
        // |y log2 x| at least 126 (`COMISD`: above 0x1.fffffffd1d571p+6, at most −150)
        if ylogx > f64::from_bits(0x405f_ffff_ffd1_d571) {
            return powf_xflow(sign_bias, f32::from_bits(0x7000_0000));
        }
        if ylogx <= -150.0 {
            return powf_xflow(sign_bias, f32::from_bits(0x1000_0000));
        }
    }
    // exp2(ylogx) = 2^(k / 32) * 2^r, r within 1/64
    let shift = f64::from_bits(POWF_SHIFT);
    let kd = ylogx + shift;
    let ki = kd.to_bits();
    let r = ylogx - (kd - shift);
    let t = POWF_EXP2[(ki & 0x1f) as usize].wrapping_add(ki.wrapping_add(sign_bias) << 47);
    let c = POWF_EXP2_POLY.map(f64::from_bits);
    let z = r * c[0] + c[1];
    let rest = r * c[2] + 1.0;
    ((z * (r * r) + rest) * f64::from_bits(t)) as f32
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

    /// logf: exact at 1 and correctly rounded at a few known values (ln 2, ln 10, ln 1/2, ln 1.5,
    /// ln 1.005, the mill's 1 + 1/200), −∞ at the zeros, ∞ at ∞, the default NaN below 0, a NaN
    /// quieted; a subnormal goes through the scaling (ln 2^-149).
    #[test]
    fn log_is_the_correctly_rounded_logarithm() {
        let cases: [(f32, u32); 8] = [
            (1.0, 0x0000_0000),
            (2.0, 0x3f31_7218),
            (10.0, 0x4013_5d8e),
            (0.5, 0xbf31_7218),
            (1.5, 0x3ecf_991f),
            (1.005, 0x3ba3_6e7e),
            (f32::from_bits(1), 0xc2ce_8ed0),
            (f32::MAX, 0x42b1_7218),
        ];
        for (x, want) in cases {
            assert_eq!(log(x).to_bits(), want, "log({x:e})");
        }
        assert_eq!(log(0.0), f32::NEG_INFINITY);
        assert_eq!(log(-0.0), f32::NEG_INFINITY);
        assert_eq!(log(f32::INFINITY), f32::INFINITY);
        assert_eq!(log(-1.0).to_bits(), INDEFINITE);
        assert_eq!(log(f32::NEG_INFINITY).to_bits(), INDEFINITE);
        assert_eq!(log(f32::from_bits(0x7f80_0001)).to_bits(), 0x7fc0_0001);
        assert_eq!(log(f32::from_bits(0xffc0_1234)).to_bits(), 0xffc0_1234);
    }

    /// The double logarithm's branches: the short series near 1 (|f| below 2^-20, with and
    /// without an exponent), the exact powers of two and both arms of the long series agree with
    /// the f32 values they round to.
    #[test]
    fn the_double_logarithm_takes_each_branch() {
        // 1 + 2^-30: the short series, log ≈ 2^-30
        let tiny = log_f64(1.0 + f64::from_bits(0x3e10_0000_0000_0000));
        assert_eq!(tiny as f32, 9.313_226e-10);
        // 4: k·ln 2 alone
        assert_eq!(
            log_f64(4.0).to_bits(),
            (2.0 * LN2_HI + 2.0 * LN2_LO).to_bits()
        );
        // 2 · (1 + 2^-30): the short series with an exponent
        assert_eq!(
            log_f64(2.0 + f64::from_bits(0x3e20_0000_0000_0000)) as f32,
            std::f32::consts::LN_2
        );
        // 1.2 and 1.4 sit on either side of the i > 0 test
        assert_eq!(log_f64(1.2) as f32, 0.182_321_56);
        assert_eq!(log_f64(1.4) as f32, 0.336_472_23);
        assert_eq!(log_f64(3.0) as f32, 1.098_612_3);
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

    /// cosf `0x14099e4d0` at its branches' bounds (2^-12, π/4, 3π/4, 5π/4, 7π/4, 9π/4, 2^28·π/2),
    /// the multiples of π/4 up to 4π, zeros, subnormals, huge values, infinities and NaNs.
    #[test]
    fn cos_is_the_games_cosf_at_its_bounds() {
        assert_bits(
            "cos",
            cos,
            &[
                (0x0000_0000, 0x3f80_0000),
                (0x8000_0000, 0x3f80_0000),
                (0x0000_0001, 0x3f80_0000),
                (0x8000_0001, 0x3f80_0000),
                (0x007f_ffff, 0x3f80_0000),
                (0x807f_ffff, 0x3f80_0000),
                (0x0080_0000, 0x3f80_0000),
                (0x8080_0000, 0x3f80_0000),
                (0x7f7f_ffff, 0x3f5a_5f96),
                (0xff7f_ffff, 0x3f5a_5f96),
                (0x7f80_0000, 0xffc0_0000),
                (0xff80_0000, 0xffc0_0000),
                (0x7fc0_0000, 0x7fc0_0000),
                (0xffc0_0000, 0xffc0_0000),
                (0x7f80_0001, 0x7fc0_0001),
                (0xff81_2345, 0xffc1_2345),
                (0x397f_ffff, 0x3f80_0000),
                (0x3980_0000, 0x3f80_0000),
                (0xb980_0000, 0x3f80_0000),
                (0x3f49_0fda, 0x3f35_04f4),
                (0x3f49_0fdb, 0x3f35_04f3),
                (0xbf49_0fdb, 0x3f35_04f3),
                (0x4016_cbe3, 0xbf35_04f0),
                (0x4016_cbe4, 0xbf35_04f3),
                (0xc016_cbe4, 0xbf35_04f3),
                (0x407b_53d1, 0xbf35_04f4),
                (0x407b_53d2, 0xbf35_04f1),
                (0xc07b_53d2, 0xbf35_04f1),
                (0x40af_eddf, 0x3f35_04f1),
                (0x40af_ede0, 0x3f35_04f7),
                (0xc0af_ede0, 0x3f35_04f7),
                (0x40e2_31d5, 0x3f35_04f9),
                (0x40e2_31d6, 0x3f35_04f3),
                (0xc0e2_31d6, 0x3f35_04f3),
                (0x4dc9_0fda, 0x3e1d_1cf0),
                (0x4dc9_0fdb, 0x3f2c_42b6),
                (0xcdc9_0fdb, 0x3f2c_42b6),
                (0x5015_02f9, 0x3f5f_84c5),
                (0x7149_f2ca, 0xbf1c_9222),
                (0x6c4a_8ae4, 0x3f61_9f67),
                (0x3f49_0fdb, 0x3f35_04f3),
                (0x3fc9_0fdb, 0xb33b_bd2e),
                (0x4016_cbe4, 0xbf35_04f3),
                (0x4049_0fdb, 0xbf80_0000),
                (0x407b_53d1, 0xbf35_04f4),
                (0x4096_cbe4, 0x324c_de2e),
                (0x40af_eddf, 0x3f35_04f1),
                (0x40c9_0fdb, 0x3f80_0000),
                (0x40e2_31d6, 0x3f35_04f3),
                (0x40fb_53d1, 0x3415_5386),
                (0x410a_3ae6, 0xbf35_04f0),
                (0x4116_cbe4, 0xbf80_0000),
                (0x4123_5ce2, 0xbf35_04ef),
                (0x412f_eddf, 0xb49b_ba77),
                (0x413c_7edd, 0x3f35_04f4),
                (0x4149_0fdb, 0x3f80_0000),
            ],
        );
    }

    /// sinf `0x14099ed60` on the inputs of the cosf test.
    #[test]
    fn sin_is_the_games_sinf_at_its_bounds() {
        assert_bits(
            "sin",
            sin,
            &[
                (0x0000_0000, 0x0000_0000),
                (0x8000_0000, 0x8000_0000),
                (0x0000_0001, 0x0000_0001),
                (0x8000_0001, 0x8000_0001),
                (0x007f_ffff, 0x007f_ffff),
                (0x807f_ffff, 0x807f_ffff),
                (0x0080_0000, 0x0080_0000),
                (0x8080_0000, 0x8080_0000),
                (0x7f7f_ffff, 0xbf05_99b3),
                (0xff7f_ffff, 0x3f05_99b3),
                (0x7f80_0000, 0xffc0_0000),
                (0xff80_0000, 0xffc0_0000),
                (0x7fc0_0000, 0x7fc0_0000),
                (0xffc0_0000, 0xffc0_0000),
                (0x7f80_0001, 0x7fc0_0001),
                (0xff81_2345, 0xffc1_2345),
                (0x397f_ffff, 0x397f_ffff),
                (0x3980_0000, 0x3980_0000),
                (0xb980_0000, 0xb980_0000),
                (0x3f49_0fda, 0x3f35_04f3),
                (0x3f49_0fdb, 0x3f35_04f3),
                (0xbf49_0fdb, 0xbf35_04f3),
                (0x4016_cbe3, 0x3f35_04f6),
                (0x4016_cbe4, 0x3f35_04f3),
                (0xc016_cbe4, 0xbf35_04f3),
                (0x407b_53d1, 0xbf35_04f2),
                (0x407b_53d2, 0xbf35_04f5),
                (0xc07b_53d2, 0x3f35_04f5),
                (0x40af_eddf, 0xbf35_04f5),
                (0x40af_ede0, 0xbf35_04ef),
                (0xc0af_ede0, 0x3f35_04ef),
                (0x40e2_31d5, 0x3f35_04ee),
                (0x40e2_31d6, 0x3f35_04f3),
                (0xc0e2_31d6, 0xbf35_04f3),
                (0x4dc9_0fda, 0xbf7c_f804),
                (0x4dc9_0fdb, 0xbf3d_5f98),
                (0xcdc9_0fdb, 0x3f3d_5f98),
                (0x5015_02f9, 0xbef9_9a64),
                (0x7149_f2ca, 0xbf4a_89b0),
                (0x6c4a_8ae4, 0x3ef1_e998),
                (0x3f49_0fdb, 0x3f35_04f3),
                (0x3fc9_0fdb, 0x3f80_0000),
                (0x4016_cbe4, 0x3f35_04f3),
                (0x4049_0fdb, 0xb3bb_bd2e),
                (0x407b_53d1, 0xbf35_04f2),
                (0x4096_cbe4, 0xbf80_0000),
                (0x40af_eddf, 0xbf35_04f5),
                (0x40c9_0fdb, 0x343b_bd2e),
                (0x40e2_31d6, 0x3f35_04f3),
                (0x40fb_53d1, 0x3f80_0000),
                (0x410a_3ae6, 0x3f35_04f7),
                (0x4116_cbe4, 0xb2cc_de2e),
                (0x4123_5ce2, 0xbf35_04f7),
                (0x412f_eddf, 0xbf80_0000),
                (0x413c_7edd, 0xbf35_04f3),
                (0x4149_0fdb, 0x34bb_bd2e),
            ],
        );
    }

    /// `0x14099db70` (`__kernel_sindf`) and `0x14099cad0` (`__kernel_cosdf`) on doubles: zeros, a
    /// subnormal, small values, ±π/4 (the reduction's float bound and the double), 1, 2 and the
    /// non-finite, rounded to f32 as the game does.
    #[test]
    fn the_kernels_are_the_games_on_doubles() {
        for (x, game) in [
            (0x0000_0000_0000_0000, 0x0000_0000),
            (0x8000_0000_0000_0000, 0x0000_0000),
            (0x0000_0000_0000_0001, 0x0000_0000),
            (0x3e40_0000_0000_0000, 0x3200_0000),
            (0xbe40_0000_0000_0000, 0xb200_0000),
            (0x3fe0_0000_0000_0000, 0x3ef5_7744),
            (0xbfe0_0000_0000_0000, 0xbef5_7744),
            (0x3fe9_21fb_6000_0000, 0x3f35_04f3),
            (0xbfe9_21fb_6000_0000, 0xbf35_04f3),
            (0x3fe9_21fb_5444_2d18, 0x3f35_04f3),
            (0x3fd5_5555_5555_5555, 0x3ea7_8610),
            (0x3f50_624d_d2f1_a9fc, 0x3a83_126d),
            (0x3ff0_0000_0000_0000, 0x3f57_6aa5),
            (0x4000_0000_0000_0000, 0x3f68_c9e4),
            (0x7ff0_0000_0000_0000, 0x7f80_0000),
            (0xfff0_0000_0000_0000, 0xff80_0000),
            (0x7ff8_0000_0000_0001, 0x7fc0_0000),
        ] {
            let got = k_sin(f64::from_bits(x)).to_bits();
            assert_eq!(got, game, "k_sin({x:016x}): {got:08x}, game {game:08x}");
        }
        for (x, game) in [
            (0x0000_0000_0000_0000, 0x3f80_0000),
            (0x8000_0000_0000_0000, 0x3f80_0000),
            (0x0000_0000_0000_0001, 0x3f80_0000),
            (0x3e40_0000_0000_0000, 0x3f80_0000),
            (0xbe40_0000_0000_0000, 0x3f80_0000),
            (0x3fe0_0000_0000_0000, 0x3f60_a940),
            (0xbfe0_0000_0000_0000, 0x3f60_a940),
            (0x3fe9_21fb_6000_0000, 0x3f35_04f3),
            (0xbfe9_21fb_6000_0000, 0x3f35_04f3),
            (0x3fe9_21fb_5444_2d18, 0x3f35_04f3),
            (0x3fd5_5555_5555_5555, 0x3f71_e8b3),
            (0x3f50_624d_d2f1_a9fc, 0x3f7f_fff8),
            (0x3ff0_0000_0000_0000, 0x3f0a_5141),
            (0x4000_0000_0000_0000, 0xbed4_f969),
            (0x7ff0_0000_0000_0000, 0xffc0_0000),
            (0xfff0_0000_0000_0000, 0xffc0_0000),
            (0x7ff8_0000_0000_0001, 0x7fc0_0000),
        ] {
            let got = k_cos(f64::from_bits(x)).to_bits();
            assert_eq!(got, game, "k_cos({x:016x}): {got:08x}, game {game:08x}");
        }
    }

    /// floor `0x14099e7d0`: halves, both signs, 2^51 and 2^52 around, a tiny
    /// negative (−1), zeros kept with their sign, the non-finite as they are.
    #[test]
    fn floor_is_the_games_floor() {
        for (x, game) in [
            (0x0000_0000_0000_0000, 0x0000_0000_0000_0000),
            (0x8000_0000_0000_0000, 0x8000_0000_0000_0000),
            (0x3fe0_0000_0000_0000, 0x0000_0000_0000_0000),
            (0xbfe0_0000_0000_0000, 0xbff0_0000_0000_0000),
            (0x3fef_fffd_e721_0be9, 0x0000_0000_0000_0000),
            (0xbfef_fffd_e721_0be9, 0xbff0_0000_0000_0000),
            (0x3ff0_0000_0000_0000, 0x3ff0_0000_0000_0000),
            (0xbff0_0000_0000_0000, 0xbff0_0000_0000_0000),
            (0x3ff8_0000_0000_0000, 0x3ff0_0000_0000_0000),
            (0xbff8_0000_0000_0000, 0xc000_0000_0000_0000),
            (0x4008_0000_0000_0000, 0x4008_0000_0000_0000),
            (0xc008_0000_0000_0000, 0xc008_0000_0000_0000),
            (0x4320_0000_0000_0001, 0x4320_0000_0000_0000),
            (0xc320_0000_0000_0001, 0xc320_0000_0000_0002),
            (0x4330_0000_0000_0000, 0x4330_0000_0000_0000),
            (0xc330_0000_0000_0000, 0xc330_0000_0000_0000),
            (0x4330_0000_0000_0001, 0x4330_0000_0000_0001),
            (0x7e37_e43c_8800_759c, 0x7e37_e43c_8800_759c),
            (0x81a5_6e1f_c2f8_f359, 0xbff0_0000_0000_0000),
            (0x0000_0000_0000_0001, 0x0000_0000_0000_0000),
            (0x7ff0_0000_0000_0000, 0x7ff0_0000_0000_0000),
            (0xfff0_0000_0000_0000, 0xfff0_0000_0000_0000),
            (0x40c8_1cd6_c8b4_3958, 0x40c8_1c80_0000_0000),
            (0x7ff8_0000_0000_0123, 0x7ff8_0000_0000_0123),
        ] {
            let got = floor(f64::from_bits(x)).to_bits();
            assert_eq!(got, game, "floor({x:016x}): {got:016x}, game {game:016x}");
        }
    }

    /// scalbn `0x14099ecd0`: exponents past 1023 (one or two steps, then capped), below −1022
    /// (into the subnormals, rounded once), and the non-finite.
    #[test]
    fn scalbn_is_the_games_scalbn() {
        for (x, n, game) in [
            (0x3ff0_0000_0000_0000, 5, 0x4040_0000_0000_0000),
            (0x3ff0_0000_0000_0000, 0, 0x3ff0_0000_0000_0000),
            (0x4008_0000_0000_0000, -2, 0x3fe8_0000_0000_0000),
            (0x3ff0_0000_0000_0000, 1023, 0x7fe0_0000_0000_0000),
            (0x3ff0_0000_0000_0000, 1024, 0x7ff0_0000_0000_0000),
            (0x3ff0_0000_0000_0000, 2046, 0x7ff0_0000_0000_0000),
            (0x3ff0_0000_0000_0000, 2047, 0x7ff0_0000_0000_0000),
            (0x3fe0_0000_0000_0000, 3000, 0x7ff0_0000_0000_0000),
            (0x3ff0_0000_0000_0000, -1022, 0x0010_0000_0000_0000),
            (0x3ff0_0000_0000_0000, -1023, 0x0008_0000_0000_0000),
            (0x3ff0_0000_0000_0000, -1074, 0x0000_0000_0000_0001),
            (0x3ff0_0000_0000_0000, -1075, 0x0000_0000_0000_0000),
            (0x3ff8_0000_0000_0000, -1074, 0x0000_0000_0000_0002),
            (0x4008_0000_0000_0000, -1076, 0x0000_0000_0000_0001),
            (0x3ff0_0000_0000_0000, -2000, 0x0000_0000_0000_0000),
            (0x3ff0_0000_0000_0000, -3000, 0x0000_0000_0000_0000),
            (0x01a5_6e1f_c2f8_f359, 900, 0x39e5_6e1f_c2f8_f359),
            (0x7e37_e43c_8800_759c, -2000, 0x0137_e43c_8800_759c),
            (0x7ff0_0000_0000_0000, -5, 0x7ff0_0000_0000_0000),
            (0x8000_0000_0000_0000, 7, 0x8000_0000_0000_0000),
            (0xc01d_0000_0000_0000, 3, 0xc04d_0000_0000_0000),
        ] {
            let got = scalbn(f64::from_bits(x), n).to_bits();
            assert_eq!(
                got, game,
                "scalbn({x:016x}, {n}): {got:016x}, game {game:016x}"
            );
        }
    }

    /// `__rem_pio2f` `0x14099da00`: the quadrant count and the remainder, below 2^28·π/2 by the
    /// rounded quotient, above by `__rem_pio2_large`, the non-finite as `x − x` with 0.
    #[test]
    fn rem_pio2_is_the_games_reduction() {
        for (x, n, game) in [
            (0x3f49_0fdb, 1, 0xbfe9_21fb_4888_5a31),
            (0x3fc9_0fdb, 1, 0x3e67_77a5_cf72_cece),
            (0x4049_0fdb, 2, 0x3e77_77a5_cf72_cece),
            (0x40c9_0fdb, 4, 0x3e87_77a5_cf72_cece),
            (0xc0c9_0fdb, -4, 0xbe87_77a5_cf72_cece),
            (0x40e2_31d6, 5, 0xbfe9_21fb_4aa9_c2f3),
            (0x4b80_0000, 10_680_707, 0x3fe5_a892_0fc7_46b2),
            (0x4dc9_0fda, 268_435_443, 0x3fc3_b78e_1866_0580),
            (0x4dc9_0fdb, 7, 0x3fe7_9e9e_5b72_7593),
            (0xcdc9_0fdb, -7, 0xbfe7_9e9e_5b72_7593),
            (0x4e00_0000, 6, 0xbfd5_4a76_7d18_30ee),
            (0x5a00_0000, 3, 0xbfe1_d1af_db06_8d79),
            (0x6c4a_8ae4, 4, 0x3fdf_7eb4_e77f_f98c),
            (0x7149_f2ca, 7, 0xbfe5_0f0d_49e8_d06f),
            (0x7f7f_ffff, 4, 0xbfe1_91cf_e681_daf7),
            (0xff7f_ffff, -4, 0x3fe1_91cf_e681_daf7),
            (0x7f80_0000, 0, 0xfff8_0000_0000_0000),
            (0xff80_0000, 0, 0xfff8_0000_0000_0000),
            (0x7fc0_0000, 0, 0x7ff8_0000_0000_0000),
            (0x5d7f_9c2a, 7, 0xbfe6_5821_b8a0_00a7),
            (0x47c3_4f80, 63661, 0x3fe1_1f0c_047c_0469),
            (0x4216_cbe4, 24, 0x3e79_9bc5_b961_b1b0),
            (0xc800_0000, -83443, 0xbfa5_8e8e_c12c_0aa8),
        ] {
            let (got_n, got) = rem_pio2(f32::from_bits(x));
            assert_eq!(
                (got_n, got.to_bits()),
                (n, game),
                "rem_pio2({x:08x}): {got_n} {:016x}, game {n} {game:016x}",
                got.to_bits()
            );
        }
    }

    /// `__rem_pio2f` on huge inputs whose reduction takes the rare branches: the fraction's chunks
    /// all 0 (one or two more terms of 2/π, with and without the carry of 1 − q), q0 of 0, 1 and
    /// 2 with the high chunk's bit test (ih 1), and a negative input.
    #[test]
    fn rem_pio2_takes_more_terms_when_the_fraction_s_chunks_are_zero() {
        for (x, n, game) in [
            (0x6f79_be45, 5, 0x3e1b_bdd5_2a58_eafb),
            (0x7b16_75c0, 3, 0x3e70_e91f_c449_51b6),
            (0x57b8_2989, 3, 0x3e6c_b7dc_b9c8_a533),
            (0x63e6_00c1, 1, 0x3e6d_a72e_8575_cfdf),
            (0x6ff9_be45, 2, 0x3e2b_bdd5_2a58_eafb),
            (0x5838_2989, 6, 0x3e7c_b7dc_b9c8_a533),
            (0x6466_00c1, 2, 0x3e7d_a72e_8575_cfdf),
            (0x7034_9fe1, 7, 0x3e7b_0431_1a18_8133),
            (0x636c_1b4a, 6, 0xbe74_5d0d_4e3e_7488),
            (0x7b46_9595, 7, 0xbe74_714e_b3c1_cc19),
            (0x5790_3500, 1, 0xbe77_b153_aba4_8465),
            (0x63ec_1b4a, 4, 0xbe84_5d0d_4e3e_7488),
            (0x6f8a_3cc8, 5, 0xbe79_4853_c772_f283),
            (0x5810_3500, 2, 0xbe87_b153_aba4_8465),
            (0x5858_4f80, 3, 0xbe91_c4fe_c0bb_634c),
            (0x642e_0733, 7, 0xbe56_25d8_2e0e_3264),
            (0x62ec_1b4a, 7, 0xbe64_5d0d_4e3e_7488),
            (0x5700_0002, 1, 0xbfc3_7ba6_e1fe_0b25),
            (0x5780_0001, 3, 0xbfb6_5807_4029_c626),
            (0x5800_0001, 6, 0xbfc6_5807_4029_c626),
            (0x5700_0000, 2, 0x3fb0_9f46_83d2_5023),
            (0x5780_0000, 4, 0x3fc0_9f46_83d2_5023),
            (0x5800_0000, 0, 0x3fd0_9f46_83d2_5023),
            (0x4dc9_43ed, 4, 0xbec7_abaf_e9a6_faf6),
            (0x4dcd_a377, 4, 0x3ebf_8c97_7e1e_2550),
            (0xd810_3500, -2, 0x3e87_b153_aba4_8465),
        ] {
            let (got_n, got) = rem_pio2(f32::from_bits(x));
            assert_eq!(
                (got_n, got.to_bits()),
                (n, game),
                "rem_pio2({x:08x}): {got_n} {:016x}, game {n} {game:016x}",
                got.to_bits()
            );
        }
    }

    /// cosf and sinf on the rare reductions' inputs: `(x, cos, sin)`, the game's outputs.
    #[test]
    fn cos_and_sin_are_the_games_on_the_rare_reductions() {
        for (x, game_cos, game_sin) in [
            (0x6f79_be45, 0xb0dd_eea9, 0x3f80_0000),
            (0x7b16_75c0, 0x3387_48fe, 0xbf80_0000),
            (0x57b8_2989, 0x3365_bee6, 0xbf80_0000),
            (0x63e6_00c1, 0xb36d_3974, 0x3f80_0000),
            (0x6ff9_be45, 0xbf80_0000, 0xb15d_eea9),
            (0x5838_2989, 0xbf80_0000, 0xb3e5_bee6),
            (0x6466_00c1, 0xbf80_0000, 0xb3ed_3974),
            (0x7034_9fe1, 0x33d8_2189, 0xbf80_0000),
            (0x636c_1b4a, 0xbf80_0000, 0x33a2_e86a),
            (0x7b46_9595, 0xb3a3_8a76, 0xbf80_0000),
            (0x5790_3500, 0x33bd_8a9d, 0x3f80_0000),
            (0x63ec_1b4a, 0x3f80_0000, 0xb422_e86a),
            (0x6f8a_3cc8, 0x33ca_429e, 0x3f80_0000),
            (0x5810_3500, 0xbf80_0000, 0x343d_8a9d),
            (0x5858_4f80, 0xb48e_27f6, 0xbf80_0000),
            (0x642e_0733, 0xb2b1_2ec1, 0xbf80_0000),
            (0x62ec_1b4a, 0xb322_e86a, 0xbf80_0000),
            (0x5700_0002, 0x3e1b_4352, 0x3f7d_0a4a),
            (0x5780_0001, 0xbdb2_8626, 0xbf7f_0689),
            (0x5800_0001, 0xbf7c_1c09, 0x3e31_d82f),
            (0x5700_0000, 0xbf7f_75e6, 0xbd84_e24a),
            (0x5780_0000, 0x3f7d_d82f, 0x3e04_9a9b),
            (0x5800_0000, 0x3f77_6a06, 0x3e83_7cc6),
            (0x4dc9_43ed, 0x3f80_0000, 0xb63d_5d7f),
            (0x4dcd_a377, 0x3f80_0000, 0x35fc_64bc),
            (0xd810_3500, 0xbf80_0000, 0xb43d_8a9d),
        ] {
            let x = f32::from_bits(x);
            assert_eq!(
                (cos(x).to_bits(), sin(x).to_bits()),
                (game_cos, game_sin),
                "{x:?}"
            );
        }
    }

    /// `__rem_pio2_large` `0x14099cbc0` on one to three 24-bit chunks, exponents 0 to 1000 and
    /// every precision (0: one double, 1 and 2: two, 3: three), the game's results (unicorn).
    #[test]
    fn rem_pio2_large_is_the_games_at_every_precision() {
        // the chunks, e0, the precision, then the game's count and remainder pieces
        type Case = (&'static [f64], i32, usize, i32, [u64; 3]);
        let cases: [Case; 15] = [
            (
                &[13_821_620.0],
                5,
                0,
                3,
                [
                    0xbfe9_1730_74dc_99bc,
                    0x0000_0000_0000_0000,
                    0x0000_0000_0000_0000,
                ],
            ),
            (
                &[10_919_437.0],
                26,
                0,
                4,
                [
                    0x3fd5_8c3c_18eb_c3f0,
                    0x0000_0000_0000_0000,
                    0x0000_0000_0000_0000,
                ],
            ),
            (
                &[15_012_647.0],
                81,
                0,
                7,
                [
                    0xbfe3_72c7_4c2b_c5f8,
                    0x0000_0000_0000_0000,
                    0x0000_0000_0000_0000,
                ],
            ),
            (
                &[9_198_719.0],
                104,
                0,
                2,
                [
                    0x3fe3_7f17_e976_be82,
                    0x0000_0000_0000_0000,
                    0x0000_0000_0000_0000,
                ],
            ),
            (
                &[9_603_887.0],
                104,
                1,
                1,
                [
                    0x3fc4_104e_da58_bb36,
                    0x3c0c_7e38_6f19_3dbe,
                    0x0000_0000_0000_0000,
                ],
            ),
            (
                &[9_967_848.0],
                104,
                2,
                7,
                [
                    0x3fd4_eeb9_53fc_b992,
                    0xbc79_e1f2_2945_67f5,
                    0x0000_0000_0000_0000,
                ],
            ),
            (
                &[14_523_849.0],
                104,
                3,
                2,
                [
                    0xbf7e_b8b2_5b58_25b3,
                    0xbc10_b095_8f07_3545,
                    0x388e_03aa_a94e_e803,
                ],
            ),
            (
                &[9_361_668.0, 7_204_075.0],
                200,
                0,
                4,
                [
                    0xbfe2_2ad2_07d0_6f51,
                    0x0000_0000_0000_0000,
                    0x0000_0000_0000_0000,
                ],
            ),
            (
                &[9_017_680.0, 2_883_910.0],
                200,
                1,
                6,
                [
                    0x3fe3_d963_aa52_f9f5,
                    0x3c65_588c_1f11_0910,
                    0x0000_0000_0000_0000,
                ],
            ),
            (
                &[15_663_975.0, 14_031_529.0, 2_343_959.0],
                500,
                2,
                5,
                [
                    0x3fdf_0d54_52d1_af31,
                    0xbc71_35e3_0246_7fe3,
                    0x0000_0000_0000_0000,
                ],
            ),
            (
                &[12_426_263.0, 3_043_823.0, 14_244_500.0],
                500,
                3,
                4,
                [
                    0xbfdc_71b5_67ff_6277,
                    0x3c54_c672_ee3f_1526,
                    0xb904_6060_e167_ecab,
                ],
            ),
            (
                &[9_380_317.0, 4_154_104.0, 7_490_656.0],
                1000,
                0,
                2,
                [
                    0xbfe7_f8c7_1112_a643,
                    0x0000_0000_0000_0000,
                    0x0000_0000_0000_0000,
                ],
            ),
            (
                &[9_426_480.0, 13_310_388.0],
                1000,
                3,
                4,
                [
                    0xbfda_f745_087e_eb9a,
                    0x3c71_136e_7c96_c64f,
                    0x3910_16b1_04fb_25a4,
                ],
            ),
            (
                &[9_220_578.0],
                1,
                0,
                5,
                [
                    0xbfe7_60b2_3212_f1db,
                    0x0000_0000_0000_0000,
                    0x0000_0000_0000_0000,
                ],
            ),
            (
                &[12_097_745.0],
                0,
                3,
                0,
                [
                    0xbfe0_af60_dbe9_eea1,
                    0xbc81_c508_f6a3_b340,
                    0xb926_67b1_ee52_51a0,
                ],
            ),
        ];
        for (x, e0, prec, n, game) in cases {
            let (got_n, y) = rem_pio2_large(x, e0, prec);
            assert_eq!(
                (got_n, y.map(f64::to_bits)),
                (n, game),
                "rem_pio2_large({x:?}, {e0}, {prec})"
            );
        }
    }

    /// fmodf: exact remainders with x's sign (zeros kept signed, subnormals, the largest finite
    /// by the smallest subnormal, x below y), x itself by an infinite y; the invalid cases give
    /// SSE's default NaN and a NaN argument comes back quieted.
    #[test]
    fn fmod_is_the_exact_remainder_with_x_s_sign() {
        for (x, y, want) in [
            (0x40b0_0000, 0x4000_0000, 0x3fc0_0000),
            (0xc0b0_0000, 0x4000_0000, 0xbfc0_0000),
            (0x40b0_0000, 0xc000_0000, 0x3fc0_0000),
            (0x8000_0000, 0x4040_0000, 0x8000_0000),
            (0x0000_0000, 0x4040_0000, 0x0000_0000),
            (0x4000_0000, 0x4000_0000, 0x0000_0000),
            (0xc000_0000, 0x4000_0000, 0x8000_0000),
            (0x3f80_0000, 0x4040_0000, 0x3f80_0000),
            (0x4180_0000, 0x4180_0000, 0x0000_0000),
            (0x4188_0000, 0x4180_0000, 0x3f80_0000),
            (0xbf80_0000, 0x4180_0000, 0xbf80_0000),
            (0xc0e8_0000, 0x3f00_0000, 0xbe80_0000),
            (0x42c8_3333, 0x3e99_999a, 0x3e4c_cb5c),
            (0x3f80_0000, 0x7f80_0000, 0x3f80_0000),
            (0xbf80_0000, 0xff80_0000, 0xbf80_0000),
            (0x7f7f_ffff, 0x3fc0_0000, 0x0000_0000),
            (0x7f7f_ffff, 0x0000_0001, 0x0000_0000),
            (0x0000_0001, 0x0000_0001, 0x0000_0000),
            (0x0020_b5c6, 0x0000_0001, 0x0000_0000),
            (0x0036_a3b1, 0x0020_b5c6, 0x0015_edeb),
            (0x4b7f_ffff, 0x3f80_0001, 0x34c0_0000),
            (0x0080_0000, 0x007f_ffff, 0x0000_0001),
            (0x7f7f_ffff, 0x007f_ffff, 0x0000_0001),
            (0x5e80_0000, 0x3dcc_cccd, 0x3dcc_c4cd),
            (0x7f80_0000, 0x3f80_0000, 0xffc0_0000),
            (0x3f80_0000, 0x0000_0000, 0xffc0_0000),
            (0x7f80_0000, 0x0000_0000, 0xffc0_0000),
            (0x0000_0000, 0x8000_0000, 0xffc0_0000),
            (0x7fc0_0001, 0x3f80_0000, 0x7fc0_0001),
            (0x3f80_0000, 0xffc0_0002, 0xffc0_0002),
            (0x7f80_0001, 0x4000_0000, 0x7fc0_0001),
        ] {
            let got = fmod(f32::from_bits(x), f32::from_bits(y)).to_bits();
            assert_eq!(
                got, want,
                "fmod({x:08x}, {y:08x}): {got:08x}, want {want:08x}"
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

    /// powf (`FUN_14099e9e0`): exact powers come out exact; the special cases as the routine's
    /// branches give them: an exponent of 0 or a base of 1 gives 1, a NaN operand the first NaN
    /// quieted, |x| = 1 against ±∞ 1, |x| < 1 against +∞ 0 and against −∞ +∞; a zero base its
    /// square (signed for an odd exponent), inverted for a negative exponent; a negative base
    /// with a non-integer exponent the default NaN, with an odd one a negative result; a
    /// subnormal base normalised; beyond 2^128 +∞ (−∞ with the sign bias), below 2^-150 ±0.
    #[test]
    fn powf_takes_the_image_s_branches_and_exact_powers() {
        let bits = |v: f32| v.to_bits();
        for (x, y, want) in [
            (2.0f32, 10.0f32, 1024.0f32),
            (1.5, 2.0, 2.25),
            (3.0, 5.0, 243.0),
            (0.25, 0.5, 0.5),
            (0.5, 2.0, 0.25),
            (-2.0, 3.0, -8.0),
            (-2.0, 2.0, 4.0),
            (2.0, -2.0, 0.25),
        ] {
            assert_eq!(bits(powf(x, y)), bits(want), "{x}^{y}");
        }
        assert_eq!(powf(7.5, 0.0), 1.0);
        assert_eq!(powf(f32::NAN, 0.0), 1.0);
        assert_eq!(powf(1.0, f32::NAN), 1.0);
        let nan = f32::from_bits(0x7fa0_0001);
        assert_eq!(bits(powf(nan, 2.5)), 0x7fe0_0001);
        assert_eq!(bits(powf(2.0, nan)), 0x7fe0_0001);
        assert_eq!(powf(-1.0, f32::INFINITY), 1.0);
        assert_eq!(powf(0.5, f32::INFINITY), 0.0);
        assert_eq!(powf(0.5, f32::NEG_INFINITY), f32::INFINITY);
        assert_eq!(powf(2.0, f32::INFINITY), f32::INFINITY);
        assert_eq!(bits(powf(-0.0, 3.0)), bits(-0.0));
        assert_eq!(bits(powf(-0.0, 2.0)), bits(0.0));
        assert_eq!(powf(0.0, -1.0), f32::INFINITY);
        assert_eq!(powf(f32::INFINITY, 2.0), f32::INFINITY);
        assert_eq!(bits(powf(-2.0, 0.5)), INDEFINITE);
        assert_eq!(bits(powf(-8.0, 1.0 / 3.0)), INDEFINITE);
        let tiny = f32::from_bits(1);
        assert_eq!(powf(tiny, 0.5), f32::from_bits(0x1a35_04f3));
        assert_eq!(powf(2.0, 200.0), f32::INFINITY);
        assert_eq!(powf(-2.0, 201.0), f32::NEG_INFINITY);
        assert_eq!(bits(powf(2.0, -200.0)), bits(0.0));
        assert_eq!(bits(powf(-2.0, -201.0)), bits(-0.0));
    }

    /// The square of each radius from 0.025 to 7.5 by 0.025 and from 0.1 to 30 by 0.1 is the f32
    /// product: on these powf's double log2 and exp2 round as f32 multiplication does.
    #[test]
    fn powf_squares_the_radii_as_f32_multiplication_does() {
        for i in 1..=300 {
            for x in [i as f32 * 0.025, i as f32 * 0.1] {
                assert_eq!(powf(x, 2.0).to_bits(), (x * x).to_bits(), "{x}");
            }
        }
    }
}

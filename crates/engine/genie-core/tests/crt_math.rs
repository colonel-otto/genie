//! genie-core's f32 math against build 158041's own routines (sqrtf `0x14099ef70`, acosf
//! `0x14099dc60`, cosf `0x14099e4d0`, sinf `0x14099ed60`, atan2f `0x14099e020`), bit for bit on the
//! inputs of
//! `fixtures/crt_math_158041.txt`: edge cases and seeded random inputs over the ranges movement
//! uses, recorded in the tick oracle's emulator (`load_oracle/probes/crt_math.py`).

use genie_core::fmath;

/// Each routine's calls: its name, then per call the inputs and the output as f32 bits.
fn fixture() -> Vec<(String, Vec<Vec<u32>>)> {
    let text = include_str!("fixtures/crt_math_158041.txt");
    let mut out: Vec<(String, Vec<Vec<u32>>)> = Vec::new();
    for line in text.lines() {
        if !line.contains(' ') {
            out.push((line.to_string(), Vec::new()));
            continue;
        }
        let call = line
            .split(' ')
            .map(|s| u32::from_str_radix(s, 16).expect("a hex f32"))
            .collect();
        out.last_mut()
            .expect("a routine name before its calls")
            .1
            .push(call);
    }
    out
}

/// The calls where `ours` gives other bits than the game, as `inputs -> ours, game`.
fn mismatches(name: &str, ours: impl Fn(&[f32]) -> f32) -> Vec<String> {
    let (_, calls) = fixture()
        .into_iter()
        .find(|(n, _)| n == name)
        .expect("the routine is in the fixture");
    assert!(calls.len() > 4000, "{name}: {} calls", calls.len());
    calls
        .into_iter()
        .filter_map(|call| {
            let (game, inputs) = call.split_last().expect("inputs and an output");
            let args: Vec<f32> = inputs.iter().map(|&b| f32::from_bits(b)).collect();
            let got = ours(&args).to_bits();
            // NaN payloads differ between implementations; any NaN matches any NaN
            let both_nan = f32::from_bits(got).is_nan() && f32::from_bits(*game).is_nan();
            (got != *game && !both_nan)
                .then(|| format!("{inputs:08x?} -> {got:08x}, game {game:08x}"))
        })
        .collect()
}

#[test]
fn sqrt_is_the_games_sqrtf() {
    assert_eq!(
        mismatches("sqrtf", |a| fmath::sqrt(a[0])),
        Vec::<String>::new()
    );
}

#[test]
fn acos_is_the_games_acosf() {
    assert_eq!(
        mismatches("acosf", |a| fmath::acos(a[0])),
        Vec::<String>::new()
    );
}

#[test]
fn cos_is_the_games_cosf() {
    assert_eq!(
        mismatches("cosf", |a| fmath::cos(a[0])),
        Vec::<String>::new()
    );
}

#[test]
fn sin_is_the_games_sinf() {
    assert_eq!(
        mismatches("sinf", |a| fmath::sin(a[0])),
        Vec::<String>::new()
    );
}

#[test]
fn atan2_is_the_games_atan2f() {
    assert_eq!(
        mismatches("atan2f", |a| fmath::atan2(a[0], a[1])),
        Vec::<String>::new()
    );
}

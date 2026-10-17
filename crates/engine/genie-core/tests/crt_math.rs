//! genie-core's f32 math against build 158041's own routines (sqrtf `0x14099ef70`, acosf
//! `0x14099dc60`, cosf `0x14099e4d0`, sinf `0x14099ed60`), bit for bit on the inputs of
//! `fixtures/crt_math_158041.txt`: edge cases and seeded random inputs over the ranges movement
//! uses, recorded in the tick oracle's emulator (`load_oracle/probes/crt_math.py`).

use genie_core::fmath;

fn fixture() -> Vec<(String, Vec<(u32, u32)>)> {
    let text = include_str!("fixtures/crt_math_158041.txt");
    let mut out: Vec<(String, Vec<(u32, u32)>)> = Vec::new();
    for line in text.lines() {
        match line.split_once(' ') {
            None => out.push((line.to_string(), Vec::new())),
            Some((input, output)) => {
                let hex = |s: &str| u32::from_str_radix(s, 16).expect("a hex f32");
                out.last_mut()
                    .expect("a routine name before its pairs")
                    .1
                    .push((hex(input), hex(output)));
            }
        }
    }
    out
}

/// The inputs where `ours` gives other bits than the game, as `input -> ours, game`.
fn mismatches(name: &str, ours: fn(f32) -> f32) -> Vec<String> {
    let (_, pairs) = fixture()
        .into_iter()
        .find(|(n, _)| n == name)
        .expect("the routine is in the fixture");
    assert!(pairs.len() > 4000, "{name}: {} pairs", pairs.len());
    pairs
        .into_iter()
        .filter_map(|(input, game)| {
            let got = ours(f32::from_bits(input)).to_bits();
            // NaN payloads differ between implementations; any NaN matches any NaN
            let both_nan = f32::from_bits(got).is_nan() && f32::from_bits(game).is_nan();
            (got != game && !both_nan).then(|| format!("{input:08x} -> {got:08x}, game {game:08x}"))
        })
        .collect()
}

#[test]
fn sqrt_is_the_games_sqrtf() {
    assert_eq!(mismatches("sqrtf", fmath::sqrt), Vec::<String>::new());
}

#[test]
fn acos_is_the_games_acosf() {
    assert_eq!(mismatches("acosf", fmath::acos), Vec::<String>::new());
}

#[test]
fn cos_is_the_games_cosf() {
    assert_eq!(mismatches("cosf", fmath::cos), Vec::<String>::new());
}

#[test]
fn sin_is_the_games_sinf() {
    assert_eq!(mismatches("sinf", fmath::sin), Vec::<String>::new());
}

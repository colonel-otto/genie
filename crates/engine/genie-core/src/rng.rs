//! The engine's random number generators: a `WELL512a` variant in eleven streams, all reseeded at
//! the top of every tick from one frame seed.
//!
//! 158041: setSeed `0x140811ae0`, the step `0x140811070`, `rand15` (`debug_rand`) `0x1408117b0`,
//! `SeedAll` `0x140efe4f0`. The registry order is the order of the static constructors
//! (`0x142e65cf8`); every constructor (`0x140810840`) appends its generator to it.

/// One generator. Only what the simulation reads: the 16-word state and its index, the draw count
/// and the last `rand15` value (`currand`, which a save records).
// not `Copy`: a stream's draws must advance the stream, never a silent copy of it
#[allow(missing_copy_implementations)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Well512 {
    state: [u32; 16],
    index: u32,
    /// Draws since construction; `set_seed` leaves it alone.
    pub draws: u32,
    /// The last `rand15` value; the constructor sets it to `u32::MAX`.
    pub last: u32,
}

impl Default for Well512 {
    fn default() -> Self {
        Self {
            state: [0; 16],
            index: 0,
            draws: 0,
            last: u32::MAX,
        }
    }
}

impl Well512 {
    /// Fill the state from a seed: a byte-swapped xorshift32 mixed with Bob Jenkins' 32-bit
    /// integer hash, chained through the seed. The index restarts at 0.
    pub fn set_seed(&mut self, seed: u32) {
        let mut shift = seed.swap_bytes() ^ 0xaac2_9377;
        let mut hash = seed;
        for word in &mut self.state {
            shift ^= shift << 17;
            shift ^= shift >> 13;
            shift ^= shift << 5;
            hash = jenkins(hash);
            *word = shift ^ hash;
        }
        self.index = 0;
    }

    /// One step. The engine's variant of `WELL512a`: taps at index − 3 and index − 7, the mask
    /// `0xfed22169`, and an extra `^ z1` in the new word.
    pub const fn raw(&mut self) -> u32 {
        self.draws = self.draws.wrapping_add(1);
        let i = self.index as usize;
        let s = &mut self.state;
        let v0 = s[i];
        let t = s[(i.wrapping_sub(3)) & 15];
        let z1 = (((v0 << 1) ^ t) << 15) ^ t ^ v0;
        let c = s[(i.wrapping_sub(7)) & 15];
        let z2 = c ^ (c >> 11);
        let new1 = z1 ^ z2;
        s[i] = new1;
        let j = (i.wrapping_sub(1)) & 15;
        self.index = j as u32;
        let z0 = s[j];
        s[j] = z0 ^ ((((z2 << 10) ^ z1) << 16 ^ z0) << 2) ^ ((new1 & 0xfed2_2169) << 5) ^ new1 ^ z1;
        s[j]
    }

    /// `rand15`: a step masked to 15 bits, kept as `last`.
    /// A 64-bit draw (`FUN_140811100`): two steps, the first one's word high and the second
    /// one's low, counted as one draw.
    pub fn raw64(&mut self) -> u64 {
        let hi = self.raw();
        let lo = self.raw();
        self.draws = self.draws.wrapping_sub(1);
        (u64::from(hi) << 32) | u64::from(lo)
    }

    /// The next draw, 15 bits (`rand15`).
    pub const fn rand15(&mut self) -> u32 {
        self.last = self.raw() & 0x7fff;
        self.last
    }

    /// A step scaled to `0..n` (`FUN_140810cb0`): `step × n >> 32`; `last` stays.
    pub fn below(&mut self, n: u32) -> u32 {
        ((u64::from(self.raw()) * u64::from(n)) >> 32) as u32
    }

    /// An index in `0..n` as the C++ library's `std::shuffle` draws it with the generator as its
    /// engine (MSVC `_Rng_from_urng`, 32 bits a step): a step, again while it falls in the last
    /// incomplete span of `n` (`step / n` is not below `u32::MAX / n`, unless `u32::MAX % n` is
    /// `n − 1`), then `step % n`.
    pub const fn index(&mut self, n: u32) -> u32 {
        loop {
            let v = self.raw();
            if v / n < u32::MAX / n || u32::MAX % n == n - 1 {
                return v % n;
            }
        }
    }

    /// `std::shuffle` with the generator as its engine: each place from the second on swaps
    /// with an [`Well512::index`] up to it.
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in 1..items.len() {
            let j = self.index(i as u32 + 1) as usize;
            if j != i {
                items.swap(i, j);
            }
        }
    }
}

/// Bob Jenkins' 32-bit integer hash, as the compiler folded it in setSeed.
const fn jenkins(a: u32) -> u32 {
    let a = a.wrapping_mul(0x1001).wrapping_add(0x7ed5_5d16);
    let a = ((a >> 19) ^ a ^ 0xc761_c23c).wrapping_mul(0x21);
    let a = (a.wrapping_add(0xe9f8_cc1d) ^ a.wrapping_add(0x1656_67b1).wrapping_mul(0x200))
        .wrapping_mul(9)
        .wrapping_add(0xfd70_46c5);
    (a >> 16) ^ a ^ 0xb55a_4f09
}

/// The streams in registry order. The names come from the constructors' strings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    /// The Action stream.
    Action,
    /// The random map's stream.
    RandomMap,
    /// The sprites' stream.
    Sprite,
    /// The world's stream.
    World,
    /// The AI stream.
    Ai,
    /// The objects' stream.
    Object,
    /// The players' stream.
    Player,
    /// The editor's stream.
    Editor,
    /// The stream outside the sync.
    Unsynced,
    /// The communication's stream.
    Comm,
    /// The interface's stream.
    Ui,
}

/// All eleven generators.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Streams {
    rngs: [Well512; 11],
}

impl Streams {
    /// `SeedAll`: seed each generator in registry order, then take its first raw draw as the
    /// next generator's seed.
    pub fn seed_all(&mut self, seed: u32) {
        let mut seed = seed;
        for rng in &mut self.rngs {
            rng.set_seed(seed);
            seed = rng.raw();
        }
    }

    /// The stream `stream`.
    pub const fn get(&mut self, stream: Stream) -> &mut Well512 {
        &mut self.rngs[stream as usize]
    }

    /// Three different streams at once.
    ///
    /// # Panics
    ///
    /// When two of them are the same stream.
    pub fn three(&mut self, a: Stream, b: Stream, c: Stream) -> [&mut Well512; 3] {
        self.rngs
            .get_disjoint_mut([a as usize, b as usize, c as usize])
            .expect("three different streams")
    }

    /// Two different streams at once.
    ///
    /// # Panics
    ///
    /// When `a` and `b` are the same stream.
    pub fn two(&mut self, a: Stream, b: Stream) -> (&mut Well512, &mut Well512) {
        let (i, j) = (a as usize, b as usize);
        assert_ne!(i, j, "two different streams");
        if i < j {
            let (lo, hi) = self.rngs.split_at_mut(j);
            (&mut lo[i], &mut hi[0])
        } else {
            let (lo, hi) = self.rngs.split_at_mut(i);
            (&mut hi[0], &mut lo[j])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expected values from an independent Python transcription of the same decompiled functions.

    #[test]
    fn set_seed_fills_the_state_and_restarts_the_index() {
        let mut r = Well512::default();
        r.set_seed(29910);
        assert_eq!(
            &r.state[..4],
            &[0xd80f_fb63, 0x00fd_e7a2, 0x5fd2_5128, 0x2699_7df5]
        );
        assert_eq!(r.index, 0);
    }

    #[test]
    fn raw_draws_follow_the_reference() {
        let mut r = Well512::default();
        r.set_seed(29910);
        let got: Vec<u32> = (0..5).map(|_| r.raw()).collect();
        assert_eq!(
            got,
            [
                3_025_362_954,
                1_149_836_362,
                891_056_692,
                54_472_551,
                1_361_767_319
            ]
        );
        assert_eq!(r.draws, 5);
    }

    #[test]
    fn rand15_masks_to_15_bits_and_keeps_the_last_value() {
        let mut r = Well512::default();
        r.set_seed(0);
        let got: Vec<u32> = (0..5).map(|_| r.rand15()).collect();
        assert_eq!(got, [7241, 25269, 9993, 9310, 16801]);
        assert_eq!(r.last, 16801);
    }

    #[test]
    fn twenty_draws_wrap_the_index_twice_over() {
        let mut r = Well512::default();
        r.set_seed(0xdead_be3f);
        let got: Vec<u32> = (0..20).map(|_| r.raw()).collect();
        assert_eq!(&got[17..], &[3_247_376_368, 3_960_506_532, 2_739_179_492]);
    }

    #[test]
    fn seed_all_chains_the_seeds_in_registry_order() {
        let mut s = Streams::default();
        s.seed_all(29910);
        let world: Vec<u32> = (0..3).map(|_| s.get(Stream::World).rand15()).collect();
        let ai: Vec<u32> = (0..3).map(|_| s.get(Stream::Ai).rand15()).collect();
        assert_eq!(world, [19550, 2220, 13198]);
        assert_eq!(ai, [16591, 22366, 24306]);
    }

    #[test]
    fn a_64_bit_draw_is_two_steps_counted_once() {
        // FUN_140811100: the first step's word high, the second's low, the draw count +1
        let (mut a, mut b) = (Well512::default(), Well512::default());
        a.set_seed(7);
        b.set_seed(7);
        let wide = a.raw64();
        let (hi, lo) = (b.raw(), b.raw());
        assert_eq!(wide, (u64::from(hi) << 32) | u64::from(lo));
        assert_eq!((a.draws, b.draws), (1, 2));
    }
}

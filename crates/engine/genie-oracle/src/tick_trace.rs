//! The tick oracle's per-tick trace (FD-015): `GRTICK1`, a `u32` tick count, a `u32` lane count,
//! then zlib of one row per tick, the world time and the lanes' 11 checkpoint words (all `u32`
//! little endian). Build 158041's own simulation writes it while it plays a recording
//! (`aoe2de-exe-re/load_oracle/playback.py --trace`); a row holds the state after its tick.

use flate2::read::ZlibDecoder;
use std::io::Read;

const MAGIC: &[u8; 8] = b"GRTICK1\0";

/// Why a trace could not be read.
#[derive(Debug)]
pub enum TraceError {
    NotATrace,
    Inflate(std::io::Error),
    /// The rows hold fewer bytes than the header announces.
    Truncated,
}

impl std::fmt::Display for TraceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotATrace => write!(f, "not a GRTICK1 trace"),
            Self::Inflate(e) => write!(f, "the rows do not inflate: {e}"),
            Self::Truncated => write!(f, "the rows end before the last tick"),
        }
    }
}

impl std::error::Error for TraceError {}

/// Every tick's world time and checkpoint lanes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TickTrace {
    times: Vec<u32>,
    lanes: Vec<Vec<[u32; 11]>>,
}

impl TickTrace {
    /// A trace from its file's bytes: the magic, the tick and lane counts, then each tick's row
    /// of eleven words per lane.
    ///
    /// # Errors
    ///
    /// [`TraceError`] when the bytes are not a trace, do not inflate or end early.
    ///
    /// # Panics
    ///
    /// Never: the header's length is checked before its words are read.
    pub fn decode(data: &[u8]) -> Result<Self, TraceError> {
        if data.len() < 16 || &data[..8] != MAGIC {
            return Err(TraceError::NotATrace);
        }
        let ticks = u32::from_le_bytes(data[8..12].try_into().unwrap()) as usize;
        let lane_count = u32::from_le_bytes(data[12..16].try_into().unwrap()) as usize;
        let mut rows = Vec::new();
        let _ = ZlibDecoder::new(&data[16..])
            .read_to_end(&mut rows)
            .map_err(TraceError::Inflate)?;
        let row = 4 * (1 + 11 * lane_count);
        if rows.len() < ticks * row {
            return Err(TraceError::Truncated);
        }
        let word = |at: usize| u32::from_le_bytes(rows[at..at + 4].try_into().unwrap());
        let mut times = Vec::with_capacity(ticks);
        let mut lanes = Vec::with_capacity(ticks);
        for t in 0..ticks {
            let base = t * row;
            times.push(word(base));
            lanes.push(
                (0..lane_count)
                    .map(|l| std::array::from_fn(|w| word(base + 4 + 4 * (11 * l + w))))
                    .collect(),
            );
        }
        Ok(Self { times, lanes })
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.times.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.times.is_empty()
    }

    /// The world time after tick `tick` (0-based), in milliseconds.
    #[must_use]
    pub fn world_time(&self, tick: usize) -> u32 {
        self.times[tick]
    }

    /// The lanes after tick `tick`, in lobby order.
    #[must_use]
    pub fn lanes(&self, tick: usize) -> &[[u32; 11]] {
        &self.lanes[tick]
    }

    /// The first (tick, lane, word) where two traces differ, over the ticks both hold.
    #[must_use]
    pub fn first_difference(&self, other: &Self) -> Option<(usize, usize, usize)> {
        self.lanes
            .iter()
            .zip(&other.lanes)
            .enumerate()
            .find_map(|(t, (a, b))| {
                a.iter()
                    .zip(b)
                    .enumerate()
                    .find_map(|(l, (x, y))| (0..11).find(|&w| x[w] != y[w]).map(|w| (t, l, w)))
            })
    }
}

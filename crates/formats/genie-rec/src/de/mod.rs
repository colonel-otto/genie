//! Recorded games of Age of Empires II: Definitive Edition (save 66.x and later).
//!
//! The rest of this crate reads the older formats (AoK, AoC, HD). This module reads the current
//! one. Its layouts follow the workspace's reverse engineering of the 158041 engine and the
//! [aoe2rec](https://github.com/aoe2ct/aoe2rec) crate and ImHex patterns (MIT, see `NOTICE`).
//!
//! A recording is a list of chapters. Each chapter is a compressed world save (the "header") and
//! the ops recorded after it, up to the next chapter or the end of the file. Everything here
//! borrows from the recording's bytes.

pub mod body;

pub use body::{Checkpoint, Op, OpReader, Pregame, TriggerSync, View};

/// Errors while reading a DE recording. Offsets are byte offsets into the recording file.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("the recording ends inside a record at byte {at:#x}")]
    Truncated { at: usize },
    #[error("unknown op {op} at byte {at:#x}")]
    UnknownOp { op: u32, at: usize },
    #[error("the chapter at byte {at:#x} points outside the recording")]
    InvalidChapter { at: usize },
    #[error("the header of the chapter at byte {at:#x} does not inflate")]
    Inflate { at: usize },
}

/// One chapter: a compressed world save and the ops recorded after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chapter<'a> {
    /// Position in the recording's chapter list; only chapter 0 starts with a [`Pregame`] record.
    pub index: usize,
    /// File offset of the chapter.
    pub offset: usize,
    /// The raw-deflate-compressed world save.
    pub compressed_header: &'a [u8],
    /// File offset of the first op.
    pub body_offset: usize,
    /// The ops, up to the next chapter or the end of the file.
    pub body: &'a [u8],
}

/// The version fields at the start of a chapter header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaveVersion {
    pub major: u16,
    /// A binary fraction: 39321 / 65536 is the ".6" of save 66.6.
    pub minor: u16,
    /// The engine build (`de.build`), the UI build + 0x20000.
    pub build: u32,
}

impl SaveVersion {
    /// The save version in hundredths, e.g. 6660 for 66.6.
    pub fn hundredths(&self) -> u32 {
        // the engine rounds the fraction to two decimals (66.6 -> 6660)
        u32::from(self.major) * 100 + (u32::from(self.minor) * 100 + 32768) / 65536
    }
}

/// Split a recording into its chapters.
pub fn chapters(data: &[u8]) -> Result<Vec<Chapter<'_>>, Error> {
    let mut out = Vec::new();
    let mut offset = 0;
    loop {
        let word = |at: usize| {
            data.get(at..at + 4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
        };
        let invalid = Error::InvalidChapter { at: offset };
        let (header_end, next) = word(offset).zip(word(offset + 4)).ok_or(invalid.clone())?;
        let end = if next == 0 { data.len() } else { next };
        if header_end < offset + 8
            || header_end > end
            || end > data.len()
            || (next != 0 && next <= offset)
        {
            return Err(invalid);
        }
        out.push(Chapter {
            index: out.len(),
            offset,
            compressed_header: &data[offset + 8..header_end],
            body_offset: header_end,
            body: &data[header_end..end],
        });
        if next == 0 {
            return Ok(out);
        }
        offset = next;
    }
}

impl<'a> Chapter<'a> {
    /// The ops of this chapter.
    pub fn ops(&self) -> OpReader<'a> {
        OpReader::new(self.body, self.body_offset, self.index == 0)
    }

    /// Inflate the start of the header and read its version fields.
    pub fn save_version(&self) -> Result<SaveVersion, Error> {
        use std::io::Read;
        // "VER 9.4\0", the old f32 save version, minor, major, build
        let mut start = [0u8; 20];
        flate2::read::DeflateDecoder::new(self.compressed_header)
            .read_exact(&mut start)
            .map_err(|_| Error::Inflate { at: self.offset })?;
        Ok(SaveVersion {
            minor: u16::from_le_bytes([start[12], start[13]]),
            major: u16::from_le_bytes([start[14], start[15]]),
            build: u32::from_le_bytes([start[16], start[17], start[18], start[19]]),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn deflate(bytes: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
        e.write_all(bytes).unwrap();
        e.finish().unwrap()
    }

    fn le(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    /// A recording with the given chapters, each a (compressed header, body) pair.
    fn recording(parts: &[(&[u8], &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        for (i, (header, body)) in parts.iter().enumerate() {
            let start = out.len();
            let header_end = start + 8 + header.len();
            let next = if i + 1 < parts.len() {
                header_end + body.len()
            } else {
                0
            };
            out.extend(le(&[header_end as u32, next as u32]));
            out.extend_from_slice(header);
            out.extend_from_slice(body);
        }
        out
    }

    #[test]
    fn chapters_split_at_the_next_chapter_offsets() {
        let data = recording(&[(b"HDR0", b"body-zero"), (b"HDR-1", b"body1")]);
        let ch = chapters(&data).unwrap();
        assert_eq!(ch.len(), 2);
        assert_eq!(
            (
                ch[0].index,
                ch[0].offset,
                ch[0].compressed_header,
                ch[0].body
            ),
            (0, 0, &b"HDR0"[..], &b"body-zero"[..])
        );
        assert_eq!(
            (ch[1].index, ch[1].compressed_header, ch[1].body),
            (1, &b"HDR-1"[..], &b"body1"[..])
        );
        assert_eq!(ch[1].body_offset, ch[1].offset + 8 + 5);
    }

    #[test]
    fn a_chapter_pointing_outside_the_file_is_an_error() {
        let mut data = recording(&[(b"HDR0", b"body")]);
        data[0..4].copy_from_slice(&999u32.to_le_bytes());
        assert_eq!(chapters(&data), Err(Error::InvalidChapter { at: 0 }));
    }

    #[test]
    fn save_version_reads_the_inflated_header_start() {
        let mut header = b"VER 9.4\0".to_vec();
        header.extend((-1.0f32).to_le_bytes());
        header.extend(39321u16.to_le_bytes());
        header.extend(66u16.to_le_bytes());
        header.extend(158041u32.to_le_bytes());
        header.extend([0u8; 64]);
        let data = recording(&[(&deflate(&header), b"")]);
        let v = chapters(&data).unwrap()[0].save_version().unwrap();
        assert_eq!(
            (v.major, v.minor, v.build, v.hundredths()),
            (66, 39321, 158041, 6660)
        );
    }

    #[test]
    fn a_header_that_does_not_inflate_is_an_error() {
        let data = recording(&[(&[0xff; 16], b"")]);
        assert_eq!(
            chapters(&data).unwrap()[0].save_version(),
            Err(Error::Inflate { at: 0 })
        );
    }
}

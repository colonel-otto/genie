//! The ops recorded after a chapter's world save.
//!
//! Every record starts with a `u32` op id (`SIMULATION.md`, "Record ops"). DE recordings carry no
//! sequence byte after it. The reader dispatches on the op id by hand: binrw's enum derive would try
//! the variants in order and allocate an error for every mismatch.

use super::Error;
use binrw::binrw;

/// Op 5 as the first record of a recording.
#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pregame {
    /// Syncs between two checkpoints (500 in every tier-1 recording).
    pub checkpoint_interval: u32,
    /// aoe2rec's name; 1 in every multiplayer recording seen.
    pub multiplayer: u32,
    /// aoe2rec's name ("rec owner"); not verified against the engine.
    pub recording_player: u32,
    pub unknown3: u32,
    /// aoe2rec calls it "use sequence numbers", but recordings with 1 here carry no sequence byte.
    pub unknown4: u32,
    pub unknown5: u32,
    pub unknown6: u32,
}

/// The per-player state checkpoint of an op-2 record: 8 lanes of the 11-word block, then the world
/// time. Lanes follow the lobby slots; unused lanes are zero.
#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Checkpoint {
    pub lanes: [[u32; 11]; 8],
    pub world_time: u32,
}

/// Op 3: the recording player's camera position.
#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub x: f32,
    pub y: f32,
    pub player: i32,
}

/// Op 5 after the first record: the recorded outcome of a trigger condition.
#[binrw]
#[brw(little)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TriggerSync {
    pub condition: i32,
    pub trigger: i32,
}

/// One record of a chapter's op stream.
#[derive(Debug, Clone, PartialEq)]
pub enum Op<'a> {
    Pregame(Pregame),
    /// Op 1: a command packet, executed in the tick it was recorded in.
    Command {
        packet: &'a [u8],
        world_time: u32,
    },
    /// Op 2: the next tick's length in milliseconds.
    Sync {
        dt: u32,
    },
    /// Op 2 carrying a checkpoint. Boxed: it is 356 bytes and comes once per `checkpoint_interval`
    /// syncs, so every other op stays small and allocation-free.
    Checkpoint {
        dt: u32,
        checkpoint: Box<Checkpoint>,
    },
    View(View),
    /// Op 4: a chat line (JSON text in DE).
    Chat {
        channel: i32,
        text: &'a [u8],
    },
    TriggerSync(TriggerSync),
    /// Op 6: the end of playback. The postgame trailer follows.
    End {
        trailer: &'a [u8],
    },
}

/// Iterator over the ops of one chapter. It stops after `End`, at the end of the body, or after the
/// first error.
#[derive(Debug, Clone)]
pub struct OpReader<'a> {
    data: &'a [u8],
    base: usize,
    pos: usize,
    pregame_allowed: bool,
    done: bool,
}

impl<'a> OpReader<'a> {
    /// `base` is the body's file offset, used in error offsets. Only the first chapter's first record
    /// may be a [`Pregame`].
    pub fn new(body: &'a [u8], base: usize, first_chapter: bool) -> Self {
        Self {
            data: body,
            base,
            pos: 0,
            pregame_allowed: first_chapter,
            done: false,
        }
    }
}

impl<'a> OpReader<'a> {
    fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    /// The next `n` bytes, or `Truncated` at the start of the current record.
    fn take(&mut self, n: usize, record: usize) -> Result<&'a [u8], Error> {
        if self.remaining() < n {
            return Err(Error::Truncated {
                at: self.base + record,
            });
        }
        let bytes = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(bytes)
    }

    fn u32(&mut self, record: usize) -> Result<u32, Error> {
        let b = self.take(4, record)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// A fixed-size binrw record of `size` bytes.
    fn fixed<T>(&mut self, size: usize, record: usize) -> Result<T, Error>
    where
        T: for<'b> binrw::BinRead<Args<'b> = ()>,
    {
        let bytes = self.take(size, record)?;
        T::read_le(&mut binrw::io::Cursor::new(bytes)).map_err(|_| Error::Truncated {
            at: self.base + record,
        })
    }

    fn read_op(&mut self) -> Result<Op<'a>, Error> {
        let record = self.pos;
        let first = std::mem::replace(&mut self.pregame_allowed, false);
        match self.u32(record)? {
            1 => {
                let len = self.u32(record)? as usize;
                let packet = self.take(len, record)?;
                let world_time = self.u32(record)?;
                Ok(Op::Command { packet, world_time })
            }
            2 => {
                let dt = self.u32(record)?;
                let marker = self.data.get(self.pos..self.pos + 4);
                if marker == Some(&[0, 0, 0, 0][..]) {
                    self.pos += 4;
                    let checkpoint = Box::new(self.fixed(CHECKPOINT_SIZE, record)?);
                    Ok(Op::Checkpoint { dt, checkpoint })
                } else {
                    Ok(Op::Sync { dt })
                }
            }
            3 => Ok(Op::View(self.fixed(12, record)?)),
            4 => {
                let channel = self.u32(record)? as i32;
                let len = self.u32(record)? as usize;
                let text = self.take(len, record)?;
                Ok(Op::Chat { channel, text })
            }
            5 if first => Ok(Op::Pregame(self.fixed(28, record)?)),
            5 => Ok(Op::TriggerSync(self.fixed(8, record)?)),
            6 => {
                let trailer = &self.data[self.pos..];
                self.pos = self.data.len();
                self.done = true;
                Ok(Op::End { trailer })
            }
            op => Err(Error::UnknownOp {
                op,
                at: self.base + record,
            }),
        }
    }
}

/// 8 lanes x 11 words + the world time.
const CHECKPOINT_SIZE: usize = (8 * 11 + 1) * 4;

impl<'a> Iterator for OpReader<'a> {
    type Item = Result<Op<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done || self.remaining() == 0 {
            return None;
        }
        let op = self.read_op();
        if op.is_err() {
            self.done = true;
        }
        Some(op)
    }
}

impl Op<'_> {
    /// Write the record back in the recording's encoding.
    pub fn write(&self, out: &mut Vec<u8>) {
        let words =
            |out: &mut Vec<u8>, ws: &[u32]| ws.iter().for_each(|w| out.extend(w.to_le_bytes()));
        match self {
            Op::Pregame(p) => {
                words(out, &[5]);
                write_fixed(p, out);
            }
            Op::Command { packet, world_time } => {
                words(out, &[1, packet.len() as u32]);
                out.extend_from_slice(packet);
                words(out, &[*world_time]);
            }
            Op::Sync { dt } => words(out, &[2, *dt]),
            Op::Checkpoint { dt, checkpoint } => {
                words(out, &[2, *dt, 0]);
                write_fixed(checkpoint.as_ref(), out);
            }
            Op::View(v) => {
                words(out, &[3]);
                write_fixed(v, out);
            }
            Op::Chat { channel, text } => {
                words(out, &[4, *channel as u32, text.len() as u32]);
                out.extend_from_slice(text);
            }
            Op::TriggerSync(t) => {
                words(out, &[5]);
                write_fixed(t, out);
            }
            Op::End { trailer } => {
                words(out, &[6]);
                out.extend_from_slice(trailer);
            }
        }
    }
}

/// Append a fixed-size binrw record. Writing plain numbers into a growing `Vec` cannot fail.
fn write_fixed<T>(value: &T, out: &mut Vec<u8>)
where
    T: for<'b> binrw::BinWrite<Args<'b> = ()>,
{
    let mut cursor = binrw::io::Cursor::new(out);
    cursor.set_position(cursor.get_ref().len() as u64);
    value
        .write_le(&mut cursor)
        .expect("writing fixed-size numbers into a Vec cannot fail");
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: usize = 0x100;

    fn le(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    fn read(body: &[u8], first_chapter: bool) -> Vec<Result<Op<'_>, Error>> {
        OpReader::new(body, BASE, first_chapter).collect()
    }

    fn checkpoint_body() -> (Vec<u8>, Checkpoint) {
        let mut lanes = [[0u32; 11]; 8];
        lanes[1] = [0, 700, 11230, 126, 150, 0, 63, 771100, 2, 62248, 586462];
        let mut body = le(&[2, 33, 0]);
        body.extend(lanes.iter().flatten().flat_map(|w| w.to_le_bytes()));
        body.extend(le(&[11298]));
        (
            body,
            Checkpoint {
                lanes,
                world_time: 11298,
            },
        )
    }

    #[test]
    fn op_5_is_pregame_only_as_the_first_record_of_the_first_chapter() {
        let mut body = le(&[5, 500, 1, 1, 2, 0, 0, 0]);
        body.extend(le(&[5, 7, 3]));
        let pregame = Pregame {
            checkpoint_interval: 500,
            multiplayer: 1,
            recording_player: 1,
            unknown3: 2,
            unknown4: 0,
            unknown5: 0,
            unknown6: 0,
        };
        assert_eq!(
            read(&body, true),
            vec![
                Ok(Op::Pregame(pregame)),
                Ok(Op::TriggerSync(TriggerSync {
                    condition: 7,
                    trigger: 3
                }))
            ]
        );
        assert_eq!(
            read(&le(&[5, 7, 3]), false),
            vec![Ok(Op::TriggerSync(TriggerSync {
                condition: 7,
                trigger: 3
            }))]
        );
    }

    #[test]
    fn a_command_borrows_its_packet() {
        let mut body = le(&[1, 3]);
        body.extend([0x67, 1, 0x10]);
        body.extend(le(&[4242]));
        assert_eq!(
            read(&body, false),
            vec![Ok(Op::Command {
                packet: &[0x67, 1, 0x10],
                world_time: 4242
            })]
        );
    }

    #[test]
    fn a_sync_followed_by_another_op_is_a_plain_sync() {
        let mut body = le(&[2, 104, 3]);
        body.extend(le(&[0x431e0000, 0x41c00000, 5]));
        let view = View {
            x: 158.0,
            y: 24.0,
            player: 5,
        };
        assert_eq!(
            read(&body, false),
            vec![Ok(Op::Sync { dt: 104 }), Ok(Op::View(view))]
        );
    }

    #[test]
    fn a_sync_followed_by_a_zero_marker_carries_a_checkpoint() {
        let (body, checkpoint) = checkpoint_body();
        let checkpoint = Box::new(checkpoint);
        assert_eq!(
            read(&body, false),
            vec![Ok(Op::Checkpoint { dt: 33, checkpoint })]
        );
    }

    #[test]
    fn a_sync_at_the_end_of_the_body_is_a_plain_sync() {
        assert_eq!(read(&le(&[2, 20]), false), vec![Ok(Op::Sync { dt: 20 })]);
    }

    #[test]
    fn a_chat_borrows_its_text() {
        let mut body = le(&[4, u32::MAX, 2]);
        body.extend(b"hi");
        assert_eq!(
            read(&body, false),
            vec![Ok(Op::Chat {
                channel: -1,
                text: b"hi"
            })]
        );
    }

    #[test]
    fn end_returns_the_trailer_and_stops() {
        let mut body = le(&[6]);
        body.extend(b"postgame");
        assert_eq!(
            read(&body, false),
            vec![Ok(Op::End {
                trailer: b"postgame"
            })]
        );
    }

    #[test]
    fn a_truncated_record_reports_its_file_offset_and_stops() {
        let mut body = le(&[2, 20, 1, 10]);
        body.extend([1, 2, 3]);
        assert_eq!(
            read(&body, false),
            vec![
                Ok(Op::Sync { dt: 20 }),
                Err(Error::Truncated { at: BASE + 8 })
            ]
        );
    }

    #[test]
    fn an_unknown_op_is_an_error() {
        assert_eq!(
            read(&le(&[9]), false),
            vec![Err(Error::UnknownOp { op: 9, at: BASE })]
        );
    }

    #[test]
    fn writing_every_op_reproduces_the_body() {
        let mut body = le(&[5, 500, 1, 1, 2, 0, 0, 0, 1, 4, 0x0167, 4242, 2, 104]);
        body.extend(checkpoint_body().0);
        body.extend(le(&[3, 0x431e0000, 0x41c00000, 5, 4, u32::MAX, 2]));
        body.extend(b"hi");
        body.extend(le(&[5, 7, 3, 6]));
        body.extend(b"postgame");
        let mut out = Vec::new();
        for op in OpReader::new(&body, BASE, true) {
            op.unwrap().write(&mut out);
        }
        assert_eq!(out, body);
    }
}

//! What the unit tests share: a header reader over bytes and the write-read-write check.

use crate::element::{ReadableHeaderElement, WritableHeaderElement};
use crate::reader::RecordingHeaderReader;
use std::io::Read;

/// A header reader over `bytes` at save `version` (game version `VER 9.4`).
pub fn header_reader(bytes: &[u8], version: f32) -> RecordingHeaderReader<&[u8]> {
    let mut reader = RecordingHeaderReader::new(bytes);
    reader.set_version(b"VER 9.4", version);
    reader
}

/// Whether `reader` has no bytes left.
pub fn consumed(reader: &mut RecordingHeaderReader<&[u8]>) -> bool {
    let mut rest = vec![];
    reader.read_to_end(&mut rest).unwrap() == 0
}

/// `value` written, read back at `version` after `setup` (a map size, a player count) and
/// written again: the same bytes, all of them read. Gives the value read.
pub fn writes_back_with<T: ReadableHeaderElement + WritableHeaderElement>(
    value: &T,
    version: f32,
    setup: impl FnOnce(&mut RecordingHeaderReader<&[u8]>),
) -> T {
    let mut bytes = vec![];
    value.write_to(&mut bytes).unwrap();
    let mut reader = header_reader(&bytes, version);
    setup(&mut reader);
    let read = <T as ReadableHeaderElement>::read_from(&mut reader).unwrap();
    assert!(consumed(&mut reader), "bytes left after the read");
    let mut again = vec![];
    read.write_to(&mut again).unwrap();
    assert_eq!(again, bytes);
    read
}

/// [`writes_back_with`] without a setup.
pub fn writes_back<T: ReadableHeaderElement + WritableHeaderElement>(value: &T, version: f32) -> T {
    writes_back_with(value, version, |_| {})
}

/// The element of type `T` read from `bytes` at `version`, all of them read.
pub fn read_all<T: ReadableHeaderElement>(bytes: &[u8], version: f32) -> T {
    let mut reader = header_reader(bytes, version);
    let read = <T as ReadableHeaderElement>::read_from(&mut reader).unwrap();
    assert!(consumed(&mut reader), "bytes left after the read");
    read
}

/// Little-endian words.
pub fn words(ws: &[i32]) -> Vec<u8> {
    ws.iter().flat_map(|w| w.to_le_bytes()).collect()
}

/// Little-endian floats.
pub fn floats(fs: &[f32]) -> Vec<u8> {
    fs.iter().flat_map(|f| f.to_le_bytes()).collect()
}

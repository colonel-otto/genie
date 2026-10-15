//! Building blocks of the header layout.

#[cfg(test)]
use binrw::BinRead;
use binrw::binread;

/// A DE string: the magic `60 0A`, a u16 length, the bytes.
#[binread]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[br(little, magic = b"\x60\x0a")]
pub struct DeString {
    #[br(temp)]
    len: u16,
    #[br(count = len)]
    pub bytes: Vec<u8>,
}

/// A list of up to `max` strings, each preceded by a u32 crc. A crc in 1..=254 ends the list early
/// (pattern: `DeStringArray` with its `break`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StringList {
    pub entries: Vec<(u32, DeString)>,
    /// The crc that ended the list, or `None` when it held `max` strings.
    pub terminator: Option<u32>,
}

#[binrw::parser(reader, endian)]
pub fn string_list(max: usize) -> binrw::BinResult<StringList> {
    use binrw::BinRead;
    let mut entries = Vec::new();
    while entries.len() < max {
        let crc = u32::read_options(reader, endian, ())?;
        if (1..255).contains(&crc) {
            return Ok(StringList {
                entries,
                terminator: Some(crc),
            });
        }
        entries.push((crc, DeString::read_options(reader, endian, ())?));
    }
    Ok(StringList {
        entries,
        terminator: None,
    })
}

/// The next byte, without consuming it.
pub fn peek_u8<R: binrw::io::Read + binrw::io::Seek>(reader: &mut R) -> binrw::BinResult<u8> {
    use binrw::io::SeekFrom;
    let mut b = [0u8; 1];
    reader.read_exact(&mut b)?;
    reader.seek(SeekFrom::Current(-1))?;
    Ok(b[0])
}

/// Consume `magic` or fail with `BadMagic` at its position.
pub fn expect<R: binrw::io::Read + binrw::io::Seek>(
    reader: &mut R,
    magic: &[u8],
) -> binrw::BinResult<()> {
    let pos = reader.stream_position()?;
    let mut got = vec![0u8; magic.len()];
    reader.read_exact(&mut got)?;
    if got != magic {
        return Err(binrw::Error::BadMagic {
            pos,
            found: Box::new(got),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use binrw::io::Cursor;

    #[test]
    fn de_string_reads_magic_length_and_bytes() {
        let mut c = Cursor::new(b"\x60\x0a\x02\x00hi!".as_slice());
        assert_eq!(DeString::read(&mut c).unwrap().bytes, b"hi");
        assert_eq!(c.position(), 6);
    }

    #[test]
    fn string_list_stops_at_a_small_crc() {
        let bytes = b"\x00\x00\x00\x00\x60\x0a\x01\x00a\x05\x00\x00\x00rest";
        let mut c = Cursor::new(bytes.as_slice());
        let list = string_list(&mut c, binrw::Endian::Little, (10,)).unwrap();
        assert_eq!(list.entries.len(), 1);
        assert_eq!(
            (list.entries[0].1.bytes.as_slice(), list.terminator),
            (b"a".as_slice(), Some(5))
        );
        assert_eq!(c.position(), 13);
    }

    #[test]
    fn string_list_ends_when_full() {
        let bytes = b"\x00\x01\x00\x00\x60\x0a\x00\x00";
        let list = string_list(
            &mut Cursor::new(bytes.as_slice()),
            binrw::Endian::Little,
            (1,),
        )
        .unwrap();
        assert_eq!((list.entries.len(), list.terminator), (1, None));
    }

    #[test]
    fn de_string_rejects_a_wrong_magic() {
        assert!(DeString::read(&mut Cursor::new(b"\x60\x0b\x00\x00".as_slice())).is_err());
    }
}

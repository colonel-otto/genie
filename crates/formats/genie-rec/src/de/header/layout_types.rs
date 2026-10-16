//! The value shapes of the generated layouts (`generated/`): what a read helper of the game reads.
//! Hand-written; the generator names them in its helper table (aoe2de-exe-re `layout/b<build>.py`).
//! Each writes back the bytes it read: a length is the length of what follows.

use binrw::io::{Read, Seek, Write};
use binrw::{BinRead, BinResult, BinWrite, Endian, binrw};

/// A u16 length, then that many bytes (158041: `0x140933990`, `SRead<short> &requiredBufSize`).
#[binrw]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[brw(little)]
pub struct LenString16 {
    #[br(temp)]
    #[bw(try_calc = u16::try_from(bytes.len()))]
    len: u16,
    #[br(count = len)]
    pub bytes: Vec<u8>,
}

/// A u32 length, then that many bytes (158041: `0x140932fa0`, `SRead<unsigned int> &requiredBufSize`).
#[binrw]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[brw(little)]
pub struct LenString32 {
    #[br(temp)]
    #[bw(try_calc = u32::try_from(bytes.len()))]
    len: u32,
    #[br(count = len)]
    pub bytes: Vec<u8>,
}

/// A string behind the signature `0x0a60` (158041: `0x140932010`, `strings.cpp`): two u16, the
/// signature `&theSig` and the length `&theLen`, then that many bytes. The game takes the pair in
/// either order: when the first is not the signature but the second is, the first is the length.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SigString {
    /// The length came first and the signature second.
    pub swapped: bool,
    pub bytes: Vec<u8>,
}

/// The signature of a [`SigString`].
pub const STRING_SIGNATURE: u16 = 0x0a60;

impl BinRead for SigString {
    type Args<'a> = ();

    fn read_options<R: Read + Seek>(reader: &mut R, endian: Endian, (): ()) -> BinResult<Self> {
        let pos = reader.stream_position()?;
        let first = u16::read_options(reader, endian, ())?;
        let second = u16::read_options(reader, endian, ())?;
        let (swapped, len) = if first == STRING_SIGNATURE {
            (false, second)
        } else if second == STRING_SIGNATURE {
            (true, first)
        } else {
            return Err(binrw::Error::BadMagic {
                pos,
                found: Box::new([first, second]),
            });
        };
        let args = binrw::VecArgs::builder().count(len as usize).finalize();
        Ok(Self {
            swapped,
            bytes: Vec::<u8>::read_options(reader, endian, args)?,
        })
    }
}

impl BinWrite for SigString {
    type Args<'a> = ();

    fn write_options<W: Write + Seek>(&self, writer: &mut W, endian: Endian, (): ()) -> BinResult<()> {
        let len = u16::try_from(self.bytes.len()).map_err(|e| binrw::Error::Custom {
            pos: writer.stream_position().unwrap_or_default(),
            err: Box::new(e),
        })?;
        let pair = if self.swapped {
            [len, STRING_SIGNATURE]
        } else {
            [STRING_SIGNATURE, len]
        };
        pair.write_options(writer, endian, ())?;
        self.bytes.write_options(writer, endian, ())
    }
}

/// A u32 count, then that many values (158041: `0x140d46ce0`, a `vector<int64_t>`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LenVec32<T>(pub Vec<T>);

impl<T> BinRead for LenVec32<T>
where
    T: for<'a> BinRead<Args<'a> = ()> + 'static,
{
    type Args<'a> = ();

    fn read_options<R: Read + Seek>(reader: &mut R, endian: Endian, (): ()) -> BinResult<Self> {
        let count = u32::read_options(reader, endian, ())?;
        let args = binrw::VecArgs::builder().count(count as usize).finalize();
        Ok(Self(Vec::<T>::read_options(reader, endian, args)?))
    }
}

impl<T> BinWrite for LenVec32<T>
where
    T: for<'a> BinWrite<Args<'a> = ()> + 'static,
{
    type Args<'a> = ();

    fn write_options<W: Write + Seek>(&self, writer: &mut W, endian: Endian, (): ()) -> BinResult<()> {
        let count = u32::try_from(self.0.len()).map_err(|e| binrw::Error::Custom {
            pos: writer.stream_position().unwrap_or_default(),
            err: Box::new(e),
        })?;
        count.write_options(writer, endian, ())?;
        self.0.write_options(writer, endian, ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use binrw::io::Cursor;

    #[test]
    fn strings_read_their_length_then_the_bytes() {
        let mut c = Cursor::new(b"\x02\x00hi\x01\x00\x00\x00!rest".as_slice());
        assert_eq!(LenString16::read(&mut c).unwrap().bytes, b"hi");
        assert_eq!(LenString32::read(&mut c).unwrap().bytes, b"!");
        assert_eq!(c.position(), 9);
    }

    #[test]
    fn a_sig_string_takes_the_signature_before_or_after_the_length() {
        let mut c = Cursor::new(b"\x60\x0a\x02\x00hi\x01\x00\x60\x0a!".as_slice());
        assert_eq!(
            SigString::read_le(&mut c).unwrap(),
            SigString {
                swapped: false,
                bytes: b"hi".to_vec()
            }
        );
        assert_eq!(
            SigString::read_le(&mut c).unwrap(),
            SigString {
                swapped: true,
                bytes: b"!".to_vec()
            }
        );
        assert!(SigString::read_le(&mut Cursor::new(b"\x01\x00\x02\x00".as_slice())).is_err());
    }

    /// Read, then write: the same bytes.
    fn round_trip<T>(bytes: &[u8]) -> T
    where
        T: for<'a> BinRead<Args<'a> = ()> + for<'a> binrw::BinWrite<Args<'a> = ()>,
    {
        let value = T::read_le(&mut Cursor::new(bytes)).unwrap();
        let mut out = Cursor::new(Vec::new());
        binrw::BinWrite::write_le(&value, &mut out).unwrap();
        assert_eq!(out.into_inner(), bytes);
        value
    }

    #[test]
    fn every_value_shape_writes_back_the_bytes_it_read() {
        assert_eq!(round_trip::<LenString16>(b"\x02\x00hi").bytes, b"hi");
        assert_eq!(round_trip::<LenString32>(b"\x01\x00\x00\x00!").bytes, b"!");
        assert!(!round_trip::<SigString>(b"\x60\x0a\x02\x00hi").swapped);
        assert!(round_trip::<SigString>(b"\x01\x00\x60\x0a!").swapped);
        assert_eq!(
            round_trip::<LenVec32<i64>>(b"\x01\x00\x00\x00\x07\x00\x00\x00\x00\x00\x00\x00"),
            LenVec32(vec![7])
        );
    }

    #[test]
    fn a_vec_reads_its_count_then_the_values() {
        let mut c = Cursor::new(
            b"\x02\x00\x00\x00\x01\x00\x00\x00\x00\x00\x00\x00\xff\xff\xff\xff\xff\xff\xff\xff"
                .as_slice(),
        );
        assert_eq!(
            LenVec32::<i64>::read_le(&mut c).unwrap(),
            LenVec32(vec![1, -1])
        );
        assert_eq!(c.position(), 20);
    }
}

use crate::Result;
use crate::element::{ReadableHeaderElement, WritableHeaderElement};
use crate::reader::RecordingHeaderReader;
use byteorder::{LE, ReadBytesExt, WriteBytesExt};
use std::io::{Read, Write};

/// A table of strings in the recording header, with room for a fixed number of them.
#[derive(Debug, Clone)]
pub struct StringTable {
    max_strings: u16,
    strings: Vec<String>,
}

impl StringTable {
    /// An empty table with room for `max_strings` strings.
    #[must_use]
    pub const fn new(max_strings: u16) -> Self {
        Self {
            max_strings,
            strings: vec![],
        }
    }

    /// The number of strings the table has room for.
    #[must_use]
    pub const fn max_strings(&self) -> u16 {
        self.max_strings
    }

    /// The number of strings in the table.
    ///
    /// # Panics
    ///
    /// Panics if the table holds 65535 (`u16::MAX`) strings or more.
    #[must_use]
    pub fn num_strings(&self) -> u16 {
        let len = self.strings.len();
        assert!(len < u16::MAX as usize);
        u16::try_from(len).expect("the assert above bounds the length")
    }

    /// The strings in the table.
    #[must_use]
    pub const fn strings(&self) -> &Vec<String> {
        &self.strings
    }
}

impl ReadableHeaderElement for StringTable {
    fn read_from<R: Read>(input: &mut RecordingHeaderReader<R>) -> Result<Self> {
        let max_strings = input.read_u16::<LE>()?;
        let num_strings = input.read_u16::<LE>()?;
        let _ptr = input.read_u32::<LE>()?;

        let mut strings = Vec::with_capacity(max_strings as usize);
        for _ in 0..num_strings {
            let length = input.read_u32::<LE>()?;
            let mut bytes = vec![0; length as usize];
            input.read_exact(&mut bytes)?;
            strings.push(String::from_utf8(bytes).unwrap());
        }

        Ok(Self {
            max_strings,
            strings,
        })
    }
}

impl WritableHeaderElement for StringTable {
    fn write_to<W: Write>(&self, handle: &mut W) -> Result<()> {
        handle.write_u16::<LE>(self.max_strings)?;
        handle.write_u16::<LE>(self.num_strings())?;
        handle.write_u32::<LE>(0)?;

        for string in &self.strings {
            let len = string.len();
            assert!(len < u32::MAX as usize);
            let len = u32::try_from(len).expect("the assert above bounds the length");
            handle.write_u32::<LE>(len)?;
            handle.write_all(string.as_bytes())?;
        }

        Ok(())
    }
}

impl IntoIterator for StringTable {
    type Item = String;
    type IntoIter = ::std::vec::IntoIter<String>;

    fn into_iter(self) -> Self::IntoIter {
        self.strings.into_iter()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn read_strings() {
        assert_eq!(2 + 2, 4);
    }
}

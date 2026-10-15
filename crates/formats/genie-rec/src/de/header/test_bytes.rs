//! Little-endian test inputs, built field by field.

#[derive(Default)]
pub struct Bytes(pub Vec<u8>);

impl Bytes {
    pub fn u8(mut self, v: u8) -> Self {
        self.0.push(v);
        self
    }
    pub fn i16(mut self, v: i16) -> Self {
        self.0.extend(v.to_le_bytes());
        self
    }
    pub fn u16(mut self, v: u16) -> Self {
        self.0.extend(v.to_le_bytes());
        self
    }
    pub fn i32(mut self, v: i32) -> Self {
        self.0.extend(v.to_le_bytes());
        self
    }
    pub fn u32(mut self, v: u32) -> Self {
        self.0.extend(v.to_le_bytes());
        self
    }
    pub fn f32(mut self, v: f32) -> Self {
        self.0.extend(v.to_le_bytes());
        self
    }
    pub fn zeros(mut self, n: usize) -> Self {
        self.0.extend(std::iter::repeat_n(0, n));
        self
    }
    pub fn bytes(mut self, b: &[u8]) -> Self {
        self.0.extend(b);
        self
    }
    /// A DE string: `60 0A`, the u16 length, the bytes.
    pub fn de_string(self, s: &[u8]) -> Self {
        self.u8(0x60).u8(0x0a).i16(s.len() as i16).bytes(s)
    }
}

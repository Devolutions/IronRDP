#[derive(Debug)]
pub struct ReadCursor<'a> {
    inner: &'a [u8],
    pos: usize,
}

impl<'a> ReadCursor<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { inner: bytes, pos: 0 }
    }

    pub fn len(&self) -> usize {
        self.inner.len() - self.pos
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn eof(&self) -> bool {
        self.is_empty()
    }

    pub fn remaining(&self) -> &[u8] {
        &self.inner[self.pos..]
    }

    pub fn inner(&self) -> &[u8] {
        self.inner
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn read_array<const N: usize>(&mut self) -> [u8; N] {
        let bytes = &self.inner[self.pos..self.pos + N];
        self.pos += N;
        bytes.try_into().expect("N-elements array")
    }

    pub fn read_slice(&mut self, n: usize) -> &'a [u8] {
        let bytes = &self.inner[self.pos..self.pos + n];
        self.pos += n;
        bytes
    }

    pub fn read_u8(&mut self) -> u8 {
        self.read_array::<1>()[0]
    }

    pub fn read_u16(&mut self) -> u16 {
        u16::from_le_bytes(self.read_array::<2>())
    }

    pub fn read_u16_be(&mut self) -> u16 {
        u16::from_be_bytes(self.read_array::<2>())
    }

    pub fn read_u32(&mut self) -> u32 {
        u32::from_le_bytes(self.read_array::<4>())
    }

    pub fn read_u32_be(&mut self) -> u32 {
        u32::from_be_bytes(self.read_array::<4>())
    }

    pub fn peek<const N: usize>(&mut self) -> [u8; N] {
        self.inner[self.pos..self.pos + N].try_into().expect("N-elements array")
    }

    pub fn peek_slice(&mut self, n: usize) -> &'a [u8] {
        &self.inner[self.pos..self.pos + n]
    }

    pub fn peek_u8(&mut self) -> u8 {
        self.peek::<1>()[0]
    }

    pub fn peek_u16(&mut self) -> u16 {
        u16::from_le_bytes(self.peek::<2>())
    }

    pub fn peek_u16_be(&mut self) -> u16 {
        u16::from_be_bytes(self.peek::<2>())
    }

    pub fn peek_u32(&mut self) -> u32 {
        u32::from_le_bytes(self.peek::<4>())
    }

    pub fn peek_u32_be(&mut self) -> u32 {
        u32::from_be_bytes(self.peek::<4>())
    }

    pub fn advance(&mut self, len: usize) {
        self.pos += len;
    }

    pub fn advanced(&'a self, len: usize) -> ReadCursor<'a> {
        ReadCursor {
            inner: self.inner,
            pos: self.pos + len,
        }
    }

    pub fn rewind(&mut self, len: usize) {
        self.pos -= len;
    }

    pub fn rewinded(&'a self, len: usize) -> ReadCursor<'a> {
        ReadCursor {
            inner: self.inner,
            pos: self.pos - len,
        }
    }
}

#[derive(Debug)]
pub struct WriteCursor<'a> {
    inner: &'a mut [u8],
    pos: usize,
}

impl<'a> WriteCursor<'a> {
    pub fn new(bytes: &'a mut [u8]) -> Self {
        Self { inner: bytes, pos: 0 }
    }

    pub fn len(&self) -> usize {
        self.inner.len() - self.pos
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn eof(&self) -> bool {
        self.is_empty()
    }

    pub fn remaining(&self) -> &[u8] {
        &self.inner[self.pos..]
    }

    pub fn remaining_mut(&mut self) -> &mut [u8] {
        &mut self.inner[self.pos..]
    }

    pub fn inner(&self) -> &[u8] {
        self.inner
    }

    pub fn inner_mut(&mut self) -> &mut [u8] {
        self.inner
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn write_array<const N: usize>(&mut self, array: [u8; N]) {
        self.inner[self.pos..self.pos + N].copy_from_slice(&array);
        self.pos += N;
    }

    pub fn write_slice(&mut self, slice: &[u8]) {
        let n = slice.len();
        self.inner[self.pos..self.pos + n].copy_from_slice(slice);
        self.pos += n;
    }

    pub fn write_u8(&mut self, value: u8) {
        self.write_array(value.to_le_bytes())
    }

    pub fn write_u16(&mut self, value: u16) {
        self.write_array(value.to_le_bytes())
    }

    pub fn write_u16_be(&mut self, value: u16) {
        self.write_array(value.to_be_bytes())
    }

    pub fn write_u32(&mut self, value: u32) {
        self.write_array(value.to_le_bytes())
    }

    pub fn write_u32_be(&mut self, value: u32) {
        self.write_array(value.to_be_bytes())
    }

    pub fn advance(&mut self, len: usize) {
        self.pos += len;
    }

    pub fn advanced(&'a mut self, len: usize) -> WriteCursor<'a> {
        WriteCursor {
            inner: self.inner,
            pos: self.pos + len,
        }
    }

    pub fn rewind(&mut self, len: usize) {
        self.pos -= len;
    }

    pub fn rewinded(&'a mut self, len: usize) -> WriteCursor<'a> {
        WriteCursor {
            inner: self.inner,
            pos: self.pos - len,
        }
    }
}

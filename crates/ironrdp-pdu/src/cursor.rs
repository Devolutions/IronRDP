use crate::PduResult;

#[derive(Clone, Debug)]
pub struct ReadCursor<'a> {
    inner: &'a [u8],
    pos: usize,
}

impl<'a> ReadCursor<'a> {
    #[inline]
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self { inner: bytes, pos: 0 }
    }

    #[inline]
    #[track_caller]
    pub const fn len(&self) -> usize {
        self.inner.len() - self.pos
    }

    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub const fn eof(&self) -> bool {
        self.is_empty()
    }

    #[inline]
    #[track_caller]
    pub fn remaining(&self) -> &[u8] {
        let idx = core::cmp::min(self.pos, self.inner.len());
        &self.inner[idx..]
    }

    #[inline]
    pub const fn inner(&self) -> &[u8] {
        self.inner
    }

    #[inline]
    pub const fn pos(&self) -> usize {
        self.pos
    }

    #[inline]
    #[track_caller]
    pub fn read_array<const N: usize>(&mut self) -> [u8; N] {
        let bytes = &self.inner[self.pos..self.pos + N];
        self.pos += N;
        bytes.try_into().expect("N-elements array")
    }

    #[inline]
    #[track_caller]
    pub fn read_slice(&mut self, n: usize) -> &'a [u8] {
        let bytes = &self.inner[self.pos..self.pos + n];
        self.pos += n;
        bytes
    }

    pub fn read_remaining(&mut self) -> &[u8] {
        self.read_slice(self.len())
    }

    #[inline]
    #[track_caller]
    pub fn read_u8(&mut self) -> u8 {
        self.read_array::<1>()[0]
    }

    #[inline]
    pub fn try_read_u8(&mut self, ctx: &'static str) -> PduResult<u8> {
        ensure_size!(ctx: ctx, in: self, size: 1);
        Ok(self.read_array::<1>()[0])
    }

    #[inline]
    #[track_caller]
    pub fn read_u16(&mut self) -> u16 {
        u16::from_le_bytes(self.read_array::<2>())
    }

    #[inline]
    #[track_caller]
    pub fn read_u16_be(&mut self) -> u16 {
        u16::from_be_bytes(self.read_array::<2>())
    }

    #[inline]
    pub fn try_read_u16(&mut self, ctx: &'static str) -> PduResult<u16> {
        ensure_size!(ctx: ctx, in: self, size: 2);
        Ok(u16::from_le_bytes(self.read_array::<2>()))
    }

    #[inline]
    pub fn try_read_u16_be(&mut self, ctx: &'static str) -> PduResult<u16> {
        ensure_size!(ctx: ctx, in: self, size: 2);
        Ok(u16::from_be_bytes(self.read_array::<2>()))
    }

    #[inline]
    #[track_caller]
    pub fn read_u32(&mut self) -> u32 {
        u32::from_le_bytes(self.read_array::<4>())
    }

    #[inline]
    #[track_caller]
    pub fn read_u32_be(&mut self) -> u32 {
        u32::from_be_bytes(self.read_array::<4>())
    }

    #[inline]
    pub fn try_read_u32(&mut self, ctx: &'static str) -> PduResult<u32> {
        ensure_size!(ctx: ctx, in: self, size: 4);
        Ok(u32::from_le_bytes(self.read_array::<4>()))
    }

    #[inline]
    pub fn try_read_u32_be(&mut self, ctx: &'static str) -> PduResult<u32> {
        ensure_size!(ctx: ctx, in: self, size: 4);
        Ok(u32::from_be_bytes(self.read_array::<4>()))
    }

    #[inline]
    #[track_caller]
    pub fn read_u64(&mut self) -> u64 {
        u64::from_le_bytes(self.read_array::<8>())
    }

    #[inline]
    #[track_caller]
    pub fn read_u64_be(&mut self) -> u64 {
        u64::from_be_bytes(self.read_array::<8>())
    }

    #[inline]
    pub fn try_read_u64(&mut self, ctx: &'static str) -> PduResult<u64> {
        ensure_size!(ctx: ctx, in: self, size: 8);
        Ok(u64::from_le_bytes(self.read_array::<8>()))
    }

    #[inline]
    pub fn try_read_u64_be(&mut self, ctx: &'static str) -> PduResult<u64> {
        ensure_size!(ctx: ctx, in: self, size: 8);
        Ok(u64::from_be_bytes(self.read_array::<8>()))
    }

    #[inline]
    pub fn read_i32(&mut self) -> i32 {
        i32::from_le_bytes(self.read_array::<4>())
    }

    #[inline]
    pub fn read_i32_be(&mut self) -> i32 {
        i32::from_be_bytes(self.read_array::<4>())
    }

    #[inline]
    pub fn try_read_i32(&mut self, ctx: &'static str) -> PduResult<i32> {
        ensure_size!(ctx: ctx, in: self, size: 4);
        Ok(i32::from_le_bytes(self.read_array::<4>()))
    }

    #[inline]
    pub fn try_read_i32_be(&mut self, ctx: &'static str) -> PduResult<i32> {
        ensure_size!(ctx: ctx, in: self, size: 4);
        Ok(i32::from_be_bytes(self.read_array::<4>()))
    }

    #[inline]
    pub fn read_i64(&mut self) -> i64 {
        i64::from_le_bytes(self.read_array::<8>())
    }

    #[inline]
    pub fn read_i64_be(&mut self) -> i64 {
        i64::from_be_bytes(self.read_array::<8>())
    }

    #[inline]
    pub fn try_read_i64(&mut self, ctx: &'static str) -> PduResult<i64> {
        ensure_size!(ctx: ctx, in: self, size: 8);
        Ok(i64::from_le_bytes(self.read_array::<8>()))
    }

    #[inline]
    pub fn try_read_i64_be(&mut self, ctx: &'static str) -> PduResult<i64> {
        ensure_size!(ctx: ctx, in: self, size: 8);
        Ok(i64::from_be_bytes(self.read_array::<8>()))
    }

    #[inline]
    #[track_caller]
    pub fn peek<const N: usize>(&mut self) -> [u8; N] {
        self.inner[self.pos..self.pos + N].try_into().expect("N-elements array")
    }

    #[inline]
    #[track_caller]
    pub fn peek_slice(&mut self, n: usize) -> &'a [u8] {
        &self.inner[self.pos..self.pos + n]
    }

    #[inline]
    #[track_caller]
    pub fn peek_u8(&mut self) -> u8 {
        self.peek::<1>()[0]
    }

    #[inline]
    pub fn try_peek_u8(&mut self, ctx: &'static str) -> PduResult<u8> {
        ensure_size!(ctx: ctx, in: self, size: 1);
        Ok(self.peek::<1>()[0])
    }

    #[inline]
    #[track_caller]
    pub fn peek_u16(&mut self) -> u16 {
        u16::from_le_bytes(self.peek::<2>())
    }

    #[inline]
    #[track_caller]
    pub fn peek_u16_be(&mut self) -> u16 {
        u16::from_be_bytes(self.peek::<2>())
    }

    #[inline]
    pub fn try_peek_u16(&mut self, ctx: &'static str) -> PduResult<u16> {
        ensure_size!(ctx: ctx, in: self, size: 2);
        Ok(u16::from_le_bytes(self.peek::<2>()))
    }

    #[inline]
    pub fn try_peek_u16_be(&mut self, ctx: &'static str) -> PduResult<u16> {
        ensure_size!(ctx: ctx, in: self, size: 2);
        Ok(u16::from_be_bytes(self.peek::<2>()))
    }

    #[inline]
    #[track_caller]
    pub fn peek_u32(&mut self) -> u32 {
        u32::from_le_bytes(self.peek::<4>())
    }

    #[inline]
    #[track_caller]
    pub fn peek_u32_be(&mut self) -> u32 {
        u32::from_be_bytes(self.peek::<4>())
    }

    #[inline]
    pub fn try_peek_u32(&mut self, ctx: &'static str) -> PduResult<u32> {
        ensure_size!(ctx: ctx, in: self, size: 4);
        Ok(u32::from_le_bytes(self.peek::<4>()))
    }

    #[inline]
    pub fn try_peek_u32_be(&mut self, ctx: &'static str) -> PduResult<u32> {
        ensure_size!(ctx: ctx, in: self, size: 4);
        Ok(u32::from_be_bytes(self.peek::<4>()))
    }

    #[inline]
    #[track_caller]
    pub fn peek_u64(&mut self) -> u64 {
        u64::from_le_bytes(self.peek::<8>())
    }

    #[inline]
    #[track_caller]
    pub fn peek_u64_be(&mut self) -> u64 {
        u64::from_be_bytes(self.peek::<8>())
    }

    #[inline]
    pub fn try_peek_u64(&mut self, ctx: &'static str) -> PduResult<u64> {
        ensure_size!(ctx: ctx, in: self, size: 8);
        Ok(u64::from_le_bytes(self.peek::<8>()))
    }

    #[inline]
    pub fn try_peek_u64_be(&mut self, ctx: &'static str) -> PduResult<u64> {
        ensure_size!(ctx: ctx, in: self, size: 8);
        Ok(u64::from_be_bytes(self.peek::<8>()))
    }

    #[inline]
    #[track_caller]
    pub fn advance(&mut self, len: usize) {
        self.pos += len;
    }

    #[inline]
    #[track_caller]
    #[must_use]
    pub const fn advanced(&'a self, len: usize) -> ReadCursor<'a> {
        ReadCursor {
            inner: self.inner,
            pos: self.pos + len,
        }
    }

    #[inline]
    #[track_caller]
    pub fn rewind(&mut self, len: usize) {
        self.pos -= len;
    }

    #[inline]
    #[track_caller]
    #[must_use]
    pub const fn rewinded(&'a self, len: usize) -> ReadCursor<'a> {
        ReadCursor {
            inner: self.inner,
            pos: self.pos - len,
        }
    }
}

#[cfg(feature = "std")]
impl std::io::Read for ReadCursor<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n_to_copy = core::cmp::min(buf.len(), self.len());
        let to_copy = self.read_slice(n_to_copy);
        buf.copy_from_slice(to_copy);
        Ok(n_to_copy)
    }
}

#[derive(Debug)]
pub struct WriteCursor<'a> {
    inner: &'a mut [u8],
    pos: usize,
}

impl<'a> WriteCursor<'a> {
    #[inline]
    pub fn new(bytes: &'a mut [u8]) -> Self {
        Self { inner: bytes, pos: 0 }
    }

    #[inline]
    #[track_caller]
    pub const fn len(&self) -> usize {
        self.inner.len() - self.pos
    }

    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub const fn eof(&self) -> bool {
        self.is_empty()
    }

    #[inline]
    #[track_caller]
    pub fn remaining(&self) -> &[u8] {
        let idx = core::cmp::min(self.pos, self.inner.len());
        &self.inner[idx..]
    }

    #[inline]
    #[track_caller]
    pub fn remaining_mut(&mut self) -> &mut [u8] {
        let idx = core::cmp::min(self.pos, self.inner.len());
        &mut self.inner[idx..]
    }

    #[inline]
    pub const fn inner(&self) -> &[u8] {
        self.inner
    }

    #[inline]
    pub fn inner_mut(&mut self) -> &mut [u8] {
        self.inner
    }

    #[inline]
    pub const fn pos(&self) -> usize {
        self.pos
    }

    #[inline]
    #[track_caller]
    pub fn write_array<const N: usize>(&mut self, array: [u8; N]) {
        self.inner[self.pos..self.pos + N].copy_from_slice(&array);
        self.pos += N;
    }

    #[inline]
    #[track_caller]
    pub fn write_slice(&mut self, slice: &[u8]) {
        let n = slice.len();
        self.inner[self.pos..self.pos + n].copy_from_slice(slice);
        self.pos += n;
    }

    #[inline]
    #[track_caller]
    pub fn write_u8(&mut self, value: u8) {
        self.write_array(value.to_le_bytes())
    }

    #[inline]
    #[track_caller]
    pub fn write_i8(&mut self, value: i8) {
        self.write_array(value.to_le_bytes())
    }

    #[inline]
    #[track_caller]
    pub fn write_u16(&mut self, value: u16) {
        self.write_array(value.to_le_bytes())
    }

    #[inline]
    #[track_caller]
    pub fn write_u16_be(&mut self, value: u16) {
        self.write_array(value.to_be_bytes())
    }

    #[inline]
    #[track_caller]
    pub fn write_u32(&mut self, value: u32) {
        self.write_array(value.to_le_bytes())
    }

    #[inline]
    #[track_caller]
    pub fn write_u32_be(&mut self, value: u32) {
        self.write_array(value.to_be_bytes())
    }

    #[inline]
    #[track_caller]
    pub fn write_i32(&mut self, value: i32) {
        self.write_array(value.to_le_bytes())
    }

    #[inline]
    #[track_caller]
    pub fn write_u64(&mut self, value: u64) {
        self.write_array(value.to_le_bytes())
    }

    #[inline]
    #[track_caller]
    pub fn write_u64_be(&mut self, value: u64) {
        self.write_array(value.to_be_bytes())
    }

    #[inline]
    #[track_caller]
    pub fn write_i64(&mut self, value: i64) {
        self.write_array(value.to_le_bytes())
    }

    #[inline]
    #[track_caller]
    pub fn write_i64_be(&mut self, value: i64) {
        self.write_array(value.to_be_bytes())
    }

    #[inline]
    #[track_caller]
    pub fn advance(&mut self, len: usize) {
        self.pos += len;
    }

    #[inline]
    #[track_caller]
    #[must_use]
    pub fn advanced(&'a mut self, len: usize) -> WriteCursor<'a> {
        WriteCursor {
            inner: self.inner,
            pos: self.pos + len,
        }
    }

    #[inline]
    #[track_caller]
    pub fn rewind(&mut self, len: usize) {
        self.pos -= len;
    }

    #[inline]
    #[track_caller]
    #[must_use]
    pub fn rewinded(&'a mut self, len: usize) -> WriteCursor<'a> {
        WriteCursor {
            inner: self.inner,
            pos: self.pos - len,
        }
    }
}

#[cfg(feature = "std")]
impl std::io::Write for WriteCursor<'_> {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.write_slice(buf);
        Ok(buf.len())
    }

    #[inline]
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

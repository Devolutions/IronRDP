use core::fmt;

/// Error indicating that there are not enough bytes in the buffer to perform an operation.
#[derive(Copy, Eq, PartialEq, Clone, Debug)]
pub struct NotEnoughBytesError {
    received: usize,
    expected: usize,
}

impl NotEnoughBytesError {
    /// The number of bytes received.
    #[must_use]
    #[inline]
    pub const fn received(&self) -> usize {
        self.received
    }

    /// The number of bytes expected.
    #[must_use]
    #[inline]
    pub const fn expected(&self) -> usize {
        self.expected
    }
}

impl fmt::Display for NotEnoughBytesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "not enough bytes for operation: received {} bytes, expected {} bytes",
            self.received, self.expected
        )
    }
}

#[cfg(feature = "std")]
impl core::error::Error for NotEnoughBytesError {}

macro_rules! ensure_enough_bytes {
    (in: $buf:ident, size: $expected:expr) => {{
        let received = $buf.len();
        let expected = $expected;
        if !(received >= expected) {
            return Err(NotEnoughBytesError { received, expected });
        }
    }};
}

/// A cursor for reading bytes from a buffer.
#[derive(Clone, Debug)]
pub struct ReadCursor<'a> {
    inner: &'a [u8],
    pos: usize,
}

impl<'a> ReadCursor<'a> {
    /// Create a new `ReadCursor` from a byte slice.
    #[inline]
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self { inner: bytes, pos: 0 }
    }

    /// Returns the number of bytes remaining.
    #[inline]
    #[track_caller]
    pub const fn len(&self) -> usize {
        self.inner.len() - self.pos
    }

    /// Returns `true` if there are no bytes remaining.
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns `true` if there are no bytes remaining.
    #[inline]
    pub const fn eof(&self) -> bool {
        self.is_empty()
    }

    /// Returns a slice of the remaining bytes.
    #[inline]
    #[track_caller]
    pub fn remaining(&self) -> &'a [u8] {
        let idx = core::cmp::min(self.pos, self.inner.len());
        &self.inner[idx..]
    }

    /// Returns two cursors, one with the first `mid` bytes and the other with the remaining bytes.
    #[inline]
    #[track_caller]
    #[must_use]
    pub const fn split_at_peek(&self, mid: usize) -> (ReadCursor<'a>, ReadCursor<'a>) {
        let (left, right) = self.inner.split_at(self.pos + mid);
        let left = ReadCursor {
            inner: left,
            pos: self.pos,
        };
        let right = ReadCursor { inner: right, pos: 0 };
        (left, right)
    }

    /// Returns two cursors, one with the first `mid` bytes and the other with the remaining bytes.
    ///
    /// The current cursor will be moved to the end.
    #[inline]
    #[track_caller]
    #[must_use]
    pub fn split_at(&mut self, mid: usize) -> (ReadCursor<'a>, ReadCursor<'a>) {
        let res = self.split_at_peek(mid);
        self.pos = self.inner.len();
        res
    }

    /// Return the inner byte slice.
    #[inline]
    pub const fn inner(&self) -> &[u8] {
        self.inner
    }

    /// Returns the current position.
    #[inline]
    pub const fn pos(&self) -> usize {
        self.pos
    }

    /// Read an array of `N` bytes.
    #[inline]
    #[track_caller]
    pub fn read_array<const N: usize>(&mut self) -> [u8; N] {
        let bytes = &self.inner[self.pos..self.pos + N];
        self.pos += N;
        bytes.try_into().expect("N-elements array")
    }

    /// Read a slice of `n` bytes.
    #[inline]
    #[track_caller]
    pub fn read_slice(&mut self, n: usize) -> &'a [u8] {
        let bytes = &self.inner[self.pos..self.pos + n];
        self.pos += n;
        bytes
    }

    /// Read the remaining bytes.
    pub fn read_remaining(&mut self) -> &[u8] {
        self.read_slice(self.len())
    }

    /// Read a `u8`.
    #[inline]
    #[track_caller]
    pub fn read_u8(&mut self) -> u8 {
        self.read_array::<1>()[0]
    }

    /// Try to read a `u8`.
    #[inline]
    pub fn try_read_u8(&mut self) -> Result<u8, NotEnoughBytesError> {
        ensure_enough_bytes!(in: self, size: 1);
        Ok(self.read_array::<1>()[0])
    }

    /// Read a `i16`.
    #[inline]
    #[track_caller]
    pub fn read_i16(&mut self) -> i16 {
        i16::from_le_bytes(self.read_array::<2>())
    }

    /// Read a `i16` in big-endian.
    #[inline]
    #[track_caller]
    pub fn read_i16_be(&mut self) -> i16 {
        i16::from_be_bytes(self.read_array::<2>())
    }

    /// Try to read a `i16`.
    #[inline]
    pub fn try_read_i16(&mut self) -> Result<i16, NotEnoughBytesError> {
        ensure_enough_bytes!(in: self, size: 2);
        Ok(i16::from_le_bytes(self.read_array::<2>()))
    }

    /// Try to read a `i16` in big-endian.
    #[inline]
    pub fn try_read_i16_be(&mut self) -> Result<i16, NotEnoughBytesError> {
        ensure_enough_bytes!(in: self, size: 2);
        Ok(i16::from_be_bytes(self.read_array::<2>()))
    }

    /// Read a `u16`.
    #[inline]
    #[track_caller]
    pub fn read_u16(&mut self) -> u16 {
        u16::from_le_bytes(self.read_array::<2>())
    }

    /// Read a `u16` in big-endian.
    #[inline]
    #[track_caller]
    pub fn read_u16_be(&mut self) -> u16 {
        u16::from_be_bytes(self.read_array::<2>())
    }

    /// Try to read a `u16`.
    #[inline]
    pub fn try_read_u16(&mut self) -> Result<u16, NotEnoughBytesError> {
        ensure_enough_bytes!(in: self, size: 2);
        Ok(u16::from_le_bytes(self.read_array::<2>()))
    }

    /// Try to read a `u16` in big-endian.
    #[inline]
    pub fn try_read_u16_be(&mut self) -> Result<u16, NotEnoughBytesError> {
        ensure_enough_bytes!(in: self, size: 2);
        Ok(u16::from_be_bytes(self.read_array::<2>()))
    }

    /// Read a `u32`.
    #[inline]
    #[track_caller]
    pub fn read_u32(&mut self) -> u32 {
        u32::from_le_bytes(self.read_array::<4>())
    }

    /// Read a `u32` in big-endian.
    #[inline]
    #[track_caller]
    pub fn read_u32_be(&mut self) -> u32 {
        u32::from_be_bytes(self.read_array::<4>())
    }

    /// Try to read a `u32`.
    #[inline]
    pub fn try_read_u32(&mut self) -> Result<u32, NotEnoughBytesError> {
        ensure_enough_bytes!(in: self, size: 4);
        Ok(u32::from_le_bytes(self.read_array::<4>()))
    }

    /// Try to read a `u32` in big-endian.
    #[inline]
    pub fn try_read_u32_be(&mut self) -> Result<u32, NotEnoughBytesError> {
        ensure_enough_bytes!(in: self, size: 4);
        Ok(u32::from_be_bytes(self.read_array::<4>()))
    }

    /// Read a `u64`.
    #[inline]
    #[track_caller]
    pub fn read_u64(&mut self) -> u64 {
        u64::from_le_bytes(self.read_array::<8>())
    }

    /// Read a `u64` in big-endian.
    #[inline]
    #[track_caller]
    pub fn read_u64_be(&mut self) -> u64 {
        u64::from_be_bytes(self.read_array::<8>())
    }

    /// Try to read a `u64`.
    #[inline]
    pub fn try_read_u64(&mut self) -> Result<u64, NotEnoughBytesError> {
        ensure_enough_bytes!(in: self, size: 8);
        Ok(u64::from_le_bytes(self.read_array::<8>()))
    }

    /// Try to read a `u64` in big-endian.
    #[inline]
    pub fn try_read_u64_be(&mut self) -> Result<u64, NotEnoughBytesError> {
        ensure_enough_bytes!(in: self, size: 8);
        Ok(u64::from_be_bytes(self.read_array::<8>()))
    }

    /// Read a `i32`.
    #[inline]
    pub fn read_i32(&mut self) -> i32 {
        i32::from_le_bytes(self.read_array::<4>())
    }

    /// Read a `i32` in big-endian.
    #[inline]
    pub fn read_i32_be(&mut self) -> i32 {
        i32::from_be_bytes(self.read_array::<4>())
    }

    /// Try to read a `i32`.
    #[inline]
    pub fn try_read_i32(&mut self) -> Result<i32, NotEnoughBytesError> {
        ensure_enough_bytes!(in: self, size: 4);
        Ok(i32::from_le_bytes(self.read_array::<4>()))
    }

    /// Try to read a `i32` in big-endian.
    #[inline]
    pub fn try_read_i32_be(&mut self) -> Result<i32, NotEnoughBytesError> {
        ensure_enough_bytes!(in: self, size: 4);
        Ok(i32::from_be_bytes(self.read_array::<4>()))
    }

    /// Read a `i64`.
    #[inline]
    pub fn read_i64(&mut self) -> i64 {
        i64::from_le_bytes(self.read_array::<8>())
    }

    /// Read a `i64` in big-endian.
    #[inline]
    pub fn read_i64_be(&mut self) -> i64 {
        i64::from_be_bytes(self.read_array::<8>())
    }

    /// Try to read a `i64`.
    #[inline]
    pub fn try_read_i64(&mut self) -> Result<i64, NotEnoughBytesError> {
        ensure_enough_bytes!(in: self, size: 8);
        Ok(i64::from_le_bytes(self.read_array::<8>()))
    }

    /// Try to read a `i64` in big-endian.
    #[inline]
    pub fn try_read_i64_be(&mut self) -> Result<i64, NotEnoughBytesError> {
        ensure_enough_bytes!(in: self, size: 8);
        Ok(i64::from_be_bytes(self.read_array::<8>()))
    }

    /// Read a `u128`.
    #[inline]
    pub fn read_u128(&mut self) -> u128 {
        u128::from_le_bytes(self.read_array::<16>())
    }

    /// Read a `u128` in big-endian.
    #[inline]
    pub fn read_u128_be(&mut self) -> u128 {
        u128::from_be_bytes(self.read_array::<16>())
    }

    /// Try to read a `u128`.
    #[inline]
    pub fn try_read_u128(&mut self) -> Result<u128, NotEnoughBytesError> {
        ensure_enough_bytes!(in: self, size: 16);
        Ok(u128::from_le_bytes(self.read_array::<16>()))
    }

    /// Try to read a `u128` in big-endian.
    #[inline]
    pub fn try_read_u128_be(&mut self) -> Result<u128, NotEnoughBytesError> {
        ensure_enough_bytes!(in: self, size: 16);
        Ok(u128::from_be_bytes(self.read_array::<16>()))
    }

    /// Peek at the next `N` bytes without consuming them.
    #[inline]
    #[track_caller]
    pub fn peek<const N: usize>(&mut self) -> [u8; N] {
        self.inner[self.pos..self.pos + N].try_into().expect("N-elements array")
    }

    /// Peek at the next `N` bytes without consuming them.
    #[inline]
    #[track_caller]
    pub fn peek_slice(&mut self, n: usize) -> &'a [u8] {
        &self.inner[self.pos..self.pos + n]
    }

    /// Peek a `u8` without consuming it.
    #[inline]
    #[track_caller]
    pub fn peek_u8(&mut self) -> u8 {
        self.peek::<1>()[0]
    }

    /// Try to peek a `u8` without consuming it.
    #[inline]
    pub fn try_peek_u8(&mut self) -> Result<u8, NotEnoughBytesError> {
        ensure_enough_bytes!(in: self, size: 1);
        Ok(self.peek::<1>()[0])
    }

    /// Peek a `u16` without consuming it.
    #[inline]
    #[track_caller]
    pub fn peek_u16(&mut self) -> u16 {
        u16::from_le_bytes(self.peek::<2>())
    }

    /// Peek a big-endian `u16` without consuming it.
    #[inline]
    #[track_caller]
    pub fn peek_u16_be(&mut self) -> u16 {
        u16::from_be_bytes(self.peek::<2>())
    }

    /// Try to peek a `u16` without consuming it.
    #[inline]
    pub fn try_peek_u16(&mut self) -> Result<u16, NotEnoughBytesError> {
        ensure_enough_bytes!(in: self, size: 2);
        Ok(u16::from_le_bytes(self.peek::<2>()))
    }

    /// Try to peek a big-endian `u16` without consuming it.
    #[inline]
    pub fn try_peek_u16_be(&mut self) -> Result<u16, NotEnoughBytesError> {
        ensure_enough_bytes!(in: self, size: 2);
        Ok(u16::from_be_bytes(self.peek::<2>()))
    }

    /// Peek a `u32` without consuming it.
    #[inline]
    #[track_caller]
    pub fn peek_u32(&mut self) -> u32 {
        u32::from_le_bytes(self.peek::<4>())
    }

    /// Peek a big-endian `u32` without consuming it.
    #[inline]
    #[track_caller]
    pub fn peek_u32_be(&mut self) -> u32 {
        u32::from_be_bytes(self.peek::<4>())
    }

    /// Try to peek a `u32` without consuming it.
    #[inline]
    pub fn try_peek_u32(&mut self) -> Result<u32, NotEnoughBytesError> {
        ensure_enough_bytes!(in: self, size: 4);
        Ok(u32::from_le_bytes(self.peek::<4>()))
    }

    /// Try to peek a big-endian `u32` without consuming it.
    #[inline]
    pub fn try_peek_u32_be(&mut self) -> Result<u32, NotEnoughBytesError> {
        ensure_enough_bytes!(in: self, size: 4);
        Ok(u32::from_be_bytes(self.peek::<4>()))
    }

    /// Peek a `u64` without consuming it.
    #[inline]
    #[track_caller]
    pub fn peek_u64(&mut self) -> u64 {
        u64::from_le_bytes(self.peek::<8>())
    }

    /// Peek a big-endian `u64` without consuming it.
    #[inline]
    #[track_caller]
    pub fn peek_u64_be(&mut self) -> u64 {
        u64::from_be_bytes(self.peek::<8>())
    }

    /// Try to peek a `u64` without consuming it.
    #[inline]
    pub fn try_peek_u64(&mut self) -> Result<u64, NotEnoughBytesError> {
        ensure_enough_bytes!(in: self, size: 8);
        Ok(u64::from_le_bytes(self.peek::<8>()))
    }

    /// Try to peek a big-endian `u64` without consuming it.
    #[inline]
    pub fn try_peek_u64_be(&mut self) -> Result<u64, NotEnoughBytesError> {
        ensure_enough_bytes!(in: self, size: 8);
        Ok(u64::from_be_bytes(self.peek::<8>()))
    }

    /// Advance the cursor by `len` bytes.
    #[inline]
    #[track_caller]
    pub fn advance(&mut self, len: usize) {
        self.pos += len;
    }

    /// Return a new cursor advanced by `len` bytes.
    #[inline]
    #[track_caller]
    #[must_use]
    pub const fn advanced(&'a self, len: usize) -> ReadCursor<'a> {
        ReadCursor {
            inner: self.inner,
            pos: self.pos + len,
        }
    }

    /// Rewind the cursor by `len` bytes.
    #[inline]
    #[track_caller]
    pub fn rewind(&mut self, len: usize) {
        self.pos -= len;
    }

    /// Return a new cursor rewinded by `len` bytes.
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

/// A cursor for writing bytes to a buffer.
#[derive(Debug)]
pub struct WriteCursor<'a> {
    inner: &'a mut [u8],
    pos: usize,
}

impl<'a> WriteCursor<'a> {
    /// Create a new `WriteCursor` from a mutable slice of bytes.
    #[inline]
    pub fn new(bytes: &'a mut [u8]) -> Self {
        Self { inner: bytes, pos: 0 }
    }

    /// Returns the number of bytes remaining.
    #[inline]
    #[track_caller]
    pub const fn len(&self) -> usize {
        self.inner.len() - self.pos
    }

    /// Returns `true` if there are no bytes remaining.
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns `true` if there are no bytes remaining.
    #[inline]
    pub const fn eof(&self) -> bool {
        self.is_empty()
    }

    /// Returns a slice of the remaining bytes.
    #[inline]
    #[track_caller]
    pub fn remaining(&self) -> &[u8] {
        let idx = core::cmp::min(self.pos, self.inner.len());
        &self.inner[idx..]
    }

    /// Returns a mutable slice of the remaining bytes.
    #[inline]
    #[track_caller]
    pub fn remaining_mut(&mut self) -> &mut [u8] {
        let idx = core::cmp::min(self.pos, self.inner.len());
        &mut self.inner[idx..]
    }

    /// Returns the inner byte slice.
    #[inline]
    pub const fn inner(&self) -> &[u8] {
        self.inner
    }

    /// Returns the inner mutable byte slice.
    #[inline]
    pub fn inner_mut(&mut self) -> &mut [u8] {
        self.inner
    }

    /// Returns the current position of the cursor.
    #[inline]
    pub const fn pos(&self) -> usize {
        self.pos
    }

    /// Write an array of bytes to the buffer.
    #[inline]
    #[track_caller]
    pub fn write_array<const N: usize>(&mut self, array: [u8; N]) {
        self.inner[self.pos..self.pos + N].copy_from_slice(&array);
        self.pos += N;
    }

    /// Write a slice of bytes to the buffer.
    #[inline]
    #[track_caller]
    pub fn write_slice(&mut self, slice: &[u8]) {
        let n = slice.len();
        self.inner[self.pos..self.pos + n].copy_from_slice(slice);
        self.pos += n;
    }

    /// Write a byte to the buffer.
    #[inline]
    #[track_caller]
    pub fn write_u8(&mut self, value: u8) {
        self.write_array(value.to_le_bytes())
    }

    /// Write a signed byte to the buffer.
    #[inline]
    #[track_caller]
    pub fn write_i8(&mut self, value: i8) {
        self.write_array(value.to_le_bytes())
    }

    /// Write a little-endian `u16` to the buffer.
    #[inline]
    #[track_caller]
    pub fn write_u16(&mut self, value: u16) {
        self.write_array(value.to_le_bytes())
    }

    /// Write a big-endian `u16` to the buffer.
    #[inline]
    #[track_caller]
    pub fn write_u16_be(&mut self, value: u16) {
        self.write_array(value.to_be_bytes())
    }

    /// Write a signed little-endian `i16` to the buffer.
    #[inline]
    #[track_caller]
    pub fn write_i16(&mut self, value: i16) {
        self.write_array(value.to_le_bytes())
    }

    /// Write a signed big-endian `i16` to the buffer.
    #[inline]
    #[track_caller]
    pub fn write_i16_be(&mut self, value: i16) {
        self.write_array(value.to_be_bytes())
    }

    /// Write a little-endian `u32` to the buffer.
    #[inline]
    #[track_caller]
    pub fn write_u32(&mut self, value: u32) {
        self.write_array(value.to_le_bytes())
    }

    /// Write a big-endian `u32` to the buffer.
    #[inline]
    #[track_caller]
    pub fn write_u32_be(&mut self, value: u32) {
        self.write_array(value.to_be_bytes())
    }

    /// Write a signed little-endian `i32` to the buffer.
    #[inline]
    #[track_caller]
    pub fn write_i32(&mut self, value: i32) {
        self.write_array(value.to_le_bytes())
    }

    /// Write a little-endian `u64` to the buffer.
    #[inline]
    #[track_caller]
    pub fn write_u64(&mut self, value: u64) {
        self.write_array(value.to_le_bytes())
    }

    /// Write a big-endian `u64` to the buffer.
    #[inline]
    #[track_caller]
    pub fn write_u64_be(&mut self, value: u64) {
        self.write_array(value.to_be_bytes())
    }

    /// Write a signed little-endian `i64` to the buffer.
    #[inline]
    #[track_caller]
    pub fn write_i64(&mut self, value: i64) {
        self.write_array(value.to_le_bytes())
    }

    /// Write a signed big-endian `i64` to the buffer.
    #[inline]
    #[track_caller]
    pub fn write_i64_be(&mut self, value: i64) {
        self.write_array(value.to_be_bytes())
    }

    /// Write a little-endian `u128` to the buffer.
    #[inline]
    #[track_caller]
    pub fn write_u128(&mut self, value: u128) {
        self.write_array(value.to_le_bytes())
    }

    /// Write a big-endian `u128` to the buffer.
    #[inline]
    #[track_caller]
    pub fn write_u128_be(&mut self, value: u128) {
        self.write_array(value.to_be_bytes())
    }

    /// Advance the cursor by `len` bytes.
    #[inline]
    #[track_caller]
    pub fn advance(&mut self, len: usize) {
        self.pos += len;
    }

    /// Returns a new cursor advanced by `len` bytes.
    #[inline]
    #[track_caller]
    #[must_use]
    pub fn advanced(&'a mut self, len: usize) -> WriteCursor<'a> {
        WriteCursor {
            inner: self.inner,
            pos: self.pos + len,
        }
    }

    /// Rewind the cursor by `len` bytes.
    #[inline]
    #[track_caller]
    pub fn rewind(&mut self, len: usize) {
        self.pos -= len;
    }

    /// Returns a new cursor rewinded by `len` bytes.
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

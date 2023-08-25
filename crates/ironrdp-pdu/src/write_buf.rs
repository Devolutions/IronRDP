use std::ops::{Index, Range, RangeFrom, RangeFull, RangeInclusive, RangeTo, RangeToInclusive};

/// Max capacity to keep for the inner Vec<u8> when `WriteBuf::clear` is called.
const MAX_CAPACITY_WHEN_CLEARED: usize = 16384; // 16 kib

/// Growable buffer backed by a [`Vec<u8>`] that is incrementally filled.
///
/// This type is tracking the filled region and provides methods to
/// grow and write into the unfilled region.
///
/// Memory layout can be visualized as:
///
/// ```not_rust
/// [          Vec capacity             ]
/// [ filled | unfilled |               ]
/// [    initialized    | uninitialized ]
/// ```
pub struct WriteBuf {
    inner: Vec<u8>,
    filled: usize,
}

impl WriteBuf {
    /// Constructs a new, empty `WriteBuf`.
    ///
    /// The underlying buffer will not allocate until bytes are written to it.
    pub const fn new() -> Self {
        Self {
            inner: Vec::new(),
            filled: 0,
        }
    }

    pub const fn from_vec(buffer: Vec<u8>) -> Self {
        Self {
            inner: buffer,
            filled: 0,
        }
    }

    pub fn into_inner(self) -> Vec<u8> {
        self.inner
    }

    /// Returns length of the filled region.
    ///
    /// This is always equal to the starting index for the unfilled initialized portion of the buffer.
    pub const fn filled_len(&self) -> usize {
        self.filled
    }

    /// Returns a shared reference to the filled portion of the buffer.
    pub fn filled(&self) -> &[u8] {
        &self.inner[..self.filled]
    }

    /// Ensures initialized and unfilled portion of the buffer is big enough for `additional` more bytes.
    pub fn initialize(&mut self, additional: usize) {
        if self.inner.len() < self.filled + additional {
            self.inner.resize(self.filled + additional, 0);
        }
    }

    /// Returns a mutable reference to the first n bytes of the unfilled part of the buffer,
    /// allocating additional memory as necessary.
    pub fn unfilled_to(&mut self, n: usize) -> &mut [u8] {
        self.initialize(n);
        &mut self.inner[self.filled..self.filled + n]
    }

    /// Returns a mutable reference to the unfilled part of the buffer.
    pub fn unfilled_mut(&mut self) -> &mut [u8] {
        &mut self.inner[self.filled..]
    }

    pub fn write_array<const N: usize>(&mut self, array: [u8; N]) {
        self.initialize(N);
        self.inner[self.filled..self.filled + N].copy_from_slice(&array);
        self.filled += N;
    }

    pub fn write_slice(&mut self, slice: &[u8]) {
        let n = slice.len();
        self.initialize(n);
        self.inner[self.filled..self.filled + n].copy_from_slice(slice);
        self.filled += n;
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

    pub fn write_u64(&mut self, value: u64) {
        self.write_array(value.to_le_bytes())
    }

    pub fn write_u64_be(&mut self, value: u64) {
        self.write_array(value.to_be_bytes())
    }

    /// Set the filled cursor to the very beginning of the buffer.
    ///
    /// If the buffer grew big, it is shrinked in order to reclaim memory.
    pub fn clear(&mut self) {
        self.filled = 0;
        self.inner.shrink_to(MAX_CAPACITY_WHEN_CLEARED);
    }

    /// Advances the buffer’s cursor of `len` bytes.
    pub fn advance(&mut self, len: usize) {
        self.filled += len;
        debug_assert!(self.filled <= self.inner.len());
    }
}

#[cfg(feature = "std")]
impl std::io::Write for WriteBuf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.write_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// Allows the user to get a slice of the filled region using indexing operations (e.g.: buf[..], buf[..10], buf[2..8]).

impl Index<Range<usize>> for WriteBuf {
    type Output = [u8];

    fn index(&self, range: Range<usize>) -> &Self::Output {
        &self.filled()[range]
    }
}

impl Index<RangeFrom<usize>> for WriteBuf {
    type Output = [u8];

    fn index(&self, range: RangeFrom<usize>) -> &Self::Output {
        &self.filled()[range]
    }
}

impl Index<RangeFull> for WriteBuf {
    type Output = [u8];

    fn index(&self, _: RangeFull) -> &Self::Output {
        self.filled()
    }
}

impl Index<RangeInclusive<usize>> for WriteBuf {
    type Output = [u8];

    fn index(&self, range: RangeInclusive<usize>) -> &Self::Output {
        &self.filled()[range]
    }
}

impl Index<RangeTo<usize>> for WriteBuf {
    type Output = [u8];

    fn index(&self, range: RangeTo<usize>) -> &Self::Output {
        &self.filled()[range]
    }
}

impl Index<RangeToInclusive<usize>> for WriteBuf {
    type Output = [u8];

    fn index(&self, range: RangeToInclusive<usize>) -> &Self::Output {
        &self.filled()[range]
    }
}

/// MSB-first bit-reader.
#[cfg_attr(feature = "__test", visibility::make(pub))]
pub(crate) struct BitReader<'a> {
    /// Bytes not yet loaded into `acc`.
    data: &'a [u8],
    /// Buffered bits. The next bit to be read is bit 63, and every
    /// bit below `64 - len` is zero.
    acc: u64,
    /// Number of valid bits in `acc`, at most 64.
    len: u32,
}

impl<'a> BitReader<'a> {
    #[cfg_attr(feature = "__test", visibility::make(pub))]
    pub(crate) fn new(data: &'a [u8]) -> Self {
        let mut this = Self { data, acc: 0, len: 0 };
        this.refill();
        this
    }

    /// Tops `acc` up to more than 56 bits when input remains, so any read of up
    /// to 32 bits is served without a second bounds check.
    #[inline]
    fn refill(&mut self) {
        while self.len <= 56 {
            let Some((&byte, rest)) = self.data.split_first() else {
                break;
            };
            self.data = rest;
            self.acc |= u64::from(byte) << (56 - self.len);
            self.len += 8;
        }
    }

    #[inline]
    #[cfg_attr(feature = "__test", visibility::make(pub))]
    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0 && self.data.is_empty()
    }

    /// Consumes `n` buffered bits. `n` must not exceed `self.len`, and `n < 64`
    /// (shifting a `u64` by 64 or more overflows).
    #[inline]
    fn consume(&mut self, n: u32) {
        debug_assert!(n < 64 && n <= self.len);
        self.acc <<= n;
        self.len -= n;
    }

    /// Reads `n` bits (`n <= 32`) MSB-first, or returns `None` without consuming
    /// anything if fewer than `n` bits remain.
    #[inline]
    #[cfg_attr(feature = "__test", visibility::make(pub))]
    pub(crate) fn read(&mut self, n: u32) -> Option<u32> {
        debug_assert!(n <= 32);
        if n == 0 {
            return Some(0);
        }
        self.refill();
        if self.len < n {
            return None;
        }
        #[expect(
            clippy::cast_possible_truncation,
            clippy::as_conversions,
            reason = "n <= 32, so the shift leaves 32 bits"
        )]
        let value = (self.acc >> (64 - n)) as u32;
        self.consume(n);
        Some(value)
    }

    /// Counts and consumes leading bits equal to `value`, leaving the
    /// terminating opposite bit for the caller to read.
    #[inline]
    #[cfg_attr(feature = "__test", visibility::make(pub))]
    pub(crate) fn skip_leading(&mut self, value: bool) -> usize {
        let mut total = 0usize;

        loop {
            self.refill();
            if self.len == 0 {
                return total;
            }

            // Count leading zeros either way by inverting for a run of ones.
            let word = if value { !self.acc } else { self.acc };
            let run = word.leading_zeros().min(self.len);
            total += usize::try_from(run).unwrap_or(usize::MAX);

            if run < self.len {
                // The terminator is in the buffer. Leave it unconsumed.
                self.consume(run);
                return total;
            }

            // The whole buffer matched. Drop it and look at the next one.
            self.acc = 0;
            self.len = 0;
        }
    }
}

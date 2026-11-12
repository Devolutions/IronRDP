use ironrdp_graphics::BitReader;

/// Reference implementation that reads one bit at a time.
struct NaiveReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl NaiveReader<'_> {
    fn remaining(&self) -> usize {
        self.data.len() * 8 - self.pos
    }

    fn bit(&self, pos: usize) -> bool {
        self.data[pos / 8] & (0x80 >> (pos % 8)) != 0
    }

    fn read(&mut self, n: u32) -> Option<u32> {
        let n = usize::try_from(n).unwrap();
        if self.remaining() < n {
            return None;
        }
        let mut value = 0u32;
        for _ in 0..n {
            value = (value << 1) | u32::from(self.bit(self.pos));
            self.pos += 1;
        }
        Some(value)
    }

    fn skip_leading(&mut self, value: bool) -> usize {
        let start = self.pos;
        while self.pos < self.data.len() * 8 && self.bit(self.pos) == value {
            self.pos += 1;
        }
        self.pos - start
    }
}

#[test]
fn reads_are_msb_first_and_cross_byte_boundaries() {
    let mut r = BitReader::new(&[0b1010_1100, 0b0101_0011]);
    assert_eq!(r.read(1), Some(1));
    assert_eq!(r.read(3), Some(0b010));
    assert_eq!(r.read(8), Some(0b1100_0101));
    assert_eq!(r.read(4), Some(0b0011));
    assert!(r.is_empty());
}

#[test]
fn zero_bit_read_always_succeeds() {
    let mut r = BitReader::new(&[]);
    assert_eq!(r.read(0), Some(0));
    assert!(r.is_empty());
}

#[test]
fn short_read_fails_without_consuming() {
    let mut r = BitReader::new(&[0xAB]);
    assert_eq!(r.read(9), None);
    assert!(!r.is_empty());
    assert_eq!(r.read(8), Some(0xAB));
    assert_eq!(r.read(1), None);
    assert!(r.is_empty());
}

#[test]
fn reads_of_32_bits_span_refills() {
    let data: Vec<u8> = (0..12).collect();
    let mut r = BitReader::new(&data);
    assert_eq!(r.read(32), Some(0x0001_0203));
    assert_eq!(r.read(32), Some(0x0405_0607));
    assert_eq!(r.read(32), Some(0x0809_0A0B));
    assert!(r.is_empty());
}

#[test]
fn skip_leading_zeros_leaves_the_terminator() {
    let mut r = BitReader::new(&[0b0001_0000]);
    assert_eq!(r.skip_leading(false), 3);
    assert_eq!(r.read(1), Some(1));
}

#[test]
fn skip_leading_ones_leaves_the_terminator() {
    let mut r = BitReader::new(&[0b1110_1000]);
    assert_eq!(r.skip_leading(true), 3);
    assert_eq!(r.read(1), Some(0));
    assert_eq!(r.read(3), Some(0b100));
}

#[test]
fn skip_leading_with_no_run_consumes_nothing() {
    let mut r = BitReader::new(&[0b1000_0000]);
    assert_eq!(r.skip_leading(false), 0);
    assert_eq!(r.read(1), Some(1));
}

#[test]
fn skip_leading_spans_many_refills() {
    let mut data = vec![0u8; 20];
    data.push(0b0010_0000);
    let mut r = BitReader::new(&data);
    assert_eq!(r.skip_leading(false), 20 * 8 + 2);
    assert_eq!(r.read(1), Some(1));

    let mut data = vec![0xFFu8; 20];
    data.push(0b1111_1011);
    let mut r = BitReader::new(&data);
    assert_eq!(r.skip_leading(true), 20 * 8 + 5);
    assert_eq!(r.read(1), Some(0));
}

#[test]
fn skip_leading_run_ending_exactly_at_buffer_boundary() {
    // The first refill buffers 8 bytes (64 bits), so the run ends where the
    // terminator is the first bit of the next buffer.
    let mut data = vec![0u8; 8];
    data.push(0x80);
    let mut r = BitReader::new(&data);
    assert_eq!(r.skip_leading(false), 64);
    assert_eq!(r.read(1), Some(1));
}

#[test]
fn skip_leading_stops_at_end_of_input() {
    let mut r = BitReader::new(&[0, 0, 0]);
    assert_eq!(r.skip_leading(false), 24);
    assert!(r.is_empty());
    assert_eq!(r.read(1), None);

    let mut r = BitReader::new(&[]);
    assert_eq!(r.skip_leading(true), 0);
}

#[test]
fn matches_a_bit_at_a_time_reference() {
    let mut state = 0x9E37_79B9u32;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state
    };

    for len in [0usize, 1, 7, 8, 9, 15, 33, 100] {
        for _ in 0..50 {
            // Bias towards long runs so `skip_leading` is exercised.
            let data: Vec<u8> = core::iter::repeat_with(|| match next() % 4 {
                0 => 0x00,
                1 => 0xFF,
                _ => next().to_le_bytes()[0],
            })
            .take(len)
            .collect();

            let mut fast = BitReader::new(&data);
            let mut naive = NaiveReader { data: &data, pos: 0 };

            for _ in 0..(len * 8 + 4) {
                match next() % 3 {
                    0 => {
                        let n = next() % 33;
                        assert_eq!(fast.read(n), naive.read(n), "read({n}) on {data:02x?}");
                    }
                    1 => assert_eq!(
                        fast.skip_leading(false),
                        naive.skip_leading(false),
                        "zeros on {data:02x?}"
                    ),
                    _ => assert_eq!(fast.skip_leading(true), naive.skip_leading(true), "ones on {data:02x?}"),
                }
                assert_eq!(fast.is_empty(), naive.remaining() == 0, "is_empty on {data:02x?}");
            }
        }
    }
}

use super::BPP;

const TASKBAR_HEIGHT: usize = 40;
const LINE_HEIGHT: usize = 13;
const GLYPH_WIDTH: usize = 7;

/// Synthetic screen content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pattern {
    /// Mostly flat desktop: solid background, a few windows, borders, a taskbar.
    /// The common case for an idle or lightly-used session.
    Desktop,
    /// Dense small text on white, representing terminals, editors, and spreadsheets.
    Text,
    /// Smooth 2D gradient.
    Gradient,
    /// Photo-like content: smooth blobs plus fine detail.
    Photo,
    /// Pseudorandom bytes. Not realistic, but exercises the RLGR phase well.
    Noise,
}

impl Pattern {
    pub const ALL: [Self; 5] = [Self::Desktop, Self::Text, Self::Gradient, Self::Photo, Self::Noise];

    pub fn name(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Text => "text",
            Self::Gradient => "gradient",
            Self::Photo => "photo",
            Self::Noise => "noise",
        }
    }

    pub fn from_u32(value: u32) -> Option<Self> {
        usize::try_from(value).ok().and_then(|i| Self::ALL.get(i)).copied()
    }
}

/// Deterministic xorshift, so fixtures are reproducible across runs.
struct Rng(u32);

impl Rng {
    fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
}

/// Renders `pattern` into a `width * height` RGBA buffer.
pub fn render(pattern: Pattern, width: usize, height: usize) -> Vec<u8> {
    let mut buf = vec![0u8; width * height * BPP];
    let mut rng = Rng(0x1234_5678);

    for (i, px) in buf.chunks_exact_mut(BPP).enumerate() {
        let x = i % width;
        let y = i / width;
        let [r, g, b] = match pattern {
            Pattern::Desktop => desktop_px(x, y, width, height),
            Pattern::Text => text_px(x, y),
            Pattern::Gradient => gradient_px(x, y, width, height),
            Pattern::Photo => photo_px(x, y),
            Pattern::Noise => {
                let [r, g, b, _] = rng.next_u32().to_le_bytes();
                [r, g, b]
            }
        };
        px.copy_from_slice(&[r, g, b, 0xff]);
    }

    buf
}

fn desktop_px(x: usize, y: usize, width: usize, height: usize) -> [u8; 3] {
    struct Window {
        x: usize,
        y: usize,
        width: usize,
        height: usize,
    }

    const WINDOWS: [Window; 2] = [
        Window {
            x: 60,
            y: 50,
            width: 700,
            height: 500,
        },
        Window {
            x: 420,
            y: 260,
            width: 760,
            height: 560,
        },
    ];

    // Taskbar.
    if height <= y + TASKBAR_HEIGHT {
        return if y + TASKBAR_HEIGHT == height {
            [0x50, 0x50, 0x50]
        } else {
            [0x20, 0x20, 0x20]
        };
    }

    // Two overlapping windows with title bars, borders and a text region.
    for (i, window) in WINDOWS.iter().enumerate() {
        if window.x <= x
            && x < (window.x + window.width).min(width)
            && window.y <= y
            && y < (window.y + window.height).min(height)
        {
            let lx = x - window.x;
            let ly = y - window.y;
            if ly < 2 || lx < 2 || window.width <= lx + 2 || window.height <= ly + 2 {
                return [0x70, 0x70, 0x78]; // border
            }
            if ly < 32 {
                return if i == 0 { [0x1f, 0x5f, 0x9f] } else { [0x2f, 0x2f, 0x3f] }; // title bar
            }
            // Client area: white with text lines in the upper half.
            if ly < window.height / 2 {
                return text_px(lx, ly);
            }
            return [0xf4, 0xf4, 0xf4];
        }
    }

    // Desktop background: a very slow gradient, as most themes have.
    let v = 0x30 + (x + y) * 0x20 / (width + height);
    [channel(v / 3), channel(v / 2), channel(v)]
}

/// Narrow a computed channel value to a byte.
fn channel(v: usize) -> u8 {
    u8::try_from(v).unwrap_or(u8::MAX)
}

/// Simulates a text-like image: dense small (7-pixel) glyphs on a solid background.
fn text_px(x: usize, y: usize) -> [u8; 3] {
    let line = y / LINE_HEIGHT;
    let row = y % LINE_HEIGHT;
    if 9 <= row {
        return [0xff, 0xff, 0xff]; // leading between lines
    }
    let cell = x / GLYPH_WIDTH;
    let col = x % GLYPH_WIDTH;
    if 6 <= col {
        return [0xff, 0xff, 0xff]; // inter-character gap
    }
    // Deterministic per-glyph bitmap.
    let h = (cell.wrapping_mul(2_654_435_761) ^ line.wrapping_mul(40_503) ^ row.wrapping_mul(2_246_822_519)) >> 7;
    if h & 1 != 0 || (h & 6) == 6 {
        [0x11, 0x11, 0x11]
    } else {
        [0xff, 0xff, 0xff]
    }
}

fn gradient_px(x: usize, y: usize, width: usize, height: usize) -> [u8; 3] {
    [
        channel(x * 255 / width.max(1)),
        channel(y * 255 / height.max(1)),
        channel((x + y) * 255 / (width + height).max(1)),
    ]
}

fn photo_px(x: usize, y: usize) -> [u8; 3] {
    // Cheap integer simulation of a photo with smooth blobs and fine detail.
    let a = (x * x + y * y) >> 6;
    let b = (x * 3 + y * 7) >> 1;
    let c = (x ^ y) & 0x1f;
    [
        channel(a % 200 + 28),
        channel(b % 180 + 40),
        channel((a / 3 + b / 5 + c) % 210 + 20),
    ]
}

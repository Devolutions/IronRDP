//! Inline terminal rendering of screenshots using the Sixel, Kitty, or iTerm2 graphics protocols.

use core::fmt::Write as _;
use std::io::Write;

use anyhow::Context as _;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use clap::ValueEnum;

/// Cell size in pixels assumed when the terminal does not report it: the VT340 cell that Windows
/// Terminal uses to scale Sixel images.
pub(crate) const FALLBACK_CELL_SIZE: (u32, u32) = (10, 20);
const FALLBACK_CELL_WIDTH: u32 = FALLBACK_CELL_SIZE.0;

/// Maximum number of Sixel color registers used; the palette is built per image.
const SIXEL_COLORS: usize = 256;

/// Bits kept per channel when building the color histogram (5 → 32768 buckets).
const HISTOGRAM_BITS: u32 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum Protocol {
    /// Detect the protocol from the terminal environment.
    Auto,
    /// DEC Sixel graphics (Windows Terminal, foot, mlterm, WezTerm, …).
    Sixel,
    /// Kitty graphics protocol (Kitty, Ghostty, WezTerm, …).
    Kitty,
    /// iTerm2 inline images (iTerm2, WezTerm, VS Code, …).
    Iterm2,
}

/// Writes `png` to `out` as an inline image spanning at most `columns` terminal cells.
///
/// When `columns` is `None`, the current terminal width is used.
pub(crate) fn render(out: &mut dyn Write, png: &[u8], protocol: Protocol, columns: Option<u16>) -> anyhow::Result<()> {
    let protocol = resolve_protocol(protocol)?;

    let (terminal_columns, cell_width) = terminal_geometry();
    let columns = columns.unwrap_or(terminal_columns).max(1);

    write_image(
        out,
        png,
        protocol,
        columns,
        u32::from(columns).saturating_mul(cell_width),
        "",
    )?;

    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}

/// Resolves [`Protocol::Auto`] from the terminal environment.
pub(crate) fn resolve_protocol(protocol: Protocol) -> anyhow::Result<Protocol> {
    match protocol {
        Protocol::Auto => detect_protocol()
            .context("could not detect a terminal graphics protocol; pass --protocol sixel, kitty, or iterm2"),
        explicit => Ok(explicit),
    }
}

/// Returns the pixel size of one terminal cell, if the terminal reports it.
pub(crate) fn cell_size() -> Option<(u32, u32)> {
    let size = crossterm::terminal::window_size().ok()?;
    if size.columns == 0 || size.rows == 0 {
        return None;
    }
    let width = u32::from(size.width) / u32::from(size.columns);
    let height = u32::from(size.height) / u32::from(size.rows);
    (width > 0 && height > 0).then_some((width, height))
}

/// Writes `png` at the cursor position, `columns` cells wide.
///
/// Sixel images are resampled to at most `sixel_width` pixels. `kitty_keys` is prepended to the
/// Kitty control data (for example `i=1,q=2,`) so callers can replace a previous image in place.
pub(crate) fn write_image(
    out: &mut dyn Write,
    png: &[u8],
    protocol: Protocol,
    columns: u16,
    sixel_width: u32,
    kitty_keys: &str,
) -> anyhow::Result<()> {
    match protocol {
        Protocol::Kitty => {
            // Kitty requires transmissions to be split into chunks of at most 4096 base64 bytes.
            let encoded = BASE64.encode(png);
            let mut chunks = encoded.as_bytes().chunks(4096).peekable();
            let mut first = true;
            while let Some(chunk) = chunks.next() {
                let more = u8::from(chunks.peek().is_some());
                if first {
                    write!(out, "\x1b_G{kitty_keys}f=100,a=T,c={columns},m={more};")?;
                    first = false;
                } else {
                    write!(out, "\x1b_Gm={more};")?;
                }
                out.write_all(chunk)?;
                out.write_all(b"\x1b\\")?;
            }
        }
        Protocol::Iterm2 => {
            write!(
                out,
                "\x1b]1337;File=inline=1;size={};width={columns};preserveAspectRatio=1:{}\x07",
                png.len(),
                BASE64.encode(png),
            )?;
        }
        Protocol::Sixel => {
            let image = decode_rgb(png)?.fit_width(sixel_width);
            out.write_all(encode_sixel(&image).as_bytes())?;
        }
        Protocol::Auto => anyhow::bail!("terminal graphics protocol must be resolved before writing an image"),
    }

    Ok(())
}

fn detect_protocol() -> Option<Protocol> {
    let var = |name: &str| std::env::var(name).unwrap_or_default();
    let term = var("TERM").to_ascii_lowercase();
    let term_program = var("TERM_PROGRAM").to_ascii_lowercase();

    if std::env::var_os("KITTY_WINDOW_ID").is_some() || term.contains("kitty") || term_program == "ghostty" {
        Some(Protocol::Kitty)
    } else if matches!(term_program.as_str(), "iterm.app" | "wezterm" | "vscode") {
        Some(Protocol::Iterm2)
    } else if std::env::var_os("WT_SESSION").is_some()
        || term.contains("sixel")
        || term.starts_with("foot")
        || term.starts_with("mlterm")
    {
        Some(Protocol::Sixel)
    } else {
        None
    }
}

/// Returns the terminal width in columns and the width of one cell in pixels.
fn terminal_geometry() -> (u16, u32) {
    let columns = crossterm::terminal::size().map_or(80, |(columns, _)| columns);
    let cell_width = cell_size().map_or(FALLBACK_CELL_WIDTH, |(width, _)| width);
    (columns, cell_width)
}

struct RgbImage {
    width: u32,
    height: u32,
    /// Packed 8-bit RGB samples, row-major.
    pixels: Vec<u8>,
}

impl RgbImage {
    /// Box-filters the image down so that it is at most `max_width` pixels wide, preserving the
    /// aspect ratio. Images that already fit are returned unchanged.
    fn fit_width(self, max_width: u32) -> Self {
        if max_width == 0 || self.width <= max_width {
            return self;
        }

        let width = max_width;
        let height = (u64::from(self.height) * u64::from(width) / u64::from(self.width)).max(1);
        let height = u32::try_from(height).expect("scaled height is at most the source height");

        let mut pixels = Vec::with_capacity(pixel_index(width, height, 0, 0));
        for y in 0..height {
            let y0 = scale(y, self.height, height);
            let y1 = scale(y + 1, self.height, height).max(y0 + 1);
            for x in 0..width {
                let x0 = scale(x, self.width, width);
                let x1 = scale(x + 1, self.width, width).max(x0 + 1);

                let mut sum = [0u32; 3];
                for sy in y0..y1 {
                    for sx in x0..x1 {
                        let offset = pixel_index(self.width, sy, sx, 0);
                        for (channel, sum) in sum.iter_mut().enumerate() {
                            *sum += u32::from(self.pixels[offset + channel]);
                        }
                    }
                }

                let count = (y1 - y0) * (x1 - x0);
                pixels.extend(sum.map(|sum| u8::try_from(sum / count).expect("average of 8-bit samples fits in u8")));
            }
        }

        Self { width, height, pixels }
    }
}

/// Maps destination coordinate `position` on an axis of length `destination` onto a source axis of
/// length `source`.
fn scale(position: u32, source: u32, destination: u32) -> u32 {
    u32::try_from(u64::from(position) * u64::from(source) / u64::from(destination))
        .expect("scaled coordinate is at most the source length")
}

/// Byte offset of `channel` in pixel `(x, y)` of a packed RGB image `width` pixels wide.
fn pixel_index(width: u32, y: u32, x: u32, channel: usize) -> usize {
    let pixel = usize::try_from(u64::from(y) * u64::from(width) + u64::from(x)).expect("image fits in memory");
    pixel * 3 + channel
}

fn decode_rgb(png: &[u8]) -> anyhow::Result<RgbImage> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().context("decode screenshot PNG header")?;
    let mut buffer = vec![0; reader.output_buffer_size().context("screenshot PNG is too large")?];
    let info = reader.next_frame(&mut buffer).context("decode screenshot PNG")?;
    let samples = &buffer[..info.buffer_size()];

    let pixels = match info.color_type {
        png::ColorType::Rgb => samples.to_vec(),
        png::ColorType::Rgba => samples
            .chunks_exact(4)
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
            .collect(),
        png::ColorType::Grayscale => samples.iter().flat_map(|&gray| [gray; 3]).collect(),
        png::ColorType::GrayscaleAlpha => samples.chunks_exact(2).flat_map(|pixel| [pixel[0]; 3]).collect(),
        png::ColorType::Indexed => anyhow::bail!("indexed PNG was not expanded to RGB"),
    };

    Ok(RgbImage {
        width: info.width,
        height: info.height,
        pixels,
    })
}

/// Encodes `image` as a Sixel sequence with a median-cut palette of up to 256 colors built from the
/// image, and Floyd–Steinberg dithering for gradients the palette cannot represent exactly.
fn encode_sixel(image: &RgbImage) -> String {
    let width = usize::try_from(image.width).expect("image width fits in usize");
    let height = usize::try_from(image.height).expect("image height fits in usize");
    let palette = median_cut_palette(&image.pixels);
    let indices = dither(image, &palette);

    let mut out = String::new();
    // DCS with a 1:1 pixel aspect ratio, followed by the raster attributes.
    let _ = write!(out, "\x1bP0;1;0q\"1;1;{};{}", image.width, image.height);
    for (index, color) in palette.iter().enumerate() {
        let percent = |sample: u8| (u32::from(sample) * 100 + 127) / 255;
        let _ = write!(
            out,
            "#{index};2;{};{};{}",
            percent(color[0]),
            percent(color[1]),
            percent(color[2])
        );
    }

    let mut row_bits = vec![0u8; width];
    for band in (0..height).step_by(6) {
        let rows = band..(band + 6).min(height);

        let mut present = [false; SIXEL_COLORS];
        for y in rows.clone() {
            for &index in &indices[y * width..(y + 1) * width] {
                present[usize::from(index)] = true;
            }
        }

        for color in (0..palette.len()).filter(|&color| present[color]) {
            for (x, bits) in row_bits.iter_mut().enumerate() {
                *bits = rows
                    .clone()
                    .filter(|&y| usize::from(indices[y * width + x]) == color)
                    .fold(0, |bits, y| bits | (1 << (y - band)));
            }

            let _ = write!(out, "#{color}");
            let mut x = 0;
            while x < width {
                let bits = row_bits[x];
                let run = row_bits[x..].iter().take_while(|&&next| next == bits).count();
                let sixel = char::from(0x3F + bits);
                if run > 3 {
                    let _ = write!(out, "!{run}{sixel}");
                } else {
                    out.extend(core::iter::repeat_n(sixel, run));
                }
                x += run;
            }
            // Graphics carriage return: overlay the next color on the same band.
            out.push('$');
        }
        // Graphics new line: advance to the next six-pixel band.
        out.push('-');
    }
    out.push_str("\x1b\\");
    out
}

/// Histogram bucket of an RGB color, keeping [`HISTOGRAM_BITS`] per channel.
fn bucket(rgb: [u8; 3]) -> usize {
    let shift = 8 - HISTOGRAM_BITS;
    rgb.iter()
        .fold(0, |key, &sample| (key << HISTOGRAM_BITS) | usize::from(sample >> shift))
}

/// Builds a palette of at most [`SIXEL_COLORS`] colors by median cut over a color histogram.
fn median_cut_palette(pixels: &[u8]) -> Vec<[u8; 3]> {
    #[derive(Clone)]
    struct Bucket {
        count: u64,
        sum: [u64; 3],
    }

    let mut histogram = vec![Bucket { count: 0, sum: [0; 3] }; 1 << (3 * HISTOGRAM_BITS)];
    for rgb in pixels.chunks_exact(3) {
        let bucket = &mut histogram[bucket([rgb[0], rgb[1], rgb[2]])];
        bucket.count += 1;
        for (sum, &sample) in bucket.sum.iter_mut().zip(rgb) {
            *sum += u64::from(sample);
        }
    }

    // Each occupied bucket is represented by its mean color and population.
    let colors: Vec<([u8; 3], u64)> = histogram
        .iter()
        .filter(|bucket| bucket.count > 0)
        .map(|bucket| {
            let mean = bucket
                .sum
                .map(|sum| u8::try_from(sum / bucket.count).expect("mean of 8-bit samples fits in u8"));
            (mean, bucket.count)
        })
        .collect();

    let mut boxes = vec![colors];
    while boxes.len() < SIXEL_COLORS {
        // Split the box whose widest channel range, weighted by population, is largest.
        let widest = |colors: &[([u8; 3], u64)]| {
            (0..3)
                .map(|channel| {
                    let (min, max) = colors.iter().fold((u8::MAX, u8::MIN), |(min, max), (rgb, _)| {
                        (min.min(rgb[channel]), max.max(rgb[channel]))
                    });
                    (max - min, channel)
                })
                .max()
                .expect("three channels")
        };
        let Some((index, channel)) = boxes
            .iter()
            .enumerate()
            .filter(|(_, colors)| colors.len() > 1)
            .map(|(index, colors)| {
                let (range, channel) = widest(colors);
                let population: u64 = colors.iter().map(|(_, count)| count).sum();
                (u64::from(range) * population, index, channel)
            })
            .max()
            .map(|(_, index, channel)| (index, channel))
        else {
            break;
        };

        let mut colors = boxes.swap_remove(index);
        colors.sort_unstable_by_key(|(rgb, _)| rgb[channel]);
        let half = colors.iter().map(|(_, count)| count).sum::<u64>() / 2;
        let mut seen = 0;
        let split = colors
            .iter()
            .position(|(_, count)| {
                seen += count;
                seen > half
            })
            .unwrap_or(0)
            .clamp(1, colors.len() - 1);
        let upper = colors.split_off(split);
        boxes.push(colors);
        boxes.push(upper);
    }

    boxes
        .iter()
        .map(|colors| {
            let population: u64 = colors.iter().map(|(_, count)| count).sum();
            [0, 1, 2].map(|channel| {
                let sum: u64 = colors.iter().map(|(rgb, count)| u64::from(rgb[channel]) * count).sum();
                u8::try_from(sum / population.max(1)).expect("mean of 8-bit samples fits in u8")
            })
        })
        .collect()
}

/// Maps each pixel to a palette index with Floyd–Steinberg error diffusion.
fn dither(image: &RgbImage, palette: &[[u8; 3]]) -> Vec<u8> {
    let width = usize::try_from(image.width).expect("image width fits in usize");
    let nearest = |rgb: [u8; 3]| {
        let distance = |color: &[u8; 3]| -> i32 {
            (0..3)
                .map(|channel| {
                    let delta = i32::from(rgb[channel]) - i32::from(color[channel]);
                    delta * delta
                })
                .sum()
        };
        let (index, _) = palette
            .iter()
            .enumerate()
            .min_by_key(|(_, color)| distance(color))
            .expect("palette is not empty");
        u8::try_from(index).expect("palette has at most 256 colors")
    };
    // Nearest-color lookups are memoized per histogram bucket; palette colors are bucket means, so
    // the approximation stays within one bucket's width.
    let mut cache = vec![u16::MAX; 1 << (3 * HISTOGRAM_BITS)];

    let mut indices = Vec::with_capacity(image.pixels.len() / 3);
    // Error carried into the current and next rows, in 1/16 units, per channel, with one pixel of
    // padding on each side.
    let mut current = vec![[0i32; 3]; width + 2];
    let mut next = vec![[0i32; 3]; width + 2];
    for row in image.pixels.chunks_exact(width * 3) {
        for (x, rgb) in row.chunks_exact(3).enumerate() {
            let target = [0, 1, 2].map(|channel| {
                let value = i32::from(rgb[channel]) + current[x + 1][channel] / 16;
                u8::try_from(value.clamp(0, 255)).expect("clamped to u8")
            });
            let slot = &mut cache[bucket(target)];
            if *slot == u16::MAX {
                *slot = u16::from(nearest(target));
            }
            let index = u8::try_from(*slot).expect("cached palette index fits in u8");
            indices.push(index);

            let chosen = palette[usize::from(index)];
            for channel in 0..3 {
                let error = i32::from(target[channel]) - i32::from(chosen[channel]);
                current[x + 2][channel] += error * 7;
                next[x][channel] += error * 3;
                next[x + 1][channel] += error * 5;
                next[x + 2][channel] += error;
            }
        }
        core::mem::swap(&mut current, &mut next);
        next.iter_mut().for_each(|error| *error = [0; 3]);
    }
    indices
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: u32, height: u32, rgb: [u8; 3]) -> RgbImage {
        RgbImage {
            width,
            height,
            pixels: rgb.repeat(usize::try_from(width * height).unwrap()),
        }
    }

    #[test]
    fn fit_width_preserves_aspect_ratio_and_color() {
        let image = solid(1920, 1080, [10, 200, 30]).fit_width(640);
        assert_eq!((image.width, image.height), (640, 360));
        assert!(image.pixels.chunks_exact(3).all(|pixel| pixel == [10, 200, 30]));
    }

    #[test]
    fn fit_width_keeps_small_images() {
        let image = solid(100, 50, [0, 0, 0]).fit_width(640);
        assert_eq!((image.width, image.height), (100, 50));
    }

    #[test]
    fn sixel_encodes_solid_band_with_run_length() {
        let sixel = encode_sixel(&solid(10, 6, [255, 0, 0]));
        assert!(sixel.starts_with("\x1bP0;1;0q\"1;1;10;6#0;2;100;0;0#"));
        // One-color palette; all six rows set gives '~'.
        assert!(sixel.contains("#0!10~$-"));
        assert!(sixel.ends_with("\x1b\\"));
    }

    #[test]
    fn palette_reproduces_few_colors_exactly() {
        let colors = [[12, 34, 56], [200, 100, 50], [255, 255, 255]];
        let image = RgbImage {
            width: 3,
            height: 1,
            pixels: colors.concat(),
        };
        let palette = median_cut_palette(&image.pixels);
        assert_eq!(palette.len(), 3);
        let indices = dither(&image, &palette);
        let mapped: Vec<[u8; 3]> = indices.iter().map(|&index| palette[usize::from(index)]).collect();
        assert_eq!(mapped, colors);
    }

    #[test]
    fn palette_is_capped_and_dithering_preserves_average() {
        let width = 256u32;
        let pixels: Vec<u8> = (0..64u32)
            .flat_map(|y| {
                (0..width)
                    .flat_map(move |x| [x, y * 4, (x + y) % 256].map(|value| u8::try_from(value).expect("fits in u8")))
            })
            .collect();
        let image = RgbImage {
            width,
            height: 64,
            pixels,
        };
        let palette = median_cut_palette(&image.pixels);
        assert!(palette.len() <= SIXEL_COLORS);

        let indices = dither(&image, &palette);
        let mean = |values: &mut dyn Iterator<Item = u8>| {
            let values: Vec<u64> = values.map(u64::from).collect();
            values.iter().sum::<u64>() / u64::try_from(values.len()).expect("fits in u64")
        };
        let source = mean(&mut image.pixels.chunks_exact(3).map(|rgb| rgb[0]));
        let rendered = mean(&mut indices.iter().map(|&index| palette[usize::from(index)][0]));
        assert!(source.abs_diff(rendered) <= 2, "source {source}, rendered {rendered}");
    }

    #[test]
    fn sixel_handles_partial_last_band() {
        let sixel = encode_sixel(&solid(2, 7, [0, 0, 0]));
        // The second band only has its top row set.
        assert!(sixel.contains("#0~~$-#0@@$-"));
    }
}

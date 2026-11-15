use bytes::Bytes;
use cef::{CursorInfo, CursorType};
use ironrdp_server::{DisplayUpdate, RGBAPointer};

const BUILTIN_SIZE: usize = 24;
const MAX_CUSTOM_INPUT_DIMENSION: u32 = 512;
const MAX_RDP_POINTER_DIMENSION: u32 = 32;

const CLEAR: [u8; 4] = [0x00, 0x00, 0x00, 0x00];
const BLACK: [u8; 4] = [0x00, 0x00, 0x00, 0xFF];
const WHITE: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFF];
const RED: [u8; 4] = [0x20, 0x20, 0xDC, 0xFF];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum CursorShape {
    #[default]
    Default,
    Hidden,
    Bitmap {
        width: u16,
        height: u16,
        hot_x: u16,
        hot_y: u16,
        /// Bottom-up 32 bpp BGRA pixels per MS-RDPBCGR section 2.2.9.1.1.4.4.
        bottom_up_bgra: Bytes,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltinCursor {
    Hand,
    IBeam,
    Crosshair,
    Wait,
    NotAllowed,
    ResizeEw,
    ResizeNs,
    ResizeNwse,
    ResizeNesw,
    Move,
}

impl CursorShape {
    pub fn from_cef(kind: CursorType, custom: Option<&CursorInfo>) -> Self {
        if kind == CursorType::POINTER {
            return Self::Default;
        }
        if kind == CursorType::NONE {
            return Self::Hidden;
        }
        if kind == CursorType::CUSTOM {
            return custom.and_then(Self::from_cef_custom).unwrap_or(Self::Default);
        }
        if kind == CursorType::IBEAM || kind == CursorType::VERTICALTEXT {
            return Self::builtin(BuiltinCursor::IBeam);
        }
        if kind == CursorType::HAND || kind == CursorType::DND_LINK || kind == CursorType::ALIAS {
            return Self::builtin(BuiltinCursor::Hand);
        }
        if kind == CursorType::CROSS || kind == CursorType::CELL {
            return Self::builtin(BuiltinCursor::Crosshair);
        }
        if kind == CursorType::WAIT || kind == CursorType::PROGRESS {
            return Self::builtin(BuiltinCursor::Wait);
        }
        if kind == CursorType::NOTALLOWED || kind == CursorType::NODROP || kind == CursorType::DND_NONE {
            return Self::builtin(BuiltinCursor::NotAllowed);
        }
        if kind == CursorType::EASTRESIZE
            || kind == CursorType::WESTRESIZE
            || kind == CursorType::EASTWESTRESIZE
            || kind == CursorType::COLUMNRESIZE
        {
            return Self::builtin(BuiltinCursor::ResizeEw);
        }
        if kind == CursorType::NORTHRESIZE
            || kind == CursorType::SOUTHRESIZE
            || kind == CursorType::NORTHSOUTHRESIZE
            || kind == CursorType::ROWRESIZE
        {
            return Self::builtin(BuiltinCursor::ResizeNs);
        }
        if kind == CursorType::NORTHWESTRESIZE
            || kind == CursorType::SOUTHEASTRESIZE
            || kind == CursorType::NORTHWESTSOUTHEASTRESIZE
        {
            return Self::builtin(BuiltinCursor::ResizeNwse);
        }
        if kind == CursorType::NORTHEASTRESIZE
            || kind == CursorType::SOUTHWESTRESIZE
            || kind == CursorType::NORTHEASTSOUTHWESTRESIZE
        {
            return Self::builtin(BuiltinCursor::ResizeNesw);
        }
        if kind == CursorType::MOVE
            || kind == CursorType::GRAB
            || kind == CursorType::DND_MOVE
            || kind == CursorType::DND_COPY
            || kind == CursorType::ZOOMIN
            || kind == CursorType::ZOOMOUT
            || kind == CursorType::HELP
        {
            return Self::builtin(BuiltinCursor::Move);
        }
        Self::Default
    }

    pub fn builtin(cursor: BuiltinCursor) -> Self {
        let mut grid = [[0u8; BUILTIN_SIZE]; BUILTIN_SIZE];
        let (hot_x, hot_y) = draw_mask(cursor, &mut grid);
        let outlined = outline_mask(&grid);
        let mut top_down = Vec::with_capacity(BUILTIN_SIZE * BUILTIN_SIZE * 4);
        let fill = if cursor == BuiltinCursor::NotAllowed {
            RED
        } else {
            BLACK
        };
        for row in &outlined {
            for &cell in row {
                let pixel = match cell {
                    1 => fill,
                    2 => WHITE,
                    _ => CLEAR,
                };
                top_down.extend_from_slice(&pixel);
            }
        }
        Self::from_top_down_bgra(
            u32::try_from(BUILTIN_SIZE).expect("builtin width"),
            u32::try_from(BUILTIN_SIZE).expect("builtin height"),
            i32::from(hot_x),
            i32::from(hot_y),
            &top_down,
        )
        .expect("builtin cursor geometry is valid")
    }

    fn from_cef_custom(info: &CursorInfo) -> Option<Self> {
        let width = u32::try_from(info.size.width).ok()?;
        let height = u32::try_from(info.size.height).ok()?;
        if info.buffer.is_null()
            || width == 0
            || height == 0
            || MAX_CUSTOM_INPUT_DIMENSION < width
            || MAX_CUSTOM_INPUT_DIMENSION < height
        {
            return None;
        }
        let byte_len = usize::try_from(width)
            .ok()?
            .checked_mul(usize::try_from(height).ok()?)?
            .checked_mul(4)?;
        // SAFETY: CEF supplies `size.width * size.height` 32 bpp BGRA pixels for the callback duration.
        let pixels = unsafe { std::slice::from_raw_parts(info.buffer.cast::<u8>(), byte_len) };
        Self::from_top_down_bgra(width, height, info.hotspot.x, info.hotspot.y, pixels)
    }

    pub fn from_top_down_bgra(width: u32, height: u32, hot_x: i32, hot_y: i32, bgra: &[u8]) -> Option<Self> {
        if width == 0 || height == 0 || MAX_CUSTOM_INPUT_DIMENSION < width || MAX_CUSTOM_INPUT_DIMENSION < height {
            return None;
        }
        let in_w = usize::try_from(width).ok()?;
        let in_h = usize::try_from(height).ok()?;
        if bgra.len() != in_w.checked_mul(in_h)?.checked_mul(4)? {
            return None;
        }
        if bgra.chunks_exact(4).all(|pixel| pixel[3] == 0) {
            return Some(Self::Hidden);
        }

        let max_dim = width.max(height);
        let (out_w, out_h, scaled_hot_x, scaled_hot_y) = if max_dim <= MAX_RDP_POINTER_DIMENSION {
            (
                width,
                height,
                hot_x.clamp(0, i32::try_from(width - 1).ok()?),
                hot_y.clamp(0, i32::try_from(height - 1).ok()?),
            )
        } else {
            let out_w = ((u64::from(width) * u64::from(MAX_RDP_POINTER_DIMENSION)).div_ceil(u64::from(max_dim)))
                .clamp(1, u64::from(MAX_RDP_POINTER_DIMENSION)) as u32;
            let out_h = ((u64::from(height) * u64::from(MAX_RDP_POINTER_DIMENSION)).div_ceil(u64::from(max_dim)))
                .clamp(1, u64::from(MAX_RDP_POINTER_DIMENSION)) as u32;
            let clamped_x = u64::try_from(hot_x.max(0)).unwrap_or(0).min(u64::from(width - 1));
            let clamped_y = u64::try_from(hot_y.max(0)).unwrap_or(0).min(u64::from(height - 1));
            let sx = ((clamped_x * u64::from(out_w)) / u64::from(width)).min(u64::from(out_w - 1)) as i32;
            let sy = ((clamped_y * u64::from(out_h)) / u64::from(height)).min(u64::from(out_h - 1)) as i32;
            (out_w, out_h, sx, sy)
        };

        let out_w_usize = usize::try_from(out_w).ok()?;
        let out_h_usize = usize::try_from(out_h).ok()?;
        let mut bottom_up = vec![0u8; out_w_usize * out_h_usize * 4];
        for out_row in 0..out_h_usize {
            let src_row = (out_row * in_h) / out_h_usize;
            let dest_row = out_h_usize - 1 - out_row;
            for out_col in 0..out_w_usize {
                let src_col = (out_col * in_w) / out_w_usize;
                let src_offset = (src_row * in_w + src_col) * 4;
                let dest_offset = (dest_row * out_w_usize + out_col) * 4;
                bottom_up[dest_offset..dest_offset + 4].copy_from_slice(&bgra[src_offset..src_offset + 4]);
            }
        }

        Some(Self::Bitmap {
            width: u16::try_from(out_w).ok()?,
            height: u16::try_from(out_h).ok()?,
            hot_x: u16::try_from(scaled_hot_x).ok()?,
            hot_y: u16::try_from(scaled_hot_y).ok()?,
            bottom_up_bgra: Bytes::from(bottom_up),
        })
    }

    pub fn to_display_update(&self) -> DisplayUpdate {
        match self {
            Self::Default => DisplayUpdate::DefaultPointer,
            Self::Hidden => DisplayUpdate::HidePointer,
            Self::Bitmap {
                width,
                height,
                hot_x,
                hot_y,
                bottom_up_bgra,
            } => DisplayUpdate::RGBAPointer(RGBAPointer {
                cache_index: 0,
                width: *width,
                height: *height,
                hot_x: *hot_x,
                hot_y: *hot_y,
                data: bottom_up_bgra.to_vec(),
            }),
        }
    }
}

fn set_px(grid: &mut [[u8; BUILTIN_SIZE]; BUILTIN_SIZE], x: isize, y: isize) {
    if let (Ok(ux), Ok(uy)) = (usize::try_from(x), usize::try_from(y))
        && ux < BUILTIN_SIZE
        && uy < BUILTIN_SIZE
    {
        grid[uy][ux] = 1;
    }
}

fn draw_line(grid: &mut [[u8; BUILTIN_SIZE]; BUILTIN_SIZE], x0: isize, y0: isize, x1: isize, y1: isize) {
    let steps = (x1 - x0).abs().max((y1 - y0).abs()).max(1);
    for step in 0..=steps {
        let x = x0 + ((x1 - x0) * step) / steps;
        let y = y0 + ((y1 - y0) * step) / steps;
        set_px(grid, x, y);
    }
}

fn draw_mask(cursor: BuiltinCursor, grid: &mut [[u8; BUILTIN_SIZE]; BUILTIN_SIZE]) -> (u8, u8) {
    match cursor {
        BuiltinCursor::IBeam => {
            draw_line(grid, 8, 3, 14, 3);
            draw_line(grid, 8, 19, 14, 19);
            draw_line(grid, 11, 3, 11, 19);
            (11, 11)
        }
        BuiltinCursor::Hand => {
            for y in 2..=13 {
                set_px(grid, 7, y);
                set_px(grid, 8, y);
            }
            for x in 5..=15 {
                for y in 9..=18 {
                    if x >= 7 || y >= 12 {
                        set_px(grid, x, y);
                    }
                }
            }
            draw_line(grid, 4, 12, 6, 15);
            (7, 2)
        }
        BuiltinCursor::Crosshair => {
            draw_line(grid, 11, 3, 11, 19);
            draw_line(grid, 3, 11, 19, 11);
            (11, 11)
        }
        BuiltinCursor::Wait => {
            draw_line(grid, 6, 4, 16, 4);
            draw_line(grid, 6, 18, 16, 18);
            draw_line(grid, 7, 5, 15, 17);
            draw_line(grid, 15, 5, 7, 17);
            (11, 11)
        }
        BuiltinCursor::NotAllowed => {
            for y in 0..BUILTIN_SIZE as isize {
                for x in 0..BUILTIN_SIZE as isize {
                    let dx = x - 11;
                    let dy = y - 11;
                    let dist2 = dx * dx + dy * dy;
                    if (36..=64).contains(&dist2) {
                        set_px(grid, x, y);
                    }
                }
            }
            draw_line(grid, 6, 6, 16, 16);
            draw_line(grid, 7, 6, 16, 15);
            (11, 11)
        }
        BuiltinCursor::ResizeEw => {
            draw_line(grid, 3, 11, 19, 11);
            draw_line(grid, 3, 11, 7, 7);
            draw_line(grid, 3, 11, 7, 15);
            draw_line(grid, 19, 11, 15, 7);
            draw_line(grid, 19, 11, 15, 15);
            (11, 11)
        }
        BuiltinCursor::ResizeNs => {
            draw_line(grid, 11, 3, 11, 19);
            draw_line(grid, 11, 3, 7, 7);
            draw_line(grid, 11, 3, 15, 7);
            draw_line(grid, 11, 19, 7, 15);
            draw_line(grid, 11, 19, 15, 15);
            (11, 11)
        }
        BuiltinCursor::ResizeNwse => {
            draw_line(grid, 4, 4, 18, 18);
            draw_line(grid, 4, 4, 9, 4);
            draw_line(grid, 4, 4, 4, 9);
            draw_line(grid, 18, 18, 13, 18);
            draw_line(grid, 18, 18, 18, 13);
            (11, 11)
        }
        BuiltinCursor::ResizeNesw => {
            draw_line(grid, 18, 4, 4, 18);
            draw_line(grid, 18, 4, 13, 4);
            draw_line(grid, 18, 4, 18, 9);
            draw_line(grid, 4, 18, 9, 18);
            draw_line(grid, 4, 18, 4, 13);
            (11, 11)
        }
        BuiltinCursor::Move => {
            draw_line(grid, 3, 11, 19, 11);
            draw_line(grid, 11, 3, 11, 19);
            draw_line(grid, 3, 11, 6, 8);
            draw_line(grid, 3, 11, 6, 14);
            draw_line(grid, 19, 11, 16, 8);
            draw_line(grid, 19, 11, 16, 14);
            draw_line(grid, 11, 3, 8, 6);
            draw_line(grid, 11, 3, 14, 6);
            draw_line(grid, 11, 19, 8, 16);
            draw_line(grid, 11, 19, 14, 16);
            (11, 11)
        }
    }
}

fn outline_mask(grid: &[[u8; BUILTIN_SIZE]; BUILTIN_SIZE]) -> [[u8; BUILTIN_SIZE]; BUILTIN_SIZE] {
    let mut out = *grid;
    for y in 0..BUILTIN_SIZE {
        for x in 0..BUILTIN_SIZE {
            if grid[y][x] != 0 {
                continue;
            }
            let touches_fg = (-1isize..=1).any(|dy| {
                (-1isize..=1).any(|dx| {
                    let nx = x as isize + dx;
                    let ny = y as isize + dy;
                    usize::try_from(nx)
                        .ok()
                        .zip(usize::try_from(ny).ok())
                        .is_some_and(|(ux, uy)| ux < BUILTIN_SIZE && uy < BUILTIN_SIZE && grid[uy][ux] == 1)
                })
            });
            if touches_fg {
                out[y][x] = 2;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // oracle: spec MS-RDPBCGR §2.2.9.1.1.4.4; 32 bpp pointer XOR masks store bottom-up scan lines.
    #[test]
    fn custom_bgra_cursor_flips_rows_bottom_up_and_clamps_hotspot() {
        let top_row = [0x11, 0x22, 0x33, 0xFF, 0x44, 0x55, 0x66, 0xFF];
        let bottom_row = [0x77, 0x88, 0x99, 0xFF, 0xAA, 0xBB, 0xCC, 0x80];
        let mut top_down = Vec::new();
        top_down.extend_from_slice(&top_row);
        top_down.extend_from_slice(&bottom_row);

        let shape = CursorShape::from_top_down_bgra(2, 2, 9, -3, &top_down).expect("valid cursor");
        let CursorShape::Bitmap {
            width,
            height,
            hot_x,
            hot_y,
            bottom_up_bgra,
        } = shape
        else {
            panic!("expected bitmap cursor");
        };
        assert_eq!((width, height), (2, 2));
        assert_eq!((hot_x, hot_y), (1, 0));
        assert_eq!(&bottom_up_bgra[..8], &bottom_row);
        assert_eq!(&bottom_up_bgra[8..], &top_row);
    }

    // oracle: mutant sending an invisible 32 bpp bitmap instead of HidePointer for a fully transparent custom cursor.
    #[test]
    fn transparent_custom_cursor_becomes_hidden_pointer() {
        let clear = vec![0u8; 4 * 4 * 4];
        assert_eq!(
            CursorShape::from_top_down_bgra(4, 4, 0, 0, &clear),
            Some(CursorShape::Hidden)
        );
    }

    // oracle: spec MS-RDPBCGR §2.2.7.2.7; baseline New Pointer clients accept up to 32x32 pointers.
    #[test]
    fn oversized_custom_cursor_scales_within_baseline_rdp_limit() {
        let opaque = [0x10, 0x20, 0x30, 0xFF].repeat(64 * 32);
        let shape = CursorShape::from_top_down_bgra(64, 32, 32, 16, &opaque).expect("scaled cursor");
        let CursorShape::Bitmap {
            width,
            height,
            hot_x,
            hot_y,
            ..
        } = shape
        else {
            panic!("expected bitmap cursor");
        };
        assert_eq!((width, height), (32, 16));
        assert_eq!((hot_x, hot_y), (16, 8));
    }

    // oracle: mutant mapping distinct standard CEF cursor kinds to identical pointer updates.
    #[test]
    fn standard_cef_cursor_types_map_to_distinct_rdp_shapes() {
        let pointer = CursorShape::from_cef(CursorType::POINTER, None);
        let none = CursorShape::from_cef(CursorType::NONE, None);
        let hand = CursorShape::from_cef(CursorType::HAND, None);
        let ibeam = CursorShape::from_cef(CursorType::IBEAM, None);
        assert_eq!(pointer, CursorShape::Default);
        assert_eq!(none, CursorShape::Hidden);
        assert_ne!(hand, pointer);
        assert_ne!(hand, ibeam);
    }
}

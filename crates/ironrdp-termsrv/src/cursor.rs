use std::collections::VecDeque;
use std::io;

use anyhow::Context as _;
use ironrdp_pdu::pointer::PointerPositionAttribute;
use ironrdp_server::{ColorPointer, DesktopSize, DisplayUpdate, LargePointer, RGBAPointer};
use tokio::io::AsyncWriteExt as _;
use tokio::net::TcpStream;
use tokio::net::tcp::OwnedWriteHalf;
use tracing::warn;
use windows::Win32::Foundation::{ERROR_POINT_NOT_FOUND, GetLastError};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject,
    GetDIBits, GetObjectW, HBITMAP, HDC, HGDIOBJ, RGBQUAD,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GMMP_USE_DISPLAY_POINTS, GetMouseMovePointsEx, MOUSEMOVEPOINT};
use windows::Win32::UI::WindowsAndMessaging::{
    CURSOR_SHOWING, CURSORINFO, GetCursorInfo, GetIconInfo, HICON, ICONINFO,
};

const MAX_DIMENSION: u16 = 384;
const HEADER_LEN: usize = 1 /* Kind */ + 2 /* X */ + 2 /* Y */ + 2 /* Width */ + 2 /* Height */
    + 2 /* HotX */ + 2 /* HotY */ + 8 /* Movement */;
const MAX_PAYLOAD_LEN: usize = HEADER_LEN + 384 * 384 * 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Snapshot {
    Hidden,
    HiddenAt {
        x: u16,
        y: u16,
        movement: Movement,
    },
    Visible {
        x: u16,
        y: u16,
        movement: Movement,
        image: Image,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Movement {
    serial: u64,
    from_client: bool,
}

type NativePoint = (i32, i32, u32, usize);

#[derive(Default)]
struct MotionTracker {
    input_marker: usize,
    position: Option<(u16, u16)>,
    history: Vec<NativePoint>,
    serial: u64,
}

impl MotionTracker {
    fn observe(&mut self, position: (u16, u16), history: Vec<NativePoint>) -> Movement {
        if self.position.is_some() && history.is_empty() {
            self.position = Some(position);
            return Movement {
                serial: self.serial,
                from_client: true,
            };
        }
        if self.position != Some(position) || self.history != history {
            self.serial = self.serial.wrapping_add(1);
            self.position = Some(position);
            self.history = history;
        }
        Movement {
            serial: self.serial,
            from_client: self.history.first().is_some_and(|point| point.3 == self.input_marker),
        }
    }
}

pub(crate) struct Capture {
    failed: bool,
    motion: MotionTracker,
}

impl Capture {
    pub(crate) fn new(input_marker: usize) -> Self {
        Self {
            failed: false,
            motion: MotionTracker {
                input_marker,
                ..Default::default()
            },
        }
    }

    pub(crate) fn capture(&mut self, desktop: DesktopSize) -> Snapshot {
        let mut fallback = Snapshot::Hidden;
        match Snapshot::capture(&mut self.motion, desktop, &mut fallback) {
            Ok(snapshot) => {
                self.failed = false;
                snapshot
            }
            Err(error) => {
                if !self.failed {
                    warn!(error = %format!("{error:#}"), "Native cursor unavailable; hiding cursor until capture recovers");
                }
                self.failed = true;
                fallback
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Image {
    width: u16,
    height: u16,
    hot_x: u16,
    hot_y: u16,
    alpha: bool,
    xor: Vec<u8>,
    and: Vec<u8>,
}

impl Snapshot {
    fn capture(motion: &mut MotionTracker, desktop: DesktopSize, fallback: &mut Self) -> anyhow::Result<Self> {
        let mut cursor = CURSORINFO {
            cbSize: u32::try_from(size_of::<CURSORINFO>())?,
            ..Default::default()
        };
        // SAFETY: The initialized structure remains live for this call.
        unsafe { GetCursorInfo(&mut cursor) }.context("query native cursor")?;
        if !(0..i32::from(desktop.width)).contains(&cursor.ptScreenPos.x)
            || !(0..i32::from(desktop.height)).contains(&cursor.ptScreenPos.y)
        {
            return Ok(Self::Hidden);
        }
        let x = u16::try_from(cursor.ptScreenPos.x)?;
        let y = u16::try_from(cursor.ptScreenPos.y)?;
        let point = MOUSEMOVEPOINT {
            x: i32::from(x),
            y: i32::from(y),
            ..Default::default()
        };
        let mut history = [MOUSEMOVEPOINT::default(); 64];
        // SAFETY: The query and output array are live, and the count matches the array capacity.
        let count = unsafe {
            GetMouseMovePointsEx(
                u32::try_from(size_of::<MOUSEMOVEPOINT>())?,
                &point,
                &mut history,
                GMMP_USE_DISPLAY_POINTS,
            )
        };
        let count = if count == -1 {
            // SAFETY: This reads the failure from the immediately preceding Win32 call.
            let error = unsafe { GetLastError() };
            anyhow::ensure!(error == ERROR_POINT_NOT_FOUND, "query native mouse history: {error:?}");
            0
        } else {
            usize::try_from(count)?
        };
        let movement = motion.observe(
            (x, y),
            history[..count]
                .iter()
                .map(|point| (point.x, point.y, point.time, point.dwExtraInfo))
                .collect(),
        );
        *fallback = Self::HiddenAt { x, y, movement };
        if cursor.flags != CURSOR_SHOWING {
            return Ok(fallback.clone());
        }
        let mut info = ICONINFO::default();
        // SAFETY: GetIconInfo accepts a shared cursor handle and creates owned bitmap copies.
        unsafe { GetIconInfo(HICON(cursor.hCursor.0), &mut info) }.context("copy native cursor bitmaps")?;
        let bitmaps = IconBitmaps(info);
        let mask_size = bitmap_size(bitmaps.0.hbmMask)?;
        let (width, height) = if bitmaps.0.hbmColor.is_invalid() {
            anyhow::ensure!(mask_size.1 % 2 == 0, "invalid monochrome cursor mask height");
            (mask_size.0, mask_size.1 / 2)
        } else {
            bitmap_size(bitmaps.0.hbmColor)?
        };
        let width = u16::try_from(width)?;
        let height = u16::try_from(height)?;
        let hot_x = u16::try_from(bitmaps.0.xHotspot)?;
        let hot_y = u16::try_from(bitmaps.0.yHotspot)?;
        validate_geometry(width, height, hot_x, hot_y)?;
        anyhow::ensure!(
            mask_size
                == (
                    i32::from(width),
                    i32::from(height) * if bitmaps.0.hbmColor.is_invalid() { 2 } else { 1 }
                ),
            "native cursor mask dimensions do not match its image"
        );

        // SAFETY: Creating a memory DC has no pointer preconditions.
        let dc = MemoryDc(unsafe { CreateCompatibleDC(None) });
        anyhow::ensure!(!dc.0.is_invalid(), "failed to create cursor bitmap DC");
        let mask = read_bitmap(dc.0, bitmaps.0.hbmMask, mask_size.0, mask_size.1, 1)?;
        let color = if bitmaps.0.hbmColor.is_invalid() {
            None
        } else {
            Some(read_bitmap(
                dc.0,
                bitmaps.0.hbmColor,
                i32::from(width),
                i32::from(height),
                32,
            )?)
        };
        let image = Image::from_bitmaps(width, height, hot_x, hot_y, &mask, color.as_deref())?;
        Ok(Self::Visible { x, y, movement, image })
    }

    fn encode(&self) -> Vec<u8> {
        if let Self::HiddenAt { x, y, movement } = self {
            let [x_lo, x_hi] = x.to_le_bytes();
            let [y_lo, y_hi] = y.to_le_bytes();
            let mut bytes = vec![3 | if movement.from_client { 0x80 } else { 0 }, x_lo, x_hi, y_lo, y_hi];
            bytes.extend_from_slice(&movement.serial.to_le_bytes());
            return bytes;
        }
        let Self::Visible { x, y, movement, image } = self else {
            return vec![0];
        };
        let mut bytes = Vec::with_capacity(HEADER_LEN + image.xor.len() + image.and.len());
        bytes.push((if image.alpha { 2 } else { 1 }) | if movement.from_client { 0x80 } else { 0 });
        for value in [*x, *y, image.width, image.height, image.hot_x, image.hot_y] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&movement.serial.to_le_bytes());
        bytes.extend_from_slice(&image.xor);
        bytes.extend_from_slice(&image.and);
        bytes
    }

    fn decode(bytes: &[u8]) -> anyhow::Result<Self> {
        if bytes == [0] {
            return Ok(Self::Hidden);
        }
        if bytes.len() == 13 && bytes[0] & 0x7f == 3 {
            return Ok(Self::HiddenAt {
                x: u16::from_le_bytes(bytes[1..3].try_into()?),
                y: u16::from_le_bytes(bytes[3..5].try_into()?),
                movement: Movement {
                    serial: u64::from_le_bytes(bytes[5..13].try_into()?),
                    from_client: bytes[0] & 0x80 != 0,
                },
            });
        }
        anyhow::ensure!(
            HEADER_LEN <= bytes.len() && bytes.len() <= MAX_PAYLOAD_LEN,
            "invalid cursor payload length"
        );
        anyhow::ensure!(matches!(bytes[0] & 0x7f, 1 | 2), "invalid cursor image kind");
        let mut fields = bytes[1..13]
            .chunks_exact(2)
            .map(|field| u16::from_le_bytes([field[0], field[1]]));
        let [x, y, width, height, hot_x, hot_y] =
            core::array::from_fn(|_| fields.next().expect("six validated fields"));
        validate_geometry(width, height, hot_x, hot_y)?;
        let alpha = bytes[0] & 0x7f == 2;
        let xor_len = (usize::from(width) * if alpha { 4 } else { 3 }).next_multiple_of(2) * usize::from(height);
        let and_len = if alpha {
            0
        } else {
            usize::from(width).div_ceil(16) * 2 * usize::from(height)
        };
        anyhow::ensure!(
            bytes.len() == HEADER_LEN + xor_len + and_len,
            "cursor mask lengths do not match dimensions"
        );
        Ok(Self::Visible {
            x,
            y,
            movement: Movement {
                serial: u64::from_le_bytes(bytes[13..HEADER_LEN].try_into()?),
                from_client: bytes[0] & 0x80 != 0,
            },
            image: Image {
                width,
                height,
                hot_x,
                hot_y,
                alpha,
                xor: bytes[HEADER_LEN..HEADER_LEN + xor_len].to_vec(),
                and: bytes[HEADER_LEN + xor_len..].to_vec(),
            },
        })
    }

    pub(crate) async fn write(&self, stream: &mut OwnedWriteHalf) -> anyhow::Result<()> {
        let bytes = self.encode();
        stream
            .write_u32_le(u32::try_from(bytes.len())?)
            .await
            .context("write cursor payload length")?;
        stream.write_all(&bytes).await.context("write cursor snapshot")
    }

    pub(crate) fn updates(&self, previous: Option<&Self>, updates: &mut VecDeque<DisplayUpdate>) {
        match self {
            Self::Hidden | Self::HiddenAt { .. } => {
                if !matches!(previous, Some(Self::Hidden | Self::HiddenAt { .. })) {
                    updates.push_back(DisplayUpdate::HidePointer);
                }
            }
            Self::Visible { image, .. } => {
                if !matches!(previous, Some(Self::Visible { image: old, .. }) if old == image) {
                    updates.push_back(image.update());
                }
            }
        }
        if let Some((x, y, movement)) = self.position()
            && !movement.from_client
            && previous.and_then(Self::position) != Some((x, y, movement))
        {
            updates.push_back(DisplayUpdate::PointerPosition(PointerPositionAttribute { x, y }));
        }
    }

    fn position(&self) -> Option<(u16, u16, Movement)> {
        match self {
            Self::Hidden => None,
            Self::HiddenAt { x, y, movement } | Self::Visible { x, y, movement, .. } => Some((*x, *y, *movement)),
        }
    }
}

impl Image {
    fn from_bitmaps(
        width: u16,
        height: u16,
        hot_x: u16,
        hot_y: u16,
        mask: &[u8],
        color: Option<&[u8]>,
    ) -> anyhow::Result<Self> {
        validate_geometry(width, height, hot_x, hot_y)?;
        let w = usize::from(width);
        let h = usize::from(height);
        let mask_stride = w.div_ceil(32) * 4;
        anyhow::ensure!(
            mask.len() == mask_stride * h * if color.is_some() { 1 } else { 2 },
            "invalid native cursor mask length"
        );
        if let Some(color) = color {
            anyhow::ensure!(color.len() == w * h * 4, "invalid native cursor color length");
        }
        let alpha = color.is_some_and(|data| data.chunks_exact(4).any(|pixel| pixel[3] != 0));
        let xor_stride = (w * if alpha { 4 } else { 3 }).next_multiple_of(2);
        let and_stride = w.div_ceil(16) * 2;
        let mut xor = vec![0; xor_stride * h];
        let mut and = if alpha { Vec::new() } else { vec![0; and_stride * h] };
        for row in 0..h {
            let dest = h - row - 1;
            if !alpha {
                and[dest * and_stride..(dest + 1) * and_stride]
                    .copy_from_slice(&mask[row * mask_stride..row * mask_stride + and_stride]);
            }
            for col in 0..w {
                if let Some(color) = color {
                    let pixel = &color[(row * w + col) * 4..][..4];
                    if alpha {
                        let output = &mut xor[dest * xor_stride + col * 4..][..4];
                        output.copy_from_slice(pixel);
                    } else {
                        xor[dest * xor_stride + col * 3..][..3].copy_from_slice(&pixel[..3]);
                    }
                } else {
                    let bit = (mask[(row + h) * mask_stride + col / 8] >> (7 - col % 8)) & 1;
                    xor[dest * xor_stride + col * 3..][..3].fill(if bit == 0 { 0 } else { 255 });
                }
            }
        }
        Ok(Self {
            width,
            height,
            hot_x,
            hot_y,
            alpha,
            xor,
            and,
        })
    }

    fn update(&self) -> DisplayUpdate {
        if !self.alpha {
            DisplayUpdate::ColorPointer(ColorPointer {
                cache_index: 0,
                width: self.width,
                height: self.height,
                hot_x: self.hot_x,
                hot_y: self.hot_y,
                and_mask: self.and.clone(),
                xor_mask: self.xor.clone(),
            })
        } else if self.width <= 96 && self.height <= 96 {
            DisplayUpdate::RGBAPointer(RGBAPointer {
                cache_index: 0,
                width: self.width,
                height: self.height,
                hot_x: self.hot_x,
                hot_y: self.hot_y,
                data: self.xor.clone(),
            })
        } else {
            DisplayUpdate::LargePointer(LargePointer {
                cache_index: 0,
                width: self.width,
                height: self.height,
                hot_x: self.hot_x,
                hot_y: self.hot_y,
                data: self.xor.clone(),
            })
        }
    }
}

fn validate_geometry(width: u16, height: u16, hot_x: u16, hot_y: u16) -> anyhow::Result<()> {
    anyhow::ensure!(
        (1..=MAX_DIMENSION).contains(&width) && (1..=MAX_DIMENSION).contains(&height),
        "native cursor exceeds the supported dimensions"
    );
    anyhow::ensure!(hot_x < width && hot_y < height, "cursor hotspot is outside its image");
    Ok(())
}

pub(crate) struct Receiver {
    buffer: Vec<u8>,
    filled: usize,
}

impl Default for Receiver {
    fn default() -> Self {
        Self {
            buffer: vec![0; 4],
            filled: 0,
        }
    }
}

impl Receiver {
    pub(crate) fn poll(&mut self, stream: &TcpStream) -> anyhow::Result<Option<Snapshot>> {
        loop {
            match stream.try_read(&mut self.buffer[self.filled..]) {
                Ok(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof)).context("read native cursor"),
                Ok(count) => self.filled += count,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(None),
                Err(error) => return Err(error).context("read native cursor"),
            }
            if self.filled != self.buffer.len() {
                continue;
            }
            if self.buffer.len() == 4 {
                let length = u32::from_le_bytes(self.buffer[..4].try_into()?);
                let length = usize::try_from(length)?;
                anyhow::ensure!((1..=MAX_PAYLOAD_LEN).contains(&length), "invalid cursor frame length");
                self.buffer.resize(4 + length, 0);
            } else {
                let snapshot = Snapshot::decode(&self.buffer[4..])?;
                self.buffer.truncate(4);
                self.filled = 0;
                return Ok(Some(snapshot));
            }
        }
    }
}

struct IconBitmaps(ICONINFO);

impl Drop for IconBitmaps {
    fn drop(&mut self) {
        for bitmap in [self.0.hbmMask, self.0.hbmColor] {
            if !bitmap.is_invalid() {
                // SAFETY: GetIconInfo created these bitmap handles for this owner.
                let _ = unsafe { DeleteObject(HGDIOBJ(bitmap.0)) };
            }
        }
    }
}

struct MemoryDc(HDC);

impl Drop for MemoryDc {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: This guard owns the DC created by CreateCompatibleDC.
            let _ = unsafe { DeleteDC(self.0) };
        }
    }
}

fn bitmap_size(bitmap: HBITMAP) -> anyhow::Result<(i32, i32)> {
    let mut object = BITMAP::default();
    // SAFETY: The output is a live BITMAP and the handle belongs to IconBitmaps.
    let copied = unsafe {
        GetObjectW(
            HGDIOBJ(bitmap.0),
            i32::try_from(size_of::<BITMAP>())?,
            Some(core::ptr::from_mut(&mut object).cast()),
        )
    };
    anyhow::ensure!(
        usize::try_from(copied)? == size_of::<BITMAP>(),
        "failed to query native cursor bitmap"
    );
    anyhow::ensure!(
        0 < object.bmWidth && 0 < object.bmHeight,
        "invalid native cursor bitmap dimensions"
    );
    Ok((object.bmWidth, object.bmHeight))
}

fn read_bitmap(dc: HDC, bitmap: HBITMAP, width: i32, height: i32, bpp: u16) -> anyhow::Result<Vec<u8>> {
    #[repr(C)]
    struct BitmapInfo {
        header: BITMAPINFOHEADER,
        colors: [RGBQUAD; 2],
    }
    let mut info = BitmapInfo {
        header: BITMAPINFOHEADER {
            biSize: u32::try_from(size_of::<BITMAPINFOHEADER>())?,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: bpp,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        colors: [
            RGBQUAD::default(),
            RGBQUAD {
                rgbBlue: 255,
                rgbGreen: 255,
                rgbRed: 255,
                rgbReserved: 0,
            },
        ],
    };
    let stride = (usize::try_from(width)? * usize::from(bpp)).div_ceil(32) * 4;
    let mut bytes = vec![0; stride * usize::try_from(height)?];
    // SAFETY: Dimensions are bounded before allocation, the bitmap is not selected into a DC,
    // and the repr(C) information buffer includes the monochrome palette.
    let rows = unsafe {
        GetDIBits(
            dc,
            bitmap,
            0,
            u32::try_from(height)?,
            Some(bytes.as_mut_ptr().cast()),
            core::ptr::from_mut(&mut info).cast::<BITMAPINFO>(),
            DIB_RGB_COLORS,
        )
    };
    anyhow::ensure!(rows == height, "failed to read native cursor bitmap");
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const INPUT_MARKER: usize = 0x4952_4450;

    // oracle: spec MS-RDPBCGR §2.2.9.1.1.4.4; bottom-up BGR and word-aligned XOR/AND scanlines.
    #[test]
    fn color_masks_preserve_rows_padding_and_hotspot() -> anyhow::Result<()> {
        let pixels = [
            3, 2, 1, 0, 6, 5, 4, 0, 9, 8, 7, 0, 30, 20, 10, 0, 60, 50, 40, 0, 90, 80, 70, 0,
        ];
        let image = Image::from_bitmaps(3, 2, 1, 1, &[0xa0, 0, 0, 0, 0x40, 0, 0, 0], Some(&pixels))?;
        let DisplayUpdate::ColorPointer(pointer) = image.update() else {
            panic!("expected explicit masks")
        };
        assert_eq!(
            pointer.xor_mask,
            [30, 20, 10, 60, 50, 40, 90, 80, 70, 0, 3, 2, 1, 6, 5, 4, 9, 8, 7, 0]
        );
        assert_eq!(pointer.and_mask, [0x40, 0, 0xa0, 0]);
        assert_eq!(
            (pointer.width, pointer.height, pointer.hot_x, pointer.hot_y),
            (3, 2, 1, 1)
        );
        Ok(())
    }

    // oracle: reference Win32 "Drawing Monochrome Pointers"; the four AND/XOR combinations.
    #[test]
    fn monochrome_masks_keep_black_white_transparency_and_inversion() -> anyhow::Result<()> {
        let image = Image::from_bitmaps(4, 1, 0, 0, &[0x30, 0, 0, 0, 0x50, 0, 0, 0], None)?;
        let DisplayUpdate::ColorPointer(pointer) = image.update() else {
            panic!("expected explicit masks")
        };
        assert_eq!(pointer.xor_mask, [0, 0, 0, 255, 255, 255, 0, 0, 0, 255, 255, 255]);
        assert_eq!(pointer.and_mask, [0x30, 0]);
        Ok(())
    }

    // oracle: reference GetDIBits BGRA [60,30,100,128] draws RGB [70,60,100] over [40,90,140] using DrawIconEx.
    #[test]
    fn alpha_cursor_preserves_native_bgra() -> anyhow::Result<()> {
        let image = Image::from_bitmaps(1, 1, 0, 0, &[0; 4], Some(&[60, 30, 100, 128]))?;
        let DisplayUpdate::RGBAPointer(pointer) = image.update() else {
            panic!("expected an alpha pointer")
        };
        assert_eq!(pointer.data, [60, 30, 100, 128]);
        Ok(())
    }

    // oracle: property round-trip.
    #[test]
    fn cursor_snapshot_round_trip() -> anyhow::Result<()> {
        let snapshot = Snapshot::Visible {
            x: 320,
            y: 240,
            movement: Movement::default(),
            image: Image::from_bitmaps(4, 1, 2, 0, &[0x30, 0, 0, 0, 0x50, 0, 0, 0], None)?,
        };
        assert_eq!(Snapshot::decode(&snapshot.encode())?, snapshot);
        assert_eq!(Snapshot::decode(&Snapshot::Hidden.encode())?, Snapshot::Hidden);
        Ok(())
    }

    // oracle: regression hiding the cursor must not discard an application's native position change.
    #[test]
    fn hidden_cursor_snapshot_retains_position() -> anyhow::Result<()> {
        let bytes = [3, 0xc9, 2, 0x52, 1, 1, 0, 0, 0, 0, 0, 0, 0];
        let snapshot = Snapshot::decode(&bytes)?;
        let mut updates = VecDeque::new();
        snapshot.updates(None, &mut updates);
        assert!(matches!(updates.pop_front(), Some(DisplayUpdate::HidePointer)));
        assert!(matches!(
            updates.pop_front(),
            Some(DisplayUpdate::PointerPosition(PointerPositionAttribute {
                x: 713,
                y: 338
            }))
        ));
        assert_eq!(snapshot.encode(), bytes);
        Ok(())
    }

    // oracle: reference GetMouseMovePointsEx preserves input markers, while an immediate SetCursorPos back to A has marker zero.
    #[test]
    fn injected_echo_is_suppressed_but_native_warp_back_to_same_position_is_forwarded() -> anyhow::Result<()> {
        let mut motion = MotionTracker {
            input_marker: INPUT_MARKER,
            ..Default::default()
        };
        let injected = Snapshot::HiddenAt {
            x: 713,
            y: 338,
            movement: motion.observe((713, 338), vec![(713, 338, 100, INPUT_MARKER)]),
        };
        let mut updates = VecDeque::new();
        injected.updates(None, &mut updates);
        assert!(matches!(updates.pop_front(), Some(DisplayUpdate::HidePointer)));
        assert!(
            updates.is_empty(),
            "a delayed injected position must not rewind newer client input"
        );
        assert_eq!(Snapshot::decode(&injected.encode())?, injected);

        let warped = Snapshot::HiddenAt {
            x: 713,
            y: 338,
            movement: motion.observe(
                (713, 338),
                vec![
                    (713, 338, 100, 0),
                    (813, 438, 100, INPUT_MARKER),
                    (713, 338, 100, INPUT_MARKER),
                ],
            ),
        };
        warped.updates(Some(&injected), &mut updates);
        assert!(matches!(
            updates.pop_front(),
            Some(DisplayUpdate::PointerPosition(PointerPositionAttribute {
                x: 713,
                y: 338
            }))
        ));
        warped.updates(Some(&warped), &mut updates);
        assert!(updates.is_empty());
        Ok(())
    }

    // oracle: regression retained native history belongs to its original connection, not a reconnecting client.
    #[test]
    fn previous_connection_marker_does_not_suppress_initial_position() {
        let mut motion = MotionTracker {
            input_marker: INPUT_MARKER + 1,
            ..Default::default()
        };
        let snapshot = Snapshot::HiddenAt {
            x: 713,
            y: 338,
            movement: motion.observe((713, 338), vec![(713, 338, 100, INPUT_MARKER)]),
        };
        let mut updates = VecDeque::new();
        snapshot.updates(None, &mut updates);
        assert!(matches!(updates.pop_front(), Some(DisplayUpdate::HidePointer)));
        assert!(matches!(
            updates.pop_front(),
            Some(DisplayUpdate::PointerPosition(PointerPositionAttribute {
                x: 713,
                y: 338
            }))
        ));
    }

    // oracle: mutant treating ERROR_POINT_NOT_FOUND (empty history) after a client move as a server warp rewinds the client cursor.
    #[test]
    fn missing_mouse_history_after_client_movement_does_not_warp_pointer() {
        let mut motion = MotionTracker {
            input_marker: INPUT_MARKER,
            ..Default::default()
        };
        let first = Snapshot::HiddenAt {
            x: 400,
            y: 300,
            movement: motion.observe((400, 300), vec![(400, 300, 100, INPUT_MARKER)]),
        };
        let second = Snapshot::HiddenAt {
            x: 450,
            y: 320,
            movement: motion.observe((450, 320), Vec::new()),
        };
        let mut updates = VecDeque::new();
        second.updates(Some(&first), &mut updates);
        assert!(updates.is_empty());
    }

    // oracle: mutant duplicate cursor states resend shapes, or a hidden cursor cannot become visible again.
    #[test]
    fn unchanged_cursor_is_quiet_and_visibility_restores_shape() -> anyhow::Result<()> {
        let snapshot = Snapshot::Visible {
            x: 320,
            y: 240,
            movement: Movement::default(),
            image: Image::from_bitmaps(1, 1, 0, 0, &[0; 4], Some(&[0, 0, 255, 255]))?,
        };
        let mut updates = VecDeque::new();
        snapshot.updates(Some(&snapshot), &mut updates);
        assert!(updates.is_empty());
        snapshot.updates(Some(&Snapshot::Hidden), &mut updates);
        assert!(matches!(updates.pop_front(), Some(DisplayUpdate::RGBAPointer(_))));
        assert!(matches!(
            updates.pop_front(),
            Some(DisplayUpdate::PointerPosition(PointerPositionAttribute {
                x: 320,
                y: 240
            }))
        ));
        Snapshot::Hidden.updates(Some(&snapshot), &mut updates);
        assert!(matches!(updates.pop_front(), Some(DisplayUpdate::HidePointer)));
        Ok(())
    }
}

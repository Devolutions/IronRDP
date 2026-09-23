use ironrdp_egfx::avc444::{Yuv420Planes, Yuv444Frame};
use ironrdp_pdu::geometry::ExclusiveRectangle;

const W: usize = 64;
const H: usize = 48;

/// A YUV444 source frame with distinct values in every plane and position.
fn source_frame() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let mut y = vec![0u8; W * H];
    let mut u = vec![0u8; W * H];
    let mut v = vec![0u8; W * H];
    for row in 0..H {
        for col in 0..W {
            let i = row * W + col;
            y[i] = u8::try_from((row * 7 + col * 3) % 256).expect("fits");
            u[i] = u8::try_from((row * 11 + col * 5 + 17) % 256).expect("fits");
            v[i] = u8::try_from((row * 13 + col * 9 + 101) % 256).expect("fits");
        }
    }
    (y, u, v)
}

/// Owned YUV420p planes with strides equal to their widths.
struct Planes420 {
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
}

impl Planes420 {
    fn new() -> Self {
        Self {
            y: vec![0; W * H],
            u: vec![0; W / 2 * H / 2],
            v: vec![0; W / 2 * H / 2],
        }
    }

    fn view(&self) -> Yuv420Planes<'_> {
        Yuv420Planes {
            y: &self.y,
            y_stride: W,
            u: &self.u,
            u_stride: W / 2,
            v: &self.v,
            v_stride: W / 2,
        }
    }
}

/// Split a YUV444 frame into main and auxiliary YUV420p views in YUV444v2
/// layout. A port of FreeRDP's encoder, `general_RGBToAVC444YUVv2_ANY_DOUBLE_ROW`
/// (libfreerdp/primitives/prim_YUV.c), working from YUV planes instead of RGB.
#[expect(clippy::similar_names, reason = "Y, U and V plane names")]
fn split_v2(y444: &[u8], u444: &[u8], v444: &[u8]) -> (Planes420, Planes420) {
    let (mut main, mut aux) = (Planes420::new(), Planes420::new());
    let (half, quarter) = (W / 2, W / 4);
    for row in (0..H).step_by(2) {
        for col in (0..W).step_by(2) {
            let (a, b, c, d) = (
                row * W + col,
                row * W + col + 1,
                (row + 1) * W + col,
                (row + 1) * W + col + 1,
            );

            // B1: luma.
            for i in [a, b, c, d] {
                main.y[i] = y444[i];
            }

            // B2, B3: 2x2 average of U and V.
            let average = |p: &[u8]| {
                u8::try_from((u32::from(p[a]) + u32::from(p[b]) + u32::from(p[c]) + u32::from(p[d])) / 4)
                    .expect("average of u8 fits")
            };
            main.u[row / 2 * half + col / 2] = average(u444);
            main.v[row / 2 * half + col / 2] = average(v444);

            // B4, B5: odd columns, both rows; U in the left half, V in the right.
            aux.y[row * W + col / 2] = u444[b];
            aux.y[row * W + half + col / 2] = v444[b];
            aux.y[(row + 1) * W + col / 2] = u444[d];
            aux.y[(row + 1) * W + half + col / 2] = v444[d];

            // B6-B9: even columns of the odd row; columns 4x go to the
            // auxiliary U plane, 4x+2 to the auxiliary V plane.
            let target = if col % 4 == 0 { &mut aux.u } else { &mut aux.v };
            target[row / 2 * half + col / 4] = u444[c];
            target[row / 2 * half + quarter + col / 4] = v444[c];
        }
    }
    (main, aux)
}

fn full_frame() -> [ExclusiveRectangle; 1] {
    [ExclusiveRectangle {
        left: 0,
        top: 0,
        right: u16::try_from(W).expect("fits"),
        bottom: u16::try_from(H).expect("fits"),
    }]
}

#[test]
fn main_then_auxiliary_view_reconstructs_yuv444() {
    let (y, u, v) = source_frame();
    let (main, aux) = split_v2(&y, &u, &v);

    let mut frame = Yuv444Frame::new(W, H).expect("frame");
    frame.apply_main_view(&main.view(), &full_frame()).expect("main view");
    frame
        .apply_auxiliary_view_v2(&aux.view(), &full_frame())
        .expect("auxiliary view");

    assert_eq!(frame.y(), y.as_slice(), "luma");
    for row in 0..H {
        for col in 0..W {
            let i = row * W + col;
            if row % 2 == 0 && col % 2 == 0 {
                // Only the main view's 2x2 average exists for these positions.
                let block = |p: &[u8]| {
                    (u32::from(p[i]) + u32::from(p[i + 1]) + u32::from(p[i + W]) + u32::from(p[i + W + 1])) / 4
                };
                assert_eq!(u32::from(frame.u()[i]), block(&u), "U average at ({col},{row})");
                assert_eq!(u32::from(frame.v()[i]), block(&v), "V average at ({col},{row})");
            } else {
                assert_eq!(frame.u()[i], u[i], "U at ({col},{row})");
                assert_eq!(frame.v()[i], v[i], "V at ({col},{row})");
            }
        }
    }
}

#[test]
fn main_view_alone_upsamples_chroma() {
    // A luma-only update shows at YUV420 quality: each 2x2 block takes the
    // main view's chroma sample.
    let (y, u, v) = source_frame();
    let (main, _) = split_v2(&y, &u, &v);

    let mut frame = Yuv444Frame::new(W, H).expect("frame");
    frame.apply_main_view(&main.view(), &full_frame()).expect("main view");

    for row in 0..H {
        for col in 0..W {
            let (i, j) = (row * W + col, row / 2 * (W / 2) + col / 2);
            assert_eq!(frame.u()[i], main.u[j]);
            assert_eq!(frame.v()[i], main.v[j]);
        }
    }
}

#[test]
fn updates_stay_inside_their_regions() {
    let (y, u, v) = source_frame();
    let (main, aux) = split_v2(&y, &u, &v);
    let mut frame = Yuv444Frame::new(W, H).expect("frame");
    frame.apply_main_view(&main.view(), &full_frame()).expect("main view");
    frame
        .apply_auxiliary_view_v2(&aux.view(), &full_frame())
        .expect("auxiliary view");
    let before = frame.clone();

    // A luma-only update to the top-left macroblock, from an all-black frame.
    let black = Planes420 {
        y: vec![0; W * H],
        u: vec![128; W / 2 * H / 2],
        v: vec![128; W / 2 * H / 2],
    };
    let region = [ExclusiveRectangle {
        left: 0,
        top: 0,
        right: 16,
        bottom: 16,
    }];
    frame.apply_main_view(&black.view(), &region).expect("main view");

    for row in 0..H {
        for col in 0..W {
            let i = row * W + col;
            if row < 16 && col < 16 {
                assert_eq!((frame.y()[i], frame.u()[i], frame.v()[i]), (0, 128, 128));
            } else {
                assert_eq!(
                    (frame.y()[i], frame.u()[i], frame.v()[i]),
                    (before.y()[i], before.u()[i], before.v()[i]),
                    "outside the region at ({col},{row})"
                );
            }
        }
    }
}

#[test]
fn regions_expand_to_whole_macroblocks() {
    // A region covering part of a macroblock updates the whole macroblock.
    let white = Planes420 {
        y: vec![255; W * H],
        u: vec![128; W / 2 * H / 2],
        v: vec![128; W / 2 * H / 2],
    };
    let mut frame = Yuv444Frame::new(W, H).expect("frame");
    let region = [ExclusiveRectangle {
        left: 20,
        top: 3,
        right: 21,
        bottom: 4,
    }];
    frame.apply_main_view(&white.view(), &region).expect("main view");
    for row in 0..H {
        for col in 0..W {
            let inside = (16..32).contains(&col) && row < 16;
            assert_eq!(frame.y()[row * W + col] == 255, inside, "({col},{row})");
        }
    }
}

#[test]
fn rejects_views_smaller_than_the_frame() {
    let mut frame = Yuv444Frame::new(W, H).expect("frame");
    let short = Planes420 {
        y: vec![0; W * H - 1],
        u: vec![0; W / 2 * H / 2],
        v: vec![0; W / 2 * H / 2],
    };
    assert!(frame.apply_main_view(&short.view(), &full_frame()).is_err());
    assert!(frame.apply_auxiliary_view_v2(&short.view(), &full_frame()).is_err());
}

#[test]
fn rejects_sizes_that_are_not_macroblock_aligned() {
    assert!(Yuv444Frame::new(W + 8, H).is_err());
    assert!(Yuv444Frame::new(W, 0).is_err());
}

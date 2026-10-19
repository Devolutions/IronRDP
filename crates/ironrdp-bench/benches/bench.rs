#![expect(clippy::missing_panics_doc, reason = "panics in benches are allowed")]

use core::num::{NonZeroU16, NonZeroUsize};

use criterion::{Criterion, criterion_group, criterion_main};
use ironrdp_egfx::avc444::{Yuv420Planes, Yuv444Frame};
use ironrdp_graphics::color_conversion::to_64x64_ycbcr_tile;
use ironrdp_pdu::codecs::rfx;
use ironrdp_pdu::geometry::ExclusiveRectangle;
use ironrdp_server::BitmapUpdate;
use ironrdp_server::bench::encoder::rfx::{rfx_enc, rfx_enc_tile};

pub fn rfx_enc_tile_bench(c: &mut Criterion) {
    const WIDTH: NonZeroU16 = NonZeroU16::new(64).expect("value is guaranteed to be non-zero");
    const HEIGHT: NonZeroU16 = NonZeroU16::new(64).expect("value is guaranteed to be non-zero");
    const STRIDE: NonZeroUsize = NonZeroUsize::new(64 * 4).expect("value is guaranteed to be non-zero");

    let quant = rfx::Quant::default();
    let algo = rfx::EntropyAlgorithm::Rlgr3;

    let bitmap = BitmapUpdate {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
        format: ironrdp_server::PixelFormat::ARgb32,
        data: vec![0; 64 * 64 * 4].into(),
        stride: STRIDE,
    };
    c.bench_function("rfx_enc_tile", |b| b.iter(|| rfx_enc_tile(&bitmap, &quant, algo, 0, 0)));
}

pub fn rfx_enc_bench(c: &mut Criterion) {
    const WIDTH: NonZeroU16 = NonZeroU16::new(2048).expect("value is guaranteed to be non-zero");
    const HEIGHT: NonZeroU16 = NonZeroU16::new(2048).expect("value is guaranteed to be non-zero");
    // FIXME/QUESTION: It looks like we have a bug here, don't we? The stride value should be 2048 * 4.
    const STRIDE: NonZeroUsize = NonZeroUsize::new(64 * 4).expect("value is guaranteed to be non-zero");

    let quant = rfx::Quant::default();
    let algo = rfx::EntropyAlgorithm::Rlgr3;

    let bitmap = BitmapUpdate {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
        format: ironrdp_server::PixelFormat::ARgb32,
        data: vec![0; 2048 * 2048 * 4].into(),
        stride: STRIDE,
    };
    c.bench_function("rfx_enc", |b| b.iter(|| rfx_enc(&bitmap, &quant, algo)));
}

pub fn to_ycbcr_bench(c: &mut Criterion) {
    const WIDTH: usize = 64;
    const HEIGHT: usize = 64;

    let input = vec![0; WIDTH * HEIGHT * 4];
    let stride = WIDTH * 4;
    let mut y = [0i16; WIDTH * HEIGHT];
    let mut cb = [0i16; WIDTH * HEIGHT];
    let mut cr = [0i16; WIDTH * HEIGHT];
    let format = ironrdp_graphics::image_processing::PixelFormat::ARgb32;

    c.bench_function("to_ycbcr", |b| {
        b.iter(|| {
            to_64x64_ycbcr_tile(
                &input,
                WIDTH.try_into().expect("can't panic"),
                HEIGHT.try_into().expect("can't panic"),
                stride.try_into().expect("can't panic"),
                format,
                &mut y,
                &mut cb,
                &mut cr,
            )
        })
    });
}

pub fn avc444_bench(c: &mut Criterion) {
    // A 1080p picture, coded as 1920x1088, with one region covering it.
    const WIDTH: usize = 1920;
    const HEIGHT: usize = 1080;
    const CODED_HEIGHT: usize = 1088;

    let y = vec![0u8; WIDTH * CODED_HEIGHT];
    let uv = vec![0u8; WIDTH / 2 * CODED_HEIGHT / 2];
    let view = Yuv420Planes {
        width: u32::try_from(WIDTH).expect("can't panic"),
        y: &y,
        y_stride: WIDTH,
        u: &uv,
        u_stride: WIDTH / 2,
        v: &uv,
        v_stride: WIDTH / 2,
    };
    let region = [ExclusiveRectangle {
        left: 0,
        top: 0,
        right: u16::try_from(WIDTH).expect("can't panic"),
        bottom: u16::try_from(HEIGHT).expect("can't panic"),
    }];
    let mut frame = Yuv444Frame::new(WIDTH, HEIGHT).expect("frame");

    c.bench_function("avc444_main_view", |b| {
        b.iter(|| frame.apply_main_view(&view, &region).expect("main view"))
    });
    c.bench_function("avc444_auxiliary_view_v2", |b| {
        b.iter(|| frame.apply_auxiliary_view_v2(&view, &region).expect("auxiliary view"))
    });
}

criterion_group!(benches, rfx_enc_tile_bench, rfx_enc_bench, to_ycbcr_bench, avc444_bench);
criterion_main!(benches);

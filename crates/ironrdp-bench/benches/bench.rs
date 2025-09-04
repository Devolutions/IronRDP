use core::num::{NonZeroU16, NonZeroUsize};

use criterion::{criterion_group, criterion_main, Criterion};
use ironrdp_graphics::color_conversion::to_64x64_ycbcr_tile;
use ironrdp_pdu::codecs::rfx;
use ironrdp_server::bench::encoder::rfx::{rfx_enc, rfx_enc_tile};
use ironrdp_server::BitmapUpdate;

pub fn rfx_enc_tile_bench(c: &mut Criterion) {
    let quant = rfx::Quant::default();
    let algo = rfx::EntropyAlgorithm::Rlgr3;

    const WIDTH: NonZeroU16 = NonZeroU16::new(64).expect("value is guaranteed to be non-zero");
    const HEIGHT: NonZeroU16 = NonZeroU16::new(64).expect("value is guaranteed to be non-zero");
    const STRIDE: NonZeroUsize = NonZeroUsize::new(64 * 4).expect("value is guaranteed to be non-zero");

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
    let quant = rfx::Quant::default();
    let algo = rfx::EntropyAlgorithm::Rlgr3;

    const WIDTH: NonZeroU16 = NonZeroU16::new(2048).expect("value is guaranteed to be non-zero");
    const HEIGHT: NonZeroU16 = NonZeroU16::new(2048).expect("value is guaranteed to be non-zero");
    // QUESTION: It looks like we have a bug here, don't we? The stride value should be 2048 * 4.
    const STRIDE: NonZeroUsize = NonZeroUsize::new(64 * 4).expect("value is guaranteed to be non-zero");

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
        b.iter(|| to_64x64_ycbcr_tile(&input, WIDTH, HEIGHT, stride, format, &mut y, &mut cb, &mut cr))
    });
}

criterion_group!(benches, rfx_enc_tile_bench, rfx_enc_bench, to_ycbcr_bench);
criterion_main!(benches);

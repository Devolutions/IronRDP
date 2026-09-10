#![expect(clippy::missing_panics_doc, reason = "panics in benches are allowed")]
#![allow(unused_crate_dependencies)] // The package also contains the perfenc binary.

use core::hint::black_box;
use core::num::{NonZeroU16, NonZeroUsize};

use criterion::{Criterion, criterion_group, criterion_main};
use ironrdp_graphics::color_conversion::to_64x64_ycbcr_tile;
use ironrdp_pdu::codecs::rfx;
use ironrdp_server::BitmapUpdate;
use ironrdp_server::bench::encoder::rfx::{rfx_enc, rfx_enc_tile};

fn representative_argb(width: usize, height: usize) -> Vec<u8> {
    let mut pixels = Vec::with_capacity(width * height * 4);

    for y in 0..height {
        for x in 0..width {
            let checker = ((x / 32) + (y / 32)) % 2;
            let red = u8::try_from((x * 17 + y * 11) % 256).expect("color component fits in u8");
            let green = u8::try_from((x * 7 + y * 29) % 256).expect("color component fits in u8");
            let blue = if checker == 0 { 48 } else { 208 };
            pixels.extend_from_slice(&[blue, green, red, u8::MAX]);
        }
    }

    pixels
}

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
        data: representative_argb(64, 64).into(),
        stride: STRIDE,
    };
    c.bench_function("rfx_enc_tile", |b| {
        b.iter(|| black_box(rfx_enc_tile(black_box(&bitmap), black_box(&quant), algo, 0, 0)))
    });
}

pub fn rfx_enc_bench(c: &mut Criterion) {
    const WIDTH: NonZeroU16 = NonZeroU16::new(2048).expect("value is guaranteed to be non-zero");
    const HEIGHT: NonZeroU16 = NonZeroU16::new(2048).expect("value is guaranteed to be non-zero");
    const STRIDE: NonZeroUsize = NonZeroUsize::new(2048 * 4).expect("value is guaranteed to be non-zero");

    let quant = rfx::Quant::default();
    let algo = rfx::EntropyAlgorithm::Rlgr3;

    let bitmap = BitmapUpdate {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
        format: ironrdp_server::PixelFormat::ARgb32,
        data: representative_argb(2048, 2048).into(),
        stride: STRIDE,
    };
    c.bench_function("rfx_enc", |b| {
        b.iter(|| black_box(rfx_enc(black_box(&bitmap), black_box(&quant), algo)))
    });
}

pub fn to_ycbcr_bench(c: &mut Criterion) {
    const WIDTH: usize = 64;
    const HEIGHT: usize = 64;

    let input = representative_argb(WIDTH, HEIGHT);
    let stride = WIDTH * 4;
    let mut y = [0i16; WIDTH * HEIGHT];
    let mut cb = [0i16; WIDTH * HEIGHT];
    let mut cr = [0i16; WIDTH * HEIGHT];
    let format = ironrdp_graphics::image_processing::PixelFormat::ARgb32;

    c.bench_function("to_ycbcr", |b| {
        b.iter(|| {
            to_64x64_ycbcr_tile(
                black_box(&input),
                WIDTH.try_into().expect("can't panic"),
                HEIGHT.try_into().expect("can't panic"),
                stride.try_into().expect("can't panic"),
                format,
                &mut y,
                &mut cb,
                &mut cr,
            )
            .expect("representative ARGB tile is valid");
            black_box((y[0], cb[0], cr[0]))
        });
    });
}

criterion_group!(benches, rfx_enc_tile_bench, rfx_enc_bench, to_ycbcr_bench);
criterion_main!(benches);

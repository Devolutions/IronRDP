use std::num::NonZero;

use criterion::{criterion_group, criterion_main, Criterion};
use ironrdp_pdu::codecs::rfx;
use ironrdp_server::{
    bench::encoder::rfx::{rfx_enc, rfx_enc_tile},
    BitmapUpdate,
};

pub fn rfx_enc_tile_bench(c: &mut Criterion) {
    let quant = rfx::Quant::default();
    let algo = rfx::EntropyAlgorithm::Rlgr3;
    let bitmap = BitmapUpdate {
        top: 0,
        left: 0,
        width: NonZero::new(64).unwrap(),
        height: NonZero::new(64).unwrap(),
        format: ironrdp_server::PixelFormat::ARgb32,
        data: vec![0; 64 * 64 * 4],
        order: ironrdp_server::PixelOrder::BottomToTop,
        stride: 64 * 4,
    };
    c.bench_function("rfx_enc_tile", |b| b.iter(|| rfx_enc_tile(&bitmap, &quant, algo, 0, 0)));
}

pub fn rfx_enc_bench(c: &mut Criterion) {
    let quant = rfx::Quant::default();
    let algo = rfx::EntropyAlgorithm::Rlgr3;
    let bitmap = BitmapUpdate {
        top: 0,
        left: 0,
        width: NonZero::new(2048).unwrap(),
        height: NonZero::new(2048).unwrap(),
        format: ironrdp_server::PixelFormat::ARgb32,
        data: vec![0; 2048 * 2048 * 4],
        order: ironrdp_server::PixelOrder::BottomToTop,
        stride: 64 * 4,
    };
    c.bench_function("rfx_enc", |b| b.iter(|| rfx_enc(&bitmap, &quant, algo)));
}

criterion_group!(benches, rfx_enc_tile_bench, rfx_enc_bench);
criterion_main!(benches);

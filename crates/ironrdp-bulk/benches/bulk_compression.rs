//! Benchmarks for ironrdp-bulk compression algorithms.
//!
//! Measures compress + decompress throughput for MPPC (RDP4, RDP5),
//! NCRUSH (RDP6), and XCRUSH (RDP6.1) with realistic input patterns.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

use ironrdp_bulk::{flags, BulkCompressor, CompressionType};

/// Text-like data (highly compressible, typical of protocol messages).
fn generate_text_data(size: usize) -> Vec<u8> {
    let phrases = [
        b"Session started for user Administrator on server DESKTOP-ABC1234 ".as_slice(),
        b"Desktop width=1920 height=1080 bpp=32 keyboard=en-us locale=0409 ",
        b"Channel joined: rdpdr cliprdr rdpsnd drdynvc MS_T120 ",
        b"Bitmap update received for region (0,0)-(1920,1080) compressed=true ",
    ];
    let mut data = Vec::with_capacity(size);
    let mut idx = 0;
    while data.len() < size {
        let remaining = size - data.len();
        let phrase = phrases[idx % phrases.len()];
        let chunk = &phrase[..remaining.min(phrase.len())];
        data.extend_from_slice(chunk);
        idx += 1;
    }
    data
}

/// Structured bitmap-like data (moderately compressible - runs of similar values).
fn generate_structured_bitmap(size: usize) -> Vec<u8> {
    let mut data = Vec::with_capacity(size);
    // Simulate a desktop with horizontal runs of similar color
    // Each "scanline" of 256 bytes has 4 color runs of 64 bytes each
    let colors: [[u8; 4]; 4] = [
        [30, 60, 120, 255],   // dark blue (taskbar-like)
        [240, 240, 240, 255], // light gray (window background)
        [0, 0, 0, 255],       // black (text region)
        [240, 240, 240, 255], // light gray again
    ];
    let mut scanline = 0u32;
    for i in 0..size {
        let pos_in_scanline = i % 256;
        let color_idx = pos_in_scanline / 64;
        let channel = i % 4;
        // Add slight variation every 4 scanlines to simulate content
        let variation = if scanline % 4 == 0 {
            u8::try_from(pos_in_scanline & 0x03).unwrap_or(0)
        } else {
            0
        };
        data.push(colors[color_idx][channel].wrapping_add(variation));
        if pos_in_scanline == 255 {
            scanline += 1;
        }
    }
    data
}

fn algo_name(ct: CompressionType) -> &'static str {
    match ct {
        CompressionType::Rdp4 => "mppc_rdp4",
        CompressionType::Rdp5 => "mppc_rdp5",
        CompressionType::Rdp6 => "ncrush",
        CompressionType::Rdp61 => "xcrush",
    }
}

fn bench_compress_decompress(c: &mut Criterion, ct: CompressionType, data: &[u8], label: &str) {
    let name = algo_name(ct);

    // Verify data actually compresses with this algorithm
    let mut test_comp = BulkCompressor::new(ct).expect("bulk compressor should initialize");
    let (test_size, test_flags) = test_comp.compress(data).expect("bulk compression should succeed");
    let is_compressed = test_flags & flags::PACKET_COMPRESSED != 0;

    if is_compressed {
        let compressed = test_comp.compressed_data(test_size).to_vec();

        // Benchmark compress
        {
            let mut group = c.benchmark_group(format!("{name}/{label}"));
            group.throughput(Throughput::Bytes(u64::try_from(data.len()).unwrap_or(u64::MAX)));

            group.bench_function(BenchmarkId::new("compress", data.len()), |b| {
                b.iter_batched(
                    || BulkCompressor::new(ct).expect("bulk compressor should initialize"),
                    |mut compressor| {
                        black_box(
                            compressor
                                .compress(black_box(data))
                                .expect("bulk compression should succeed"),
                        );
                    },
                    criterion::BatchSize::SmallInput,
                );
            });

            group.finish();
        }

        // Benchmark decompress
        {
            let mut group = c.benchmark_group(format!("{name}/{label}"));
            group.throughput(Throughput::Bytes(u64::try_from(data.len()).unwrap_or(u64::MAX)));

            group.bench_function(BenchmarkId::new("decompress", data.len()), |b| {
                b.iter_batched(
                    || BulkCompressor::new(ct).expect("bulk compressor should initialize"),
                    |mut decompressor| {
                        black_box(
                            decompressor
                                .decompress(black_box(&compressed), black_box(test_flags))
                                .expect("bulk decompression should succeed"),
                        );
                    },
                    criterion::BatchSize::SmallInput,
                );
            });

            group.finish();
        }
    }
}

fn bench_all(c: &mut Criterion) {
    let text_4k = generate_text_data(4096);
    let text_16k = generate_text_data(16384);
    let bitmap_4k = generate_structured_bitmap(4096);
    let bitmap_16k = generate_structured_bitmap(16384);

    for ct in [
        CompressionType::Rdp4,
        CompressionType::Rdp5,
        CompressionType::Rdp6,
        CompressionType::Rdp61,
    ] {
        bench_compress_decompress(c, ct, &text_4k, "text_4k");
        bench_compress_decompress(c, ct, &text_16k, "text_16k");
        bench_compress_decompress(c, ct, &bitmap_4k, "bitmap_4k");
        bench_compress_decompress(c, ct, &bitmap_16k, "bitmap_16k");
    }
}

criterion_group!(benches, bench_all);
criterion_main!(benches);

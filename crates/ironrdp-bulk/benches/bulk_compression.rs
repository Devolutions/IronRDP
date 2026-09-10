//! Benchmarks for supported ironrdp-bulk compression workloads.

use core::hint::black_box;

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use ironrdp_bulk::{BulkCompressor, CompressionType, flags};

const SUPPORTED_SIZE: usize = 4096;
const PASSTHROUGH_SIZE: usize = 16384;
const HISTORY_PACKETS: usize = 4;

struct CompressedPacket {
    bytes: Vec<u8>,
    flags: u32,
}

/// Text-like data, representative of protocol metadata and small messages.
fn text_data(size: usize) -> Vec<u8> {
    let phrases = [
        b"Session started for user Administrator on server DESKTOP-ABC1234 ".as_slice(),
        b"Desktop width=1920 height=1080 bpp=32 keyboard=en-us locale=0409 ",
        b"Channel joined: rdpdr cliprdr rdpsnd drdynvc MS_T120 ",
        b"Bitmap update received for region (0,0)-(1920,1080) compressed=true ",
    ];
    let mut data = Vec::with_capacity(size);
    let mut index = 0;

    while data.len() < size {
        let phrase = phrases[index % phrases.len()];
        data.extend_from_slice(&phrase[..(size - data.len()).min(phrase.len())]);
        index += 1;
    }

    data
}

/// Structured bitmap-like data with color runs and deterministic variation.
fn bitmap_data(size: usize) -> Vec<u8> {
    let colors = [
        [30, 60, 120, u8::MAX],
        [240, 240, 240, u8::MAX],
        [0, 0, 0, u8::MAX],
        [240, 240, 240, u8::MAX],
    ];
    let mut data = Vec::with_capacity(size);

    for index in 0..size {
        let scanline_offset = index % 256;
        let color = colors[scanline_offset / 64];
        let variation = if (index / 256).is_multiple_of(4) {
            u8::try_from(scanline_offset & 0x03).expect("variation fits in u8")
        } else {
            0
        };
        data.push(color[index % 4].wrapping_add(variation));
    }

    data
}

const fn algorithm_name(compression_type: CompressionType) -> &'static str {
    match compression_type {
        CompressionType::Rdp4 => "mppc-rdp4",
        CompressionType::Rdp5 => "mppc-rdp5",
        CompressionType::Rdp6 => "ncrush",
        CompressionType::Rdp61 => "xcrush",
    }
}

fn compress_packet(compressor: &mut BulkCompressor, data: &[u8]) -> CompressedPacket {
    let (size, packet_flags) = compressor.compress(data).expect("bulk compression should succeed");
    assert_ne!(
        packet_flags & flags::PACKET_COMPRESSED,
        0,
        "supported compression workload must compress"
    );
    CompressedPacket {
        bytes: compressor.compressed_data(size).to_vec(),
        flags: packet_flags,
    }
}

fn prepare_history(compression_type: CompressionType, data: &[u8]) -> Vec<CompressedPacket> {
    let mut compressor = BulkCompressor::new(compression_type);
    let mut packets = Vec::with_capacity(HISTORY_PACKETS);

    for _ in 0..HISTORY_PACKETS {
        packets.push(compress_packet(&mut compressor, data));
    }

    packets
}

fn verify_round_trip(compression_type: CompressionType, packets: &[CompressedPacket], expected: &[u8]) {
    let mut decompressor = BulkCompressor::new(compression_type);

    for packet in packets {
        let decoded = decompressor
            .decompress(&packet.bytes, packet.flags)
            .expect("prepared bulk packet should decompress");
        assert_eq!(decoded, expected, "prepared bulk packet must round-trip");
    }
}

fn bench_supported_workload(c: &mut Criterion, compression_type: CompressionType, label: &str, data: &[u8]) {
    let algorithm = algorithm_name(compression_type);
    let cold_packet = prepare_history(compression_type, data)
        .into_iter()
        .next()
        .expect("one cold packet is prepared");
    verify_round_trip(compression_type, core::slice::from_ref(&cold_packet), data);

    let history_packets = prepare_history(compression_type, data);
    verify_round_trip(compression_type, &history_packets, data);

    let mut group = c.benchmark_group(format!("{algorithm}/{label}"));
    group.throughput(Throughput::Bytes(
        u64::try_from(data.len()).expect("input size fits in u64"),
    ));

    group.bench_function(BenchmarkId::new("compress/cold", data.len()), |b| {
        b.iter(|| {
            let mut compressor = BulkCompressor::new(compression_type);
            black_box(
                compressor
                    .compress(black_box(data))
                    .expect("bulk compression should succeed"),
            )
        });
    });
    group.bench_function(BenchmarkId::new("decompress/cold", data.len()), |b| {
        b.iter(|| {
            let mut decompressor = BulkCompressor::new(compression_type);
            black_box(
                decompressor
                    .decompress(black_box(&cold_packet.bytes), cold_packet.flags)
                    .expect("prepared bulk packet should decompress")
                    .len(),
            )
        });
    });
    group.bench_function(BenchmarkId::new("compress/history", data.len()), |b| {
        b.iter_batched_ref(
            || BulkCompressor::new(compression_type),
            |compressor| {
                for _ in 0..HISTORY_PACKETS {
                    black_box(
                        compressor
                            .compress(black_box(data))
                            .expect("bulk compression should succeed"),
                    );
                }
            },
            BatchSize::SmallInput,
        );
    });
    group.bench_function(BenchmarkId::new("decompress/history", data.len()), |b| {
        b.iter_batched_ref(
            || BulkCompressor::new(compression_type),
            |decompressor| {
                for packet in &history_packets {
                    black_box(
                        decompressor
                            .decompress(black_box(&packet.bytes), packet.flags)
                            .expect("prepared bulk packet should decompress")
                            .len(),
                    );
                }
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

fn bench_passthrough(c: &mut Criterion, compression_type: CompressionType, label: &str, data: &[u8]) {
    assert!(BulkCompressor::should_skip_compression(data.len()));
    let algorithm = algorithm_name(compression_type);
    let mut group = c.benchmark_group(format!("{algorithm}/{label}"));
    group.throughput(Throughput::Bytes(
        u64::try_from(data.len()).expect("input size fits in u64"),
    ));

    group.bench_function(BenchmarkId::new("compress/passthrough", data.len()), |b| {
        b.iter(|| {
            let mut compressor = BulkCompressor::new(compression_type);
            let (size, packet_flags) = compressor
                .compress(black_box(data))
                .expect("bulk compression should succeed");
            assert_eq!(size, data.len(), "passthrough must preserve the input length");
            assert_eq!(
                packet_flags & flags::PACKET_COMPRESSED,
                0,
                "passthrough must not report compression"
            );
            black_box((size, packet_flags))
        });
    });

    group.finish();
}

fn bench_all(c: &mut Criterion) {
    let supported_text = text_data(SUPPORTED_SIZE);
    let supported_bitmap = bitmap_data(SUPPORTED_SIZE);
    let passthrough_text = text_data(PASSTHROUGH_SIZE);
    let passthrough_bitmap = bitmap_data(PASSTHROUGH_SIZE);

    for compression_type in [
        CompressionType::Rdp4,
        CompressionType::Rdp5,
        CompressionType::Rdp6,
        CompressionType::Rdp61,
    ] {
        bench_supported_workload(c, compression_type, "text", &supported_text);
        bench_supported_workload(c, compression_type, "bitmap", &supported_bitmap);
        bench_passthrough(c, compression_type, "text", &passthrough_text);
        bench_passthrough(c, compression_type, "bitmap", &passthrough_bitmap);
    }
}

criterion_group!(benches, bench_all);
criterion_main!(benches);

use rstest::rstest;

/// 64x64 tile samples were generated using rdp-rs crate
#[rstest]
#[case::x27019fd9f222cebce9dfebcddb12bfa0(
    include_bytes!("../../../test_data/rle/tile-27019fd9f222cebce9dfebcddb12bfa0-compressed.bin"),
    include_bytes!("../../../test_data/rle/tile-27019fd9f222cebce9dfebcddb12bfa0-decompressed.bin"),
)]
#[case::x284f668a9366a95e45f15b6bf634a633(
    include_bytes!("../../../test_data/rle/tile-284f668a9366a95e45f15b6bf634a633-compressed.bin"),
    include_bytes!("../../../test_data/rle/tile-284f668a9366a95e45f15b6bf634a633-decompressed.bin"),
)]
#[case::x28c08e75c82ab598c5ab85d1bfc00253(
    include_bytes!("../../../test_data/rle/tile-28c08e75c82ab598c5ab85d1bfc00253-compressed.bin"),
    include_bytes!("../../../test_data/rle/tile-28c08e75c82ab598c5ab85d1bfc00253-decompressed.bin"),
)]
#[case::x2de3f3262a5eeecc3152552c178b782a(
    include_bytes!("../../../test_data/rle/tile-2de3f3262a5eeecc3152552c178b782a-compressed.bin"),
    include_bytes!("../../../test_data/rle/tile-2de3f3262a5eeecc3152552c178b782a-decompressed.bin"),
)]
#[case::x3fc8124af9be2fe88b445db60c36eddc(
    include_bytes!("../../../test_data/rle/tile-3fc8124af9be2fe88b445db60c36eddc-compressed.bin"),
    include_bytes!("../../../test_data/rle/tile-3fc8124af9be2fe88b445db60c36eddc-decompressed.bin"),
)]
#[case::x4d75aa6a18c435c6230ba739b802a861(
    include_bytes!("../../../test_data/rle/tile-4d75aa6a18c435c6230ba739b802a861-compressed.bin"),
    include_bytes!("../../../test_data/rle/tile-4d75aa6a18c435c6230ba739b802a861-decompressed.bin"),
)]
#[case::x8b8ccc77526730d0cd8989901cc031ec(
    include_bytes!("../../../test_data/rle/tile-8b8ccc77526730d0cd8989901cc031ec-compressed.bin"),
    include_bytes!("../../../test_data/rle/tile-8b8ccc77526730d0cd8989901cc031ec-decompressed.bin"),
)]
#[case::x94bb5b131eb3bc110905dfcb0f60da79(
    include_bytes!("../../../test_data/rle/tile-94bb5b131eb3bc110905dfcb0f60da79-compressed.bin"),
    include_bytes!("../../../test_data/rle/tile-94bb5b131eb3bc110905dfcb0f60da79-decompressed.bin"),
)]
#[case::x9b06660a1da806d2d48ce3f46b45d571(
    include_bytes!("../../../test_data/rle/tile-9b06660a1da806d2d48ce3f46b45d571-compressed.bin"),
    include_bytes!("../../../test_data/rle/tile-9b06660a1da806d2d48ce3f46b45d571-decompressed.bin"),
)]
#[case::xa412fbe2b435ac627ce39048aa3d3fb3(
    include_bytes!("../../../test_data/rle/tile-a412fbe2b435ac627ce39048aa3d3fb3-compressed.bin"),
    include_bytes!("../../../test_data/rle/tile-a412fbe2b435ac627ce39048aa3d3fb3-decompressed.bin"),
)]
#[case::xaa326e7a536cc8a0420c44bdf4ef8d97(
    include_bytes!("../../../test_data/rle/tile-aa326e7a536cc8a0420c44bdf4ef8d97-compressed.bin"),
    include_bytes!("../../../test_data/rle/tile-aa326e7a536cc8a0420c44bdf4ef8d97-decompressed.bin"),
)]
#[case::xfbcefc9af4db651aefd91bcabc8ea9fc(
    include_bytes!("../../../test_data/rle/tile-fbcefc9af4db651aefd91bcabc8ea9fc-compressed.bin"),
    include_bytes!("../../../test_data/rle/tile-fbcefc9af4db651aefd91bcabc8ea9fc-decompressed.bin"),
)]
fn decompress_bpp_16(#[case] src: &[u8], #[case] expected: &[u8]) {
    let mut out = Vec::new();
    ironrdp_graphics::rle::decompress_16_bpp(src, &mut out, 64, 64).expect("decompress 16 bpp");
    assert_eq!(out, expected);
}

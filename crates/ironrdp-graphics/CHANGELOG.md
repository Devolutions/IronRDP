# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.3.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-graphics-v0.3.0...ironrdp-graphics-v0.3.1)] - 2025-05-27

### <!-- 1 -->Features

- Add helper to find diff between images ([20581bb6f1](https://github.com/Devolutions/IronRDP/commit/20581bb6f12561e22031ce0e233daeada836ea67)) 

  Add some helper to find "damaged" regions, as 64x64 tiles.

### <!-- 7 -->Build

- Yuvutils renamed to yuv (#774) ([d8ab533463](https://github.com/Devolutions/IronRDP/commit/d8ab533463b345b293a89b91b79d4ad974fe007d)) 

- Bump bitflags from 2.9.0 to 2.9.1 in the patch group across 1 directory (#792) ([87ed315bc2](https://github.com/Devolutions/IronRDP/commit/87ed315bc28fdd2dcfea89b052fa620a7e346e5a)) 



## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-graphics-v0.2.0...ironrdp-graphics-v0.3.0)] - 2025-03-12

### <!-- 7 -->Build

- Bump ironrdp-pdu



## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-graphics-v0.1.2...ironrdp-graphics-v0.2.0)] - 2025-03-07

### Performance

- Replace hand-coded yuv/rgb with yuvutils ([5f1c44027a](https://github.com/Devolutions/IronRDP/commit/5f1c44027a7f6da5271565461764dd3f61729ee4)) 

  cargo bench:
  to_ycbcr                time:   [2.2988 µs 2.3251 µs 2.3517 µs]
                          change: [-83.643% -83.534% -83.421%] (p = 0.00 < 0.05)
                          Performance has improved.

## [[0.1.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-graphics-v0.1.1...ironrdp-graphics-v0.1.2)] - 2025-01-28

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 



## [[0.1.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-graphics-v0.1.0...ironrdp-graphics-v0.1.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 

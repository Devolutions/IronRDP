# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-session-v0.3.0...ironrdp-session-v0.4.0)] - 2025-05-27

### <!-- 1 -->Features

- [**breaking**] Make DecodedImage Send ([45f66117ba](https://github.com/Devolutions/IronRDP/commit/45f66117ba05170d95b21ec7d97017b44b954f28)) 

- Add DecodeImage helpers ([cd7a60ba45](https://github.com/Devolutions/IronRDP/commit/cd7a60ba45a0241be4ecf3860ec4f82b431a7ce2)) 

### <!-- 4 -->Bug Fixes

- Update rectangle when applying None codecs updates (#728) ([a50cd643dc](https://github.com/Devolutions/IronRDP/commit/a50cd643dce9621f314231b7598d2fd31e4718c6)) 

- Return the correct updated region ([7507a152f1](https://github.com/Devolutions/IronRDP/commit/7507a152f14db594e4067bbc01e243cfba77770f)) 

  "update_rectangle" is set to empty(). The surface updates are then added
  by "union". But a union with an empty rectangle at (0,0) is still a
  rectangle at (0,0). We end up with big region updates rooted at (0,0)...

- Decrease verbosity of Rfx frame_index ([b31b99eafb](https://github.com/Devolutions/IronRDP/commit/b31b99eafb0aac2a5e5a610af21a4027ae5cd698)) 

- Decrease verbosity of FastPath header ([f9b6992e74](https://github.com/Devolutions/IronRDP/commit/f9b6992e74abb929f3001e76abaff5d7215e1cb4)) 


## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-session-v0.2.3...ironrdp-session-v0.3.0)] - 2025-03-12

### <!-- 7 -->Build

- Bump ironrdp-pdu

## [[0.2.3](https://github.com/Devolutions/IronRDP/compare/ironrdp-session-v0.2.2...ironrdp-session-v0.2.3)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 


## [[0.2.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-session-v0.2.1...ironrdp-session-v0.2.2)] - 2025-01-28

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 



## [[0.2.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-session-v0.2.0...ironrdp-session-v0.2.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 

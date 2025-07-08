# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.7.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-server-v0.6.1...ironrdp-server-v0.7.0)] - 2025-07-08

### <!-- 1 -->Features

- Update sspi dependency (#839) ([33530212c4](https://github.com/Devolutions/IronRDP/commit/33530212c42bf28c875ac078ed2408657831b417)) 

## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-server-v0.5.0...ironrdp-server-v0.6.0)] - 2025-05-27

### <!-- 1 -->Features

- Add stride debug info ([7f57817805](https://github.com/Devolutions/IronRDP/commit/7f578178056282e590179a10cd1eedb8f4d9ad63)) 

- Add Framebuffer helper struct ([1e87961d16](https://github.com/Devolutions/IronRDP/commit/1e87961d1611ed31f58b407f208295c97c0d2944)) 

  This will hold the updated bitmap data for the whole framebuffer.

- Add BitmapUpdate::sub() ([a76e84d459](https://github.com/Devolutions/IronRDP/commit/a76e84d45927d61e21c27abcfa31c4f0c7a17bbf)) 

- Implement some Encoder Debug ([137d91ae7a](https://github.com/Devolutions/IronRDP/commit/137d91ae7a096170ada289d420785c8f5de0663b)) 

- Keep last full-frame/desktop update ([aeb1193674](https://github.com/Devolutions/IronRDP/commit/aeb1193674641846ae1873def8c84a62a59213d5)) 

  It should reflect client drawing state.
  
  In following changes, we will fix it to draw bitmap updates on it, to
  keep it up to date.

- Find and send the damaged tiles ([fb3769c4a7](https://github.com/Devolutions/IronRDP/commit/fb3769c4a7fce56e340df8c4b19f7d90cda93e50)) 

  Keep a framebuffer and tile-diff against it, to save from
  encoding/sending the same bitmap data regions.

### <!-- 4 -->Bug Fixes

- Use desktop size for RFX channel size (#756) ([806f1d7694](https://github.com/Devolutions/IronRDP/commit/806f1d7694313b1a59842af300a437ae2f6c2463)) 

- [**breaking**] Remove time_warn! from the public API (#773) ([cc78b1e3dc](https://github.com/Devolutions/IronRDP/commit/cc78b1e3dc1c554dd3fcf6494763caa00ba28ad7)) 

  This is intended to be an internal macro.

### Refactor

- [**breaking**] Drop support for pixelOrder ([db6f4cdb7f](https://github.com/Devolutions/IronRDP/commit/db6f4cdb7f379713979b930e8e1fa1a813ebecc4)) 

  Dealing with multiple formats is sufficiently annoying, there isn't much
  need for awkward image layout. This was done for efficiency reason for
  bitmap encoding, but bitmap is really inefficient anyway and very few
  servers will actually provide bottom to top images (except with GL/GPU
  textures, but this is not in scope yet).

- [**breaking**] Use bytes, allowing shareable bitmap data ([3c43fdda76](https://github.com/Devolutions/IronRDP/commit/3c43fdda76f4ef6413db4010471364d6b1be2798)) 

- [**breaking**] Rename left/top -> x/y ([229070a435](https://github.com/Devolutions/IronRDP/commit/229070a43554927a01541052a819fe3fcd32a913)) 


## [[0.5.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-server-v0.4.2...ironrdp-server-v0.5.0)] - 2025-03-12

### <!-- 7 -->Build

- Bump ironrdp-pdu


## [[0.4.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-server-v0.4.1...ironrdp-server-v0.4.2)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 


## [[0.4.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-server-v0.4.0...ironrdp-server-v0.4.1)] - 2025-01-28

### <!-- 1 -->Features

- Advertize Bitmap::desktopResizeFlag ([a0fccf8d1a](https://github.com/Devolutions/IronRDP/commit/a0fccf8d1a3eeab6c73ed7d9cdbb4342cca173c4)) 

  This makes freerdp keep the flag up and handle desktop
  resize/deactivation-reactivation. It should be okay to advertize,
  if the server doesn't resize anyway, I guess.

- Add volume support (#641) ([a6c36511f6](https://github.com/Devolutions/IronRDP/commit/a6c36511f6584f67b8c6e795c34d5007ec2b24a4)) 

  Add server messages and API to support setting client volume.

### <!-- 4 -->Bug Fixes

- Drop unexpected PDUs during deactivation-reactivation ([63963182b5](https://github.com/Devolutions/IronRDP/commit/63963182b5af6ad45dc638e93de4b8a0b565c7d3)) 

  The current behaviour of handling unmatched PDUs in fn read_by_hint()
  isn't good enough. An unexpected PDUs may be received and fail to be
  decoded during Acceptor::step().
  
  Change the code to simply drop unexpected PDUs (as opposed to attempting
  to replay the unmatched leftover, which isn't clearly needed)

- Reattach existing channels ([c4587b537c](https://github.com/Devolutions/IronRDP/commit/c4587b537c7c0a148e11bc365bc3df88e2c92312)) 

  I couldn't find any explicit behaviour described in the specification,
  but apparently, we must just keep the channel state as they were during
  reactivation. This fixes various state issues during client resize.

- Do not restart static channels on reactivation ([82c7c2f5b0](https://github.com/Devolutions/IronRDP/commit/82c7c2f5b08c44b1a4f6b04c13ad24d9e2ffa371)) 

- Check client size ([0f9877ad39](https://github.com/Devolutions/IronRDP/commit/0f9877ad3901b37f58406095e05f345fbc8a5eaa)) 

  It's problematic when the client didn't resize, as we send bitmap
  updates that don't fit. The client will likely drop the connection.
  Let's have a warning for this case in the server.

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 



## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-server-v0.3.1...ironrdp-server-v0.4.0)] - 2024-12-17

### <!-- 1 -->Features

- [**breaking**] Make TlsIdentityCtx accept PEM files ([#623](https://github.com/Devolutions/IronRDP/pull/623)) ([9198284263](https://github.com/Devolutions/IronRDP/commit/9198284263e11706fed76310f796200b75111126)) 

  This is in general more convenient than DER files.

  This patch also includes a breaking change in the public API. 
  The `cert` field in the `TlsIdentityCtx` struct is replaced by a `certs` field containing multiple `CertificateDer` items.

## [[0.3.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-server-v0.3.0...ironrdp-server-v0.3.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 

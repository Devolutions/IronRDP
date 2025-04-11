# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.5.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpsnd-v0.4.0...ironrdp-rdpsnd-v0.5.0)] - 2025-04-11

### <!-- 1 -->Features

- Add support for client custom flags ([7bd92c0ce5](https://github.com/Devolutions/IronRDP/commit/7bd92c0ce5c686fe18c062b7edfeed46a709fc23)) 

  Client can support various flags, but always set ALIVE.

### <!-- 4 -->Bug Fixes

- Correct TrainingPdu wPackSize field ([abcc42e01f](https://github.com/Devolutions/IronRDP/commit/abcc42e01fda3ce9c8e1739524e0fc73b8778d83)) 

- Reply to TrainingPdu ([5dcc526f51](https://github.com/Devolutions/IronRDP/commit/5dcc526f513e8083ff335cad3cc80d2effeb7265)) 

- Lookup the associated format from the client list ([3d7bc28b97](https://github.com/Devolutions/IronRDP/commit/3d7bc28b9764b1f37b038bb2fbb676ec464ee5ee)) 

  This is an index to the client list, according to:
  https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpea/7df64d93-7594-4035-978d-229f2b15f1bc

- Send client formats that match server (#742) ([a8b9614323](https://github.com/Devolutions/IronRDP/commit/a8b96143236ad457b5241f6a2f8acfaf969472b6)) 

  Windows seems to be confused if the client replies with more formats, or
  unknown formats (opus).
  
  ---------

### Refactor

- [**breaking**] Pass format_no instead of AudioFormat ([4172571e8e](https://github.com/Devolutions/IronRDP/commit/4172571e8e061a6a120643393881b5e37f1e61ab)) 

  This can help avoid extra lookups and cloning.



## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpsnd-v0.3.1...ironrdp-rdpsnd-v0.4.0)] - 2025-03-12

### <!-- 7 -->Build

- Bump ironrdp-pdu

## [[0.3.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpsnd-v0.3.0...ironrdp-rdpsnd-v0.3.1)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 


## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpsnd-v0.2.0...ironrdp-rdpsnd-v0.3.0)] - 2025-02-05

### <!-- 1 -->Features

- New required method `get_formats` for the `RdpsndClientHandler` trait (#661) ([ccf6348270](https://github.com/Devolutions/IronRDP/commit/ccf63482706ecfbbdc6038028ea2ee086d0e3640)) 



## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpsnd-v0.1.1...ironrdp-rdpsnd-v0.2.0)] - 2025-01-28

### <!-- 1 -->Features

- Support for volume setting (#641) ([a6c36511f6](https://github.com/Devolutions/IronRDP/commit/a6c36511f6584f67b8c6e795c34d5007ec2b24a4)) 

  Add server messages and API to support setting client volume.

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 



## [[0.1.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpsnd-v0.1.0...ironrdp-rdpsnd-v0.1.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 

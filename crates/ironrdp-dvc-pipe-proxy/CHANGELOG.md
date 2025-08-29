# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-pipe-proxy-v0.1.0...ironrdp-dvc-pipe-proxy-v0.2.0)] - 2025-08-29

### <!-- 1 -->Features

- Make dvc named pipe proxy cross-platform (#896) ([166b76010c](https://github.com/Devolutions/IronRDP/commit/166b76010cbd8f8674e6e8d4801fee5cda1ad9e5)) 

  ### Changes
  - Make dvc named pipe proxy cross-platform (Unix implementation via
  `tokio::net::unix::UnixStream`)
  - Removed unsafe code for Windows implementation, switched to
  `tokio::net::windows::named_pipe`
  
  ### Testing
  This feature can be used in the [same
  way](https://github.com/Devolutions/IronRDP/pull/791) as on Windows,
  however instead of GUI test app there is new basic
  [CLI](https://github.com/Devolutions/now-proto/pull/31) app



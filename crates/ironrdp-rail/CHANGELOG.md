# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.1.0](https://github.com/Devolutions/IronRDP/releases/tag/ironrdp-rail-v0.1.0)] - 2026-08-18

### <!-- 1 -->Features

- Add RemoteApp protocol primitives ([#1636](https://github.com/Devolutions/IronRDP/issues/1636)) ([0161906731](https://github.com/Devolutions/IronRDP/commit/0161906731757356953cdb389a2cd6a42863deb2)) 

  Add portable RAIL wire types and a typed Remote Programs capability set.
  
  Validate the RAIL crate's bare `no_std` and allocation-backed
  configurations in the workspace feature matrix.
  
  Keep connection setup and windowing behavior outside this protocol
  layer.



# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.3.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-blocking-v0.3.0...ironrdp-blocking-v0.3.1)] - 2025-03-12

### <!-- 7 -->Build

- Do not use workspace dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 

  As written in the workspace Cargo.toml:
  
  > Note that for better cross-tooling interactions, do not use workspace
  dependencies for anything that is not "workspace internal" (e.g.: mostly
  dev-dependencies). E.g.: release-plz can’t detect that a dependency has
  been
  updated in a way warranting a version bump in the dependant if no commit
  is
  touching a file associated to the crate. It is technically okay to use
  that
  for "private" (i.e.: not used in the public API) dependencies too, but
  we
  still want to make follow-up releases to stay up to date with the
  community,
  even for private dependencies.
  
  Expectation is that release-plz will be able to auto-detect when bumping
  dependents is necessary.



## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-blocking-v0.2.1...ironrdp-blocking-v0.3.0)] - 2025-01-28

### <!-- 4 -->Changed

- Remove unmatched parameter from `Framed::read_by_hint` function ([63963182b5](https://github.com/Devolutions/IronRDP/commit/63963182b5af6ad45dc638e93de4b8a0b565c7d3))

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 



## [[0.2.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-blocking-v0.2.0...ironrdp-blocking-v0.2.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 

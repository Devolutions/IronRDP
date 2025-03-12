# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.1.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdpdr-native-v0.1.1...ironrdp-rdpdr-native-v0.1.2)] - 2025-03-12

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



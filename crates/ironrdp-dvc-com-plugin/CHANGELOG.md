# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.1.3](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-com-plugin-v0.1.2...ironrdp-dvc-com-plugin-v0.1.3)] - 2026-07-10

### <!-- 4 -->Bug Fixes

- Lower verbosity of routine logs in library crates ([c36032f91b](https://github.com/Devolutions/IronRDP/commit/c36032f91b27390a2cd34bfb300cfbe099d847a9)) 

  Library crates should not emit info! for routine, repeating operations;
  that floods the default logs of the final consumer, which owns the
  verbosity decision. Reserve info! for rare connection/session lifecycle
  milestones, debug! for significant one-off events, and trace! for the
  fine-grained detail only needed when nothing else explains a problem.



## [[0.1.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-com-plugin-v0.1.1...ironrdp-dvc-com-plugin-v0.1.2)] - 2026-06-05



## [[0.1.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-com-plugin-v0.1.0...ironrdp-dvc-com-plugin-v0.1.1)] - 2026-05-27

### <!-- 7 -->Build

- Update dependencies.

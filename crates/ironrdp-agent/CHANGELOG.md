# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.1.0](https://github.com/Devolutions/IronRDP/releases/tag/ironrdp-agent-v0.1.0)] - 2026-07-10

### <!-- 1 -->Features

- Introduce ironrdp-agent crate ([#1339](https://github.com/Devolutions/IronRDP/issues/1339)) ([2046639870](https://github.com/Devolutions/IronRDP/commit/2046639870530458479cdacec0b2eb056ee10edf)) 

  Add a CLI-driven, daemon-backed RDP client designed for programmatic
  (e.g. LLM) consumption. A single binary plays two roles: a long-lived
  daemon that owns the ironrdp-client engine and one RDP session, and a
  short-lived CLI that drives it over a local IPC transport (Unix domain
  socket / Windows named pipe).

- Add resize operation ([#1401](https://github.com/Devolutions/IronRDP/issues/1401)) ([a5522467ab](https://github.com/Devolutions/IronRDP/commit/a5522467ab958c6fc25efa57d2cb4f5a87acb064)) 

- Add generic --prop KEY:TYPE:VALUE property overrides ([#1402](https://github.com/Devolutions/IronRDP/issues/1402)) ([1cc7570ecb](https://github.com/Devolutions/IronRDP/commit/1cc7570ecba636812b3ff07e8f282bd37df12b20)) 



# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.0.1](https://github.com/Devolutions/IronRDP/releases/tag/ironrdp-mstsgu-v0.0.1)] - 2026-07-10

### <!-- 1 -->Features

- Add MS-TSGU (Microsoft RD Gateway) support ([#913](https://github.com/Devolutions/IronRDP/issues/913)) ([7d28ef83a6](https://github.com/Devolutions/IronRDP/commit/7d28ef83a67afa8f69a7170ff47f4547a5d01b1e)) 

  This adds a working state to connect with the ironrdp-client CLI against
  a server behind a microsoft remote desktop gateway.
  During my testing this was robust enough to work with sessions for more
  than 30 minutes.
  
  CLI Flags and prompts are implemented and can be mixed, so the following
  would prompt only for 2 passwords:
  > ironrdp-client --gw-user username@domain --gw-endpoint rdp.gw.host:443
  -u username@domain rdp.internal.host
  
  [MS-TSGU]
  https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-tsgu/0007d661-a86d-4e8f-89f7-7f77f8824188
  * This implements a MVP (in terms of recentness) state needed to connect
  through microsoft rdp gateway.
  * This only supports the HTTPS protocol with Websocket (and not the
  legacy HTTP, HTTP-RPC or UDP protocols).
  * This does not implement reconnection/reauthentication.
  * This only supports basic auth.
  
  Mostly looking for rough initial feedback (e.g. in terms of if there are
  parts that dont align with projects architecture or other areas needing
  major rework) as well as if the implemented scope would be considered
  complete enough to land this in the first place.

### <!-- 4 -->Bug Fixes

- Propagate caller location through error constructor helpers ([#1392](https://github.com/Devolutions/IronRDP/issues/1392)) ([d6990d81a1](https://github.com/Devolutions/IronRDP/commit/d6990d81a17e8349e52768ad8a82f673b1e1462d)) 

  The error constructor helpers in several crates wrap the #[track_caller]
  ironrdp_error::Error::new, but were not themselves marked
  #[track_caller]. As a result, the captured location pointed at the
  helper body instead of the real call site, giving misleading "@
  file:line" info in error reports.

### <!-- 6 -->Documentation

- Establish the MSRV policy (current is 1.89) ([#1157](https://github.com/Devolutions/IronRDP/issues/1157)) ([c10e6ff16c](https://github.com/Devolutions/IronRDP/commit/c10e6ff16cc45f094b24e87ed1d46eb88b4a0419)) 

  The MSRV is the oldest stable Rust release that is at least 6 months
  old, bounded by the Rust version available in Debian stable-backports
  and Fedora stable.

### <!-- 7 -->Build

- Fix failing CI on master ([#918](https://github.com/Devolutions/IronRDP/issues/918)) ([35c19ce444](https://github.com/Devolutions/IronRDP/commit/35c19ce444bcbb1b4250d00977d60b30fab42eca)) 

- Bump hyper from 1.6.0 to 1.7.0 ([#940](https://github.com/Devolutions/IronRDP/issues/940)) ([9e23597c50](https://github.com/Devolutions/IronRDP/commit/9e23597c50a8998cb8cdc5feb5ea04054e9eca6a)) 

- Bump tokio-tungstenite from 0.27.0 to 0.28.0 ([#1009](https://github.com/Devolutions/IronRDP/issues/1009)) ([d24dbb1e2c](https://github.com/Devolutions/IronRDP/commit/d24dbb1e2c580c2ca66c7d7a0cc3ce468e8e78a7)) 

- Bump uuid from 1.18.1 to 1.19.0 ([#1056](https://github.com/Devolutions/IronRDP/issues/1056)) ([113284a053](https://github.com/Devolutions/IronRDP/commit/113284a0539a6b1e232a7957d877a3593d82252a)) 

- Bump uuid from 1.19.0 to 1.20.0 ([#1087](https://github.com/Devolutions/IronRDP/issues/1087)) ([50168c8693](https://github.com/Devolutions/IronRDP/commit/50168c8693b1e96409adee2378818f90a6e3808d)) 

- Bump uuid from 1.20.0 to 1.21.0 ([#1119](https://github.com/Devolutions/IronRDP/issues/1119)) ([e406559e0d](https://github.com/Devolutions/IronRDP/commit/e406559e0d2743dbfc4105120baabb303584a699)) 

- Upgrade sspi to 0.19, picky to rc.22, fix NTLM fallback ([#1188](https://github.com/Devolutions/IronRDP/issues/1188)) ([c70d38a9f1](https://github.com/Devolutions/IronRDP/commit/c70d38a9f190d6ad6c84bd9027a388b5db3296ba)) 

- Bump tokio from 1.50.0 to 1.51.1 ([#1219](https://github.com/Devolutions/IronRDP/issues/1219)) ([d3e673b455](https://github.com/Devolutions/IronRDP/commit/d3e673b455ec817df7590cef27e598c8517828ae)) 

- Bump hyper from 1.8.1 to 1.9.0 ([#1224](https://github.com/Devolutions/IronRDP/issues/1224)) ([901970778d](https://github.com/Devolutions/IronRDP/commit/901970778d74780a3bbe6db7ec1ad2ad93c23ad1)) 

- Bump tokio from 1.51.1 to 1.52.1 ([#1223](https://github.com/Devolutions/IronRDP/issues/1223)) ([8bf140f49d](https://github.com/Devolutions/IronRDP/commit/8bf140f49d3bee952e395ffeb514a27c4725eb15)) 

- Bump the patch group across 1 directory with 2 updates ([#1222](https://github.com/Devolutions/IronRDP/issues/1222)) ([3fe6d157e0](https://github.com/Devolutions/IronRDP/commit/3fe6d157e0b55bddfdac20af290a6cfa6e550576)) 

- Bump tokio-tungstenite from 0.28.0 to 0.29.0 ([#1234](https://github.com/Devolutions/IronRDP/issues/1234)) ([f6fa5779a1](https://github.com/Devolutions/IronRDP/commit/f6fa5779a1cc131ce37b83d7457b8e454e695b31)) 

- Align sspi and picky dependencies ([#1385](https://github.com/Devolutions/IronRDP/issues/1385)) ([0a461b5d36](https://github.com/Devolutions/IronRDP/commit/0a461b5d366677fd2f0f664a4f0074e4ab697c42)) 



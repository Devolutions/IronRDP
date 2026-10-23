# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.6.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-pipe-proxy-v0.5.0...ironrdp-dvc-pipe-proxy-v0.6.0)] - 2026-10-23

### <!-- 1 -->Features

- [**breaking**] Tolerate unrecognized device-reported USB capability values ([#1418](https://github.com/Devolutions/IronRDP/issues/1418)) ([b2d75d4862](https://github.com/Devolutions/IronRDP/commit/b2d75d4862b4549b0a63cf9b26f02c3e8813014b)) 

  ## Problem

### <!-- 4 -->Bug Fixes

- Send pipe proxy data on its own channel ([#2209](https://github.com/Devolutions/IronRDP/issues/2209)) ([10199db3b0](https://github.com/Devolutions/IronRDP/commit/10199db3b0aeca629adc02234ffdbb570bb77d43)) 

  `DvcNamedPipeProxy` bridges a dynamic virtual channel to a local named
  pipe. When the server opens the channel, `DvcProcessor::start` receives
  its channel ID and starts a worker for it. The worker reads what a local
  client writes to the pipe, encodes it as DVC data with
  `encode_dvc_messages(channel_id, ...)`, and hands the encoded messages
  to the write callback given to `DvcNamedPipeProxy::new`, which takes a
  channel ID and the messages.
  
  The worker encoded the messages for the real channel ID but called the
  callback with channel ID 0. `ironrdp-client` uses the callback's channel
  ID to route the messages to the transport that carries that channel,
  either the main connection or a multitransport tunnel selected by
  Soft-Sync. Before routing, it wraps them with
  `DvcMessageBatch::try_new(channel_id, messages)`, which decodes each
  message and checks that it is DVC data addressed to that channel. With 0
  passed alongside messages encoded for another channel, the check fails
  with "DVC message channel ID does not match its batch", the error is
  returned from the session loop, and the RDP session ends. Any pipe proxy
  therefore dropped the connection as soon as its pipe client wrote its
  first message; in particular, `ironrdp-agent now` commands could not
  reach the remote agent over `Devolutions::Now::Agent`.
  
  The worker now passes the channel ID it was started with, which is the
  ID its messages are encoded for, so the batch check accepts them and
  they are routed with the channel. The other write callbacks wired into
  `ironrdp-client`, the DVC COM plugin host and the RDPEWA client, already
  pass the channel ID they encode with.
  
  The pipe proxy tests in `ironrdp-testsuite-extra` checked only that pipe
  data reached the callback as at least one message. A new test starts the
  proxy on a non-zero channel, writes to the pipe, and checks that the
  callback receives that channel ID and that `DvcMessageBatch::try_new`
  accepts the messages. The existing tests now also decode the forwarded
  DVC data PDUs and check that they carry exactly the bytes written to the
  pipe. Like the existing pipe proxy tests, these run on Windows only.
  
  🤖 Generated with [Claude Code](https://claude.com/claude-code)
  
  ---------



## [[0.5.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-pipe-proxy-v0.4.1...ironrdp-dvc-pipe-proxy-v0.5.0)] - 2026-07-10

### <!-- 4 -->Bug Fixes

- Lower verbosity of routine logs in library crates ([c36032f91b](https://github.com/Devolutions/IronRDP/commit/c36032f91b27390a2cd34bfb300cfbe099d847a9)) 

  Library crates should not emit info! for routine, repeating operations;
  that floods the default logs of the final consumer, which owns the
  verbosity decision. Reserve info! for rare connection/session lifecycle
  milestones, debug! for significant one-off events, and trace! for the
  fine-grained detail only needed when nothing else explains a problem.

- Remove trailing punctuation from log messages ([#1380](https://github.com/Devolutions/IronRDP/issues/1380)) ([f38554277a](https://github.com/Devolutions/IronRDP/commit/f38554277a1af3085c1fa5739cda515939d09abf)) 

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-pdu` public dependency to 0.9

- [**breaking**] Update `ironrdp-svc` public dependency to 0.8



## [[0.4.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-pipe-proxy-v0.4.0...ironrdp-dvc-pipe-proxy-v0.4.1)] - 2026-06-05



## [[0.4.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-pipe-proxy-v0.3.0...ironrdp-dvc-pipe-proxy-v0.4.0)] - 2026-05-27

### <!-- 7 -->Build

- [**breaking**] Update `ironrdp-pdu` and `ironrdp-svc` public dependencies

## [[0.2.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-pipe-proxy-v0.2.0...ironrdp-dvc-pipe-proxy-v0.2.1)] - 2025-09-24

### <!-- 4 -->Bug Fixes

- Change dvc proxy pipe mode from Message to Byte on Windows (#986) ([5f52a44b84](https://github.com/Devolutions/IronRDP/commit/5f52a44b840dd71eae6a355be00f1c4c671b3b58)) 

- Add blocking logic for sending dvc pipe messages ([3182a018e2](https://github.com/Devolutions/IronRDP/commit/3182a018e2972eb77c52ea248387c96a9eb6a6a6)) 

## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-dvc-pipe-proxy-v0.1.0...ironrdp-dvc-pipe-proxy-v0.2.0)] - 2025-08-29

### <!-- 1 -->Features

- Make dvc named pipe proxy cross-platform (#896) ([166b76010c](https://github.com/Devolutions/IronRDP/commit/166b76010cbd8f8674e6e8d4801fee5cda1ad9e5)) 

  - Make dvc named pipe proxy cross-platform (Unix implementation via
  `tokio::net::unix::UnixStream`)
  - Removed unsafe code for Windows implementation, switched to
  `tokio::net::windows::named_pipe`

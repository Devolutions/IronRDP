# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.1.0](https://github.com/Devolutions/IronRDP/releases/tag/ironrdp-viewer-v0.1.0)] - 2026-07-10

### <!-- 1 -->Features

- Split ironrdp-client into a reusable library + ironrdp-viewer binary ([#1309](https://github.com/Devolutions/IronRDP/issues/1309)) ([361bdc2fe8](https://github.com/Devolutions/IronRDP/commit/361bdc2fe87739b7ffb8a2eb1705d5014c9f209b)) 

- Gate native backends behind Cargo features ([#1338](https://github.com/Devolutions/IronRDP/issues/1338)) ([f7e6106e0f](https://github.com/Devolutions/IronRDP/commit/f7e6106e0f293c1e0f8129be82aa2d86737ba92a)) 

  ironrdp (meta crate):
  - Added:    client, client-all, client-sound, client-clipboard,
              client-rdpdr, client-smartcard, client-gateway,
              client-dvc-pipe-proxy, client-dvc-com-plugin, and
              top-level rustls / native-tls (forwarded to ironrdp-client)
  - Modified: qoi, qoiz now also gate ironrdp-client's codec



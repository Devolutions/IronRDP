# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.2.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-tls-v0.2.0...ironrdp-tls-v0.2.1)] - 2026-03-13

### <!-- 6 -->Documentation

- Establish the MSRV policy (current is 1.89) ([#1157](https://github.com/Devolutions/IronRDP/issues/1157)) ([c10e6ff16c](https://github.com/Devolutions/IronRDP/commit/c10e6ff16cc45f094b24e87ed1d46eb88b4a0419)) 

  The MSRV is the oldest stable Rust release that is at least 6 months
  old, bounded by the Rust version available in Debian stable-backports
  and Fedora stable.



## [[0.2.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-tls-v0.1.4...ironrdp-tls-v0.2.0)] - 2025-12-18

### <!-- 1 -->Features

- [**breaking**] Return x509_cert::Certificate from upgrade() ([#1054](https://github.com/Devolutions/IronRDP/issues/1054)) ([bd2aed7686](https://github.com/Devolutions/IronRDP/commit/bd2aed76867f4038c32df9a0d24532ee40d2f14c)) 

  This allows client applications to verify details of the certificate,
  possibly with the user, when connecting to a server using TLS.

## [[0.1.4](https://github.com/Devolutions/IronRDP/compare/ironrdp-tls-v0.1.3...ironrdp-tls-v0.1.4)] - 2025-08-29

### <!-- 7 -->Build

- Bump tokio from 1.46.1 to 1.47.0 (#893) ([5d513dcf09](https://github.com/Devolutions/IronRDP/commit/5d513dcf099505d4d52fe25884dc019590bc751e)) 

## [[0.1.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-tls-v0.1.1...ironrdp-tls-v0.1.2)] - 2025-01-28

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 

### <!-- 7 -->Build

- Bump tokio from 1.42.0 to 1.43.0 (#650) ([ff6c6e875b](https://github.com/Devolutions/IronRDP/commit/ff6c6e875b4c2dce7ec109c3721739f86a808a31)) 

## [[0.1.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-tls-v0.1.0...ironrdp-tls-v0.1.1)] - 2024-12-14

### Other

- Symlinks to license files in packages ([#604](https://github.com/Devolutions/IronRDP/pull/604)) ([6c2de344c2](https://github.com/Devolutions/IronRDP/commit/6c2de344c2dd93ce9621834e0497ed7c3bfaf91a)) 

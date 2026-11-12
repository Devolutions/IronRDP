# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.3.0](https://github.com/Devolutions/IronRDP/compare/ironrdp-tls-v0.2.2...ironrdp-tls-v0.3.0)] - 2026-10-23

### <!-- 1 -->Features

- Add `CertificateValidation` and `upgrade_with_certificate_validation` to opt into certificate-chain and server-name validation against the platform trust store, while `upgrade` keeps accepting any certificate ([#1520](https://github.com/Devolutions/IronRDP/issues/1520)) ([f1d53c78d3](https://github.com/Devolutions/IronRDP/commit/f1d53c78d390de1c6778773cdc859d59901466f0))

- Add `upgrade_with_certificate_validation_callback`, `upgrade_with_certificate_validation_callback_for_endpoint` and `CertificateValidationCallback` so callers can accept a certificate that fails strict validation, for example by pinning it per endpoint; only the rustls backend supports them ([#1520](https://github.com/Devolutions/IronRDP/issues/1520)) ([f1d53c78d3](https://github.com/Devolutions/IronRDP/commit/f1d53c78d390de1c6778773cdc859d59901466f0)) ([#1775](https://github.com/Devolutions/IronRDP/issues/1775)) ([312d4466e7](https://github.com/Devolutions/IronRDP/commit/312d4466e7cae15d45c73a4ffcb4ae730d5e6a30))

- Add the `rustls-verifier` feature and `rustls_client_config` to build a rustls client configuration with the same validation policy without selecting rustls as the stream backend ([#1811](https://github.com/Devolutions/IronRDP/issues/1811)) ([7292970955](https://github.com/Devolutions/IronRDP/commit/729297095564a9640183556078cf8726dd3afeed))

### <!-- 7 -->Build

- [**breaking**] Update the public `x509-cert` dependency to 0.3 ([#1449](https://github.com/Devolutions/IronRDP/issues/1449)) ([e1725e8c8a](https://github.com/Devolutions/IronRDP/commit/e1725e8c8a581b83835647b6ee563a5b3f6c7a1b))

- Raise the minimum supported Rust version to 1.94 ([#1495](https://github.com/Devolutions/IronRDP/issues/1495)) ([0aeea76e95](https://github.com/Devolutions/IronRDP/commit/0aeea76e95e6b6b590d65be353dd00596d50a895))



## [[0.2.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-tls-v0.2.1...ironrdp-tls-v0.2.2)] - 2026-07-10

### <!-- 1 -->Features

- Expose negotiated TLS version and cipher suite ([#1384](https://github.com/Devolutions/IronRDP/issues/1384)) ([8f76260ea7](https://github.com/Devolutions/IronRDP/commit/8f76260ea753f546a577ad7a1176a5740adc94cf)) 

  Adds a backend-neutral way to query the TLS parameters negotiated for an established
  ironrdp-tls::TlsStream, enabling downstream diagnostic tooling to report the negotiated
  protocol version and cipher suite alongside the existing certificate information.

- Gate native backends behind Cargo features ([#1338](https://github.com/Devolutions/IronRDP/issues/1338)) ([f7e6106e0f](https://github.com/Devolutions/IronRDP/commit/f7e6106e0f293c1e0f8129be82aa2d86737ba92a)) 


- Make the rustls crypto provider selectable ([#1387](https://github.com/Devolutions/IronRDP/issues/1387)) ([d767d99032](https://github.com/Devolutions/IronRDP/commit/d767d990325448bf3385974da7ea9b6dcc477673)) 

  Makes the ironrdp-tls rustls backend’s crypto provider selectable at compile time by restructuring Cargo features, avoiding forcing a single provider onto downstreams via tokio-rustls default features.



## [[0.2.1](https://github.com/Devolutions/IronRDP/compare/ironrdp-tls-v0.2.0...ironrdp-tls-v0.2.1)] - 2026-05-27

### <!-- 7 -->Build

- Bump tokio from 1.50.0 to 1.52.1 ([#1219](https://github.com/Devolutions/IronRDP/issues/1219)) ([d3e673b455](https://github.com/Devolutions/IronRDP/commit/d3e673b455ec817df7590cef27e598c8517828ae)) ([#1223](https://github.com/Devolutions/IronRDP/issues/1223)) ([8bf140f49d](https://github.com/Devolutions/IronRDP/commit/8bf140f49d3bee952e395ffeb514a27c4725eb15)) 

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

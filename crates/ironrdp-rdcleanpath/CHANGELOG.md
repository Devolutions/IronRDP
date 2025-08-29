# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.1.4](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdcleanpath-v0.1.3...ironrdp-rdcleanpath-v0.1.4)] - 2025-08-29

### <!-- 1 -->Features

- Preserve RDP negotiation failure details in RDCleanPath error responses (#930) ([ca11e338d7](https://github.com/Devolutions/IronRDP/commit/ca11e338d7231c86f60a110627a5d864377d8594)) 

  * Both web and desktop clients check for X.224 negotiation failure data
  in RDCleanPath error responses before falling back to generic errors
  * When X.224 Connection Confirm failure is found, convert to specific
  NegotiationFailure error type instead of generic RDCleanPath error
  * Enable clients to show meaningful error messages like "CredSSP
  authentication required" instead of generic connection failures
  * Maintain backward compatibility - existing proxies sending empty
  x224_connection_pdu continue working as before
  * Helper for proxies creating an RDCleanPath error with server response



## [[0.1.3](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdcleanpath-v0.1.2...ironrdp-rdcleanpath-v0.1.3)] - 2025-03-12

### <!-- 7 -->Build

- Update dependencies (#695) ([c21fa44fd6](https://github.com/Devolutions/IronRDP/commit/c21fa44fd6f3c6a6b74788ff68e83133c1314caa)) 

## [[0.1.2](https://github.com/Devolutions/IronRDP/compare/ironrdp-rdcleanpath-v0.1.1...ironrdp-rdcleanpath-v0.1.2)] - 2025-01-28

### <!-- 6 -->Documentation

- Use CDN URLs instead of the blob storage URLs for Devolutions logo (#631) ([dd249909a8](https://github.com/Devolutions/IronRDP/commit/dd249909a894004d4f728d30b3a4aa77a0f8193b)) 



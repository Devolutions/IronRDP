# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [[0.1.0](https://github.com/Devolutions/IronRDP/releases/tag/ironrdp-propertyset-v0.1.0)] - 2026-07-10

### <!-- 1 -->Features

- Inital support for .RDP files ([#862](https://github.com/Devolutions/IronRDP/issues/862)) ([c710909a3c](https://github.com/Devolutions/IronRDP/commit/c710909a3cb64808bfc024bbe3f326565268871e)) 

  This is paving the way for .rdp file support.

### <!-- 4 -->Bug Fixes

- Remove logging from PropertySet ([#1393](https://github.com/Devolutions/IronRDP/issues/1393)) ([1b752d282a](https://github.com/Devolutions/IronRDP/commit/1b752d282a11fc9bda5d4e051414e414b7eec50d)) 

  The debug! logging in insert/remove/get stringified raw keys and values
  on every access, which exposed secrets such as ClearTextPassword,
  gateway_password and rdcleanpath_token in logs. Observed debugging value
  was low, so the logging is removed entirely rather than redacted, and
  the tracing dependency is dropped.

### <!-- 6 -->Documentation

- Establish the MSRV policy (current is 1.89) ([#1157](https://github.com/Devolutions/IronRDP/issues/1157)) ([c10e6ff16c](https://github.com/Devolutions/IronRDP/commit/c10e6ff16cc45f094b24e87ed1d46eb88b4a0419)) 

  The MSRV is the oldest stable Rust release that is at least 6 months
  old, bounded by the Rust version available in Debian stable-backports
  and Fedora stable.



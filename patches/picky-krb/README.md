[![Crates.io](https://img.shields.io/crates/v/picky-krb.svg)](https://crates.io/crates/picky-krb)
[![docs.rs](https://docs.rs/picky-krb/badge.svg)](https://docs.rs/picky-krb)
![Crates.io](https://img.shields.io/crates/l/picky-krb)

Compatible with rustc 1.85.
Minimal rustc version bumps happen [only with minor number bumps in this project](https://github.com/Devolutions/picky-rs/issues/89#issuecomment-868303478).

# picky-krb

Provides implementation for types defined in [RFC 4120](https://www.rfc-editor.org/rfc/rfc4120.txt).

## Serializing and deserializing Kerberos structures

Use `picky_asn1_der::from_bytes` for deserialization from binary, for example:

```rust
use picky_krb::messages::AsRep;
let as_rep: AsRep = picky_asn1_der::from_bytes(&raw).unwrap();
```

And `picky_asn1_der::to_vec` for serialization to binary, for example:

```rust
use picky_krb::messages::TgsReq;
let tgs_req: TgsReq = picky_asn1_der::from_bytes(&raw).unwrap();
let tgs_req_encoded = picky_asn1_der::to_vec(&tgs_req).unwrap();
```

# IronRDP TLS

TLS boilerplate common with most IronRDP clients.

This crate exposes three features for selecting the TLS backend:

- `rustls`: use the rustls crate.
- `native-tls`: use the native-tls crate.
- `stub`: use a stubbed backend which fail at runtime when used.

These features are mutually exclusive and only one may be enabled at a time.
When more than one backend is enabled, a compile-time error is emitted.
For this reason, no feature is enabled by default.

The rationale is two-fold:

- It makes deliberate the choice of the TLS backend.
- It eliminates the risk of mistakenly enabling multiple backends at once.

With this approach, it’s obvious which backend is enabled when looking at the dependency declaration:

```toml
# This:
ironrdp-tls = { version = "x.y.z", features = ["rustls"] }

# Instead of:
ironrdp-tls = "x.y.z"
```

There is also no default feature to disable:

```toml
# This:
ironrdp-tls = { version = "x.y.z", features = ["native-tls"] }

# Instead of:
ironrdp-tls = { version = "x.y.z", default-features = false, features = ["native-tls"] }
```

This is typically more convenient and less error-prone when re-exposing the features from another crate.

```toml
[features]
rustls = ["ironrdp-tls/rustls"]
native-tls = ["ironrdp-tls/native-tls"]
stub-tls = ["ironrdp-tls/stub"]

# This:
[dependencies]
ironrdp-tls = "x.y.z"

# Instead of:
[dependencies]
ironrdp-tls = { version = "x.y.z", default-features = false }
```

(This is worse when the crate is exposing other default features which are typically not disabled by default.)

The stubbed backend is provided as an easy way to make the code compiles with minimal dependencies if required.

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP

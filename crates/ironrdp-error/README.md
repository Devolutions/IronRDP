# IronRDP Error

A lightweight and `no_std`-compatible generic `Error` type with explicit mapping between typed error domains.

## Mapping typed errors

Implement `ErrorMapping` on an outer kind for each canonical inner kind it accepts, then import
`ResultExt` to convert `Result<T, Error<InnerKind>>` values:

```rust
use core::fmt;
use ironrdp_error::{Error, ErrorMapping, ResultExt as _};

#[derive(Debug)]
enum ParseKind {
    Invalid,
}

impl fmt::Display for ParseKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid input")
    }
}

#[derive(Debug)]
enum RequestKind {
    Parse,
    ParseError(Error<ParseKind>),
}

impl fmt::Display for RequestKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse => write!(f, "request parsing failed"),
            Self::ParseError(error) => write!(f, "request parsing failed: {error}"),
        }
    }
}

impl ErrorMapping<ParseKind> for RequestKind {
    fn map_error(error: Error<ParseKind>) -> Error<Self> {
        Error::new("request", Self::ParseError(error))
    }
}

fn parse() -> Result<(), Error<ParseKind>> {
    Err(Error::new("field", ParseKind::Invalid))
}

// Uses the explicit canonical mapping above.
let _ = parse().map_err_as::<RequestKind>();

// Use this escape hatch when the outer variant should carry the complete inner error.
let _ = parse().map_err_kind("request", RequestKind::ParseError);
```

When the inner kind implements `core::error::Error`, use `map_err_source` to retain the inner
error as the source of a coarse outer kind. Source storage requires the `alloc` feature:

```rust
# use ironrdp_error::{Error, ResultExt as _};
# use core::fmt;
# #[derive(Debug)]
# enum ParseKind { Invalid }
# impl fmt::Display for ParseKind {
#     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "invalid input") }
# }
# impl core::error::Error for ParseKind {}
# #[derive(Debug)]
# enum RequestKind { Parse }
# impl fmt::Display for RequestKind {
#     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "request parsing failed") }
# }
# fn parse() -> Result<(), Error<ParseKind>> { Err(Error::new("field", ParseKind::Invalid)) }
let _ = parse().map_err_source("request", RequestKind::Parse);
```

No blanket `ErrorMapping` implementation is provided: each outer kind controls which inner kinds
have canonical mappings.

This crate is part of the [IronRDP] project.

[IronRDP]: https://github.com/Devolutions/IronRDP

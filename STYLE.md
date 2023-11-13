Our approach to "clean code" is two-fold:
- we avoid blocking PRs on style changes, but
- at the same time, the codebase is constantly refactored.

It is explicitly OK for a reviewer to flag only some nits in the PR, and then send a follow-up cleanup PR for things which are easier to explain by example, cc'ing the original author.
Sending small cleanup PRs (like renaming a single local variable) is encouraged.
These PRs are easy to merge and very welcomed.

When reviewing pull requests prefer extending this document to leaving non-reusable comments on the pull request itself.

# Style

## Formatting for sizes / lengths (e.g.: in `Encode::size()` and `FIXED_PART_SIZE` definitions)

Use an inline comment for each field of the structure.

```rust
// GOOD
const FIXED_PART_SIZE: usize = 1 /* Version */ + 1 /* Endianness */ + 2 /* CommonHeaderLength */ + 4 /* Filler */;

// GOOD
const FIXED_PART_SIZE: usize = 1 // Version
  + 1 // Endianness
  + 2 // CommonHeaderLength
  + 4; // Filler

// GOOD
fn size(&self) -> usize {
  4 // ReturnCode
  + 4 // cBytes
  + self.reader_names.size() // mszReaderNames
  + 4 // dwState
  + 4 // dwProtocol
  + self.atr.len() // pbAtr
  + 4 // cbAtrLen
}

// BAD
const FIXED_PART_SIZE: usize = 1 + 1 + 2 + 4;

// BAD
const FIXED_PART_SIZE: usize = size_of::<u8>() + size_of::<u8>() + size_of::<u16>() + size_of::<u32>();

// BAD
fn size(&self) -> usize {
  size_of::<u32>() * 5 + self.reader_names.size() + self.atr.len()
}

```

**Rationale**: boring and readable, having a comment with the name of the field is useful when following along the documentation.
Here is an excerpt illustrating this:

![Documentation excerpt](https://user-images.githubusercontent.com/3809077/272724889-681a83c9-aa83-4f48-85f4-0721c3148508.png)

`size_of::<u8>()` by itself is not really more useful than writing `1` directly.
The size of `u8` is not going to change, and it’s not hard to predict.
The struct also does not necessarily directly hold a `u8` as-is, and it may be hard to correlate a wrapper type with the corresponding `size_of::<u8>()`.
The memory representation of the wrapper type may differ from its network representation, so it’s not possible to always replace with `size_of::<Wrapper>()` instead.

## Error handling

### Return type

Use `crate_name::Result` (e.g.: `anyhow::Result`) rather than just `Result`.

**Rationale:** makes it immediately clear what result that is.

Exception: it’s not necessary when the type alias is clear enough (e.g.: `ConnectionResult`).

### Formatting of error messages

A single sentence which:
- is short and concise,
- does not start by a capital letter, and
- does not contain trailing punctuation.

This is the convention adopted by the Rust project:
- [Rust API Guidelines][api-guidelines-errors]
- [std::error::Error][std-error-trait]

```rust
// GOOD
"invalid X.509 certificate"

// BAD
"Invalid X.509 certificate."
```

**Rationale**: it’s easier to compose with other error messages. 

To illustrate with terminal error reports:
```
// GOOD
Error: invalid server license, caused by invalid X.509 certificate, caused by unexpected ASN.1 DER tag: expected SEQUENCE, got CONTEXT-SPECIFIC [19] (primitive)

// BAD
Error: Invalid server license., Caused by Invalid X.509 certificate., Caused by Unexpected ASN.1 DER tag: expected SEQUENCE, got CONTEXT-SPECIFIC [19] (primitive)
```

The error reporter (e.g.: `ironrdp_error::ErrorReport`) is responsible for adding the punctuation and/or capitalizing the text down the line.

[api-guidelines-errors]: https://rust-lang.github.io/api-guidelines/interoperability.html#error-types-are-meaningful-and-well-behaved-c-good-err
[std-error-trait]: https://doc.rust-lang.org/stable/std/error/trait.Error.html

## Logging

If any, the human-readable message should start with a capital letter and not end with a period.

```rust
// GOOD
info!("Connect to RDP host");

// BAD
info!("connect to RDP host.");
```

**Rationale**: consistency.
Log messages are typically not composed together like error messages, so it’s fine to start with a capital letter.

Use tracing ability to [record structured fields][tracing-fields].

```rust
// GOOD
info!(%server_addr, "Looked up server address");

// BAD
info!("Looked up server address: {server_addr}");
```

**Rationale**: structured diagnostic information is tracing’s strength.
It’s possible to retrieve the records emitted by tracing in a structured manner.

Name fields after what already exist consistently as much as possible.
For example, errors are typically recorded as fields named `error`.

```rust
// GOOD
error!(?error, "Active stage failed");
error!(error = ?e, "Active stage failed");
error!(%error, "Active stage failed");
error!(error = format!("{err:#}"), "Active stage failed");

// BAD
error!(?e, "Active stage failed");
error!(%err, "Active stage failed");
```

**Rationale**: consistency.
We can rely on this to filter and collect diagnostics.

[tracing-fields]: https://docs.rs/tracing/latest/tracing/index.html#recording-fields

## Helper functions

Avoid creating single-use helper functions:

```rust
// GOOD
let buf = {
    let mut buf = WriteBuf::new();
    buf.write_u32(42);
    buf
};

// BAD
let buf = prepare_buf(42);

// Somewhere else
fn prepare_buf(value: u32) -> WriteBuf {
    let mut buf = WriteBuf::new();
    buf.write_u32(value);
    buf
}
```

**Rationale:** single-use functions change frequently, adding or removing parameters adds churn.
A block serves just as well to delineate a bit of logic, but has access to all the context.
Re-using originally single-purpose function often leads to bad coupling.

Exception: if you want to make use of `return` or `?`.

## Documentation

### Doc comments should link to reference documents

Add links to specification and/or other relevant documents in doc comments.
Include verbatim the name of the section or the description of the item from the specification.
Use reference-style links for readability.
Do not make the link too long.

```rust
// GOOD

/// [2.2.3.3.8] Server Drive Query Information Request (DR_DRIVE_QUERY_INFORMATION_REQ)
///
/// The server issues a query information request on a redirected file system device.
///
/// [2.2.3.3.8]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/e43dcd68-2980-40a9-9238-344b6cf94946
pub struct ServerDriveQueryInformationRequest {
  /* snip */
}

// BAD (no doc comment)

pub struct ServerDriveQueryInformationRequest {
  /* snip */
}

// BAD (non reference-style links make barely readable, very long lines)

/// [2.2.3.3.8](https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/e43dcd68-2980-40a9-9238-344b6cf94946) Server Drive Query Information Request (DR_DRIVE_QUERY_INFORMATION_REQ)
///
/// The server issues a query information request on a redirected file system device.
pub struct ServerDriveQueryInformationRequest {
  /* snip */
}

// BAD (long link)

/// [2.2.3.3.8 Server Drive Query Information Request (DR_DRIVE_QUERY_INFORMATION_REQ)]
///
/// The server issues a query information request on a redirected file system device.
///
/// [2.2.3.3.8 Server Drive Query Information Request (DR_DRIVE_QUERY_INFORMATION_REQ)]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/e43dcd68-2980-40a9-9238-344b6cf94946
pub struct ServerDriveQueryInformationRequest {
  /* snip */
}
```

**Rationale**: consistency.
Easy cross-referencing between code and reference documents.

### Inline code comments are proper sentences

Style inline code comments as proper sentences.
Start with a capital letter, end with a dot.

```rust
// GOOD

// When building a library, `-` in the artifact name are replaced by `_`.
let artifact_name = format!("{}.wasm", package.replace('-', "_"));

// BAD

// when building a library, `-` in the artifact name are replaced by `_`
let artifact_name = format!("{}.wasm", package.replace('-', "_"));
```

**Rationale:** writing a sentence (or maybe even a paragraph) rather just "a comment" creates a more appropriate frame of mind.
It tricks you into writing down more of the context you keep in your head while coding.

### "Sentence per line" style

For `.md` and `.adoc` files, prefer a sentence-per-line format, don't wrap lines.
If the line is too long, you want to split the sentence in two.

**Rationale:** much easier to edit the text and read the diff, see [this link][asciidoctor-practices].

[asciidoctor-practices]: https://asciidoctor.org/docs/asciidoc-recommended-practices/#one-sentence-per-line

## Context parameters

Some parameters are threaded unchanged through many function calls.
They determine the "context" of the operation.
Pass such parameters first, not last.
If there are several context parameters, consider [packing them into a `struct Ctx` and passing it as `&self`][ra-ctx-struct].

```rust
// GOOD
fn do_something(connector: &mut ClientConnector, certificate: &[u8]) {
  let public_key = extract_public_key(certificate);
  do_something_else(connector, public_key, |kind| /* … */);
}

fn do_something_else(connector: &mut ClientConnector, public_key: &[u8], op: impl Fn(KeyKind) -> bool) {
  /* ... */
}

// BAD
fn do_something(certificate: &[u8], connector: &mut ClientConnector) {
  let public_key = extract_public_key(certificate);
  do_something_else(|kind| /* … */, connector, public_key);
}

fn do_something_else(op: impl Fn(KeyKind) -> bool, connector: &mut ClientConnector, public_key: &[u8]) {
  /* ... */
}
```

**Rationale:** consistency.
Context-first works better when non-context parameter is a lambda.

[ra-ctx-struct]: https://github.com/rust-lang/rust-analyzer/blob/76633199f4316b9c659d4ec0c102774d693cd940/crates/ide-db/src/path_transform.rs#L192-L339

# Runtime and compile time performance

## Avoid allocations

Avoid writing code which is slower than it needs to be.
Don't allocate a `Vec` where an iterator would do, don't allocate strings needlessly.

```rust
// GOOD
let second_word = text.split(' ').nth(1)?;

// BAD
let words: Vec<&str> = text.split(' ').collect();
let second_word = words.get(1)?;
```

**Rationale:** not allocating is almost always faster.

## Push allocations to the call site

If allocation is inevitable, let the caller allocate the resource:

```rust
// GOOD
fn frobnicate(s: String) {
    /* snip */
}

// BAD
fn frobnicate(s: &str) {
    let s = s.to_string();
    /* snip */
}
```

**Rationale:** reveals the costs.
It is also more efficient when the caller already owns the allocation.

## Avoid monomorphization

Avoid making a lot of code type parametric, *especially* on the boundaries between crates.

```rust
// GOOD
fn frobnicate(f: impl FnMut()) {
    frobnicate_impl(&mut f)
}
fn frobnicate_impl(f: &mut dyn FnMut()) {
    /* lots of code */
}

// BAD
fn frobnicate(f: impl FnMut()) {
    /* lots of code */
}
```

Avoid `AsRef` polymorphism, it pays back only for widely used libraries:

```rust
// GOOD
fn frobnicate(f: &Path) { }

// BAD
fn frobnicate(f: impl AsRef<Path>) { }
```

**Rationale:** Rust uses monomorphization to compile generic code, meaning that for each instantiation of a generic functions with concrete types, the function is compiled afresh, *per crate*.
This allows for fantastic performance, but leads to increased compile times.
Runtime performance obeys the 80/20 rule (Pareto Principle) — only a small fraction of code is hot.
Compile time **does not** obey this rule — all code has to be compiled.

---
name: rust-style
description: Apply the Devolutions Rust style conventions while writing or editing Rust. Use whenever adding or changing Rust code in this repository, including error messages, log statements, comments, invariants, size computations, and function signatures, even when the user does not mention style.
---

# Rust style

Error messages are one short sentence with no trailing punctuation and no capital added just for sentence position, while keeping proper casing for acronyms (`"invalid X.509 certificate"`), because reporters compose them into chains.
Return a crate-qualified result such as `io::Result`, not a bare `Result`, unless the alias is already clear (`ConnectionResult`).

Log messages start with a capital and have no trailing period.
Record data as structured fields rather than interpolating it (`info!(%server_addr, "Looked up server address")`), and name error fields `error`.
Reserve `info!` for rare lifecycle milestones, `debug!` for significant one-off events, and `trace!` for anything that repeats.

Inline comments and doc comments are full sentences, one sentence per line, with a capital and a period; brief field tags such as `// VER` need neither.
Doc comments link to the governing specification with reference-style links and quote the section title.

Write each field's size with its name (`4 /* ReturnCode */`), never bare arithmetic or `size_of`.

Define invariants with an `INVARIANT:` comment in the field's doc comment, before the loop, at the code for a local invariant, or in the function doc for outputs.
Reference them without the prefix when justifying an assumption or a lint suppression.
State checks positively (`if !(idx < len)`) and prefer `<` and `<=` over `>` and `>=`.

Pass context parameters first.
Put nested helpers at the end of the enclosing function after a `return`, one level deep.
Put `#[cfg(test)]` items and test modules at the end of the file, after all production items.
Prefer a block over a single-use helper unless `?` or `return` is needed.

Prefer iterators over temporary collections and let callers own allocations.
Keep generics off crate boundaries: take `&Path` rather than `impl AsRef<Path>`, and move the body of a generic function into a `dyn` implementation.

# Rust forms

Each rule in `SKILL.md` takes these forms in Rust.

In a doctest, write the marker as a hidden line, `# // oracle: ...`, so it stays out of the rendered documentation.

Properties use `proptest`, whose generators are the crate's strategies.

Snapshots use `expect-test`, whose `UPDATE_EXPECT=1` rewrites every stale value after a representation change.
Use `expect_file!` for a snapshot in a sibling file and `include_bytes!` for a blob.

Cases are `rstest`, each named with `#[case::name]`.
A cross product is enum-valued `#[values]`, so each combination's name says what it checks.
A regression test is not an `rstest` case.

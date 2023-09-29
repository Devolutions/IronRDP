use crate::prelude::*;

const EXTRA_LINTS: &[&str] = &[
    // == Safer unsafe == //
    "unsafe_op_in_unsafe_fn",
    "invalid_reference_casting",
    "pointer_structural_match",
    "clippy::undocumented_unsafe_blocks",
    "clippy::multiple_unsafe_ops_per_block",
    "clippy::transmute_ptr_to_ptr",
    "clippy::as_ptr_cast_mut",
    "clippy::cast_ptr_alignment",
    "clippy::fn_to_numeric_cast_any",
    "clippy::ptr_cast_constness",
    // == Correctness == //
    "unused_tuple_struct_fields",
    "clippy::arithmetic_side_effects",
    "clippy::cast_lossless",
    "clippy::cast_possible_truncation",
    "clippy::cast_possible_wrap",
    "clippy::cast_sign_loss",
    "clippy::float_cmp",
    "clippy::as_underscore",
    // TODO: "clippy::unwrap_used", // let’s either handle `None`, `Err` or use `expect` to give a reason
    "clippy::large_stack_frames",
    // == Style, readability == //
    "absolute_paths_not_starting_with_crate",
    "single_use_lifetimes",
    "unreachable_pub",
    "unused_lifetimes",
    "unused_qualifications",
    "keyword_idents",
    "noop_method_call",
    "clippy::semicolon_outside_block", // with semicolon-outside-block-ignore-multiline = true
    "clippy::clone_on_ref_ptr",
    "clippy::cloned_instead_of_copied",
    "clippy::trait_duplication_in_bounds",
    "clippy::type_repetition_in_bounds",
    "clippy::checked_conversions",
    "clippy::get_unwrap",
    // TODO: "clippy::similar_names", // reduce risk of confusing similar names together, and protects against typos when variable shadowing was intended
    "clippy::str_to_string",
    "clippy::string_to_string",
    // TODO: "clippy::std_instead_of_alloc",
    // TODO: "clippy::std_instead_of_core",
    "clippy::unreadable_literal",
    "clippy::separated_literal_suffix",
    "clippy::unused_self",
    // TODO: "clippy::use_self", // NOTE(@CBenoit): not sure about that one
    "clippy::useless_let_if_seq",
    // TODO: "clippy::partial_pub_fields",
    "clippy::string_add",
    "clippy::range_plus_one",
    // TODO: "missing_docs" // NOTE(@CBenoit): we probably want to ensure this in core tier crates only
    // == Compile-time / optimization == //
    "unused_crate_dependencies",
    "unused_macro_rules",
    "clippy::inline_always",
    "clippy::or_fun_call",
    "clippy::unnecessary_box_returns",
    // == Extra-pedantic clippy == //
    "clippy::collection_is_never_read",
    "clippy::copy_iterator",
    "clippy::expl_impl_clone_on_copy",
    "clippy::implicit_clone",
    "clippy::large_types_passed_by_value",
    "clippy::redundant_clone",
    "clippy::string_to_string",
    "clippy::alloc_instead_of_core",
    "clippy::empty_drop",
    "clippy::return_self_not_must_use",
    "clippy::wildcard_dependencies",
    // == Let’s not merge unintended eprint!/print! statements in libraries == //
    "clippy::print_stderr",
    "clippy::print_stdout",
    "clippy::dbg_macro",
];

pub fn fmt(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("FORMATTING");

    let output = cmd!(sh, "{CARGO} fmt --all -- --check").ignore_status().output()?;

    if !output.status.success() {
        anyhow::bail!("Bad formatting, please run 'cargo +stable fmt --all'");
    }

    println!("All good!");

    Ok(())
}

pub fn lints(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("LINTS");

    let cmd = cmd!(sh, "{CARGO} clippy --workspace --locked -- -D warnings");

    EXTRA_LINTS.iter().fold(cmd, |cmd, lint| cmd.args(["-W", lint])).run()?;

    println!("All good!");

    Ok(())
}

pub fn tests_compile(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("TESTS-COMPILE");
    cmd!(sh, "{CARGO} test --workspace --locked --no-run").run()?;
    println!("All good!");
    Ok(())
}

pub fn tests_run(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("TESTS-RUN");
    cmd!(sh, "{CARGO} test --workspace --locked").run()?;
    println!("All good!");
    Ok(())
}

use crate::prelude::*;

// TODO: when 1.74 is released use `[lints]`: https://doc.rust-lang.org/nightly/cargo/reference/unstable.html#lints
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
    "elided_lifetimes_in_paths", // https://quinedot.github.io/rust-learning/dont-hide.html
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

    // TODO: when 1.74 is released use `--keep-going`: https://doc.rust-lang.org/nightly/cargo/reference/unstable.html#keep-going
    let cmd = cmd!(sh, "{CARGO} clippy --workspace --locked -- -D warnings");

    EXTRA_LINTS.iter().fold(cmd, |cmd, lint| cmd.args(["-W", lint])).run()?;

    println!("All good!");

    Ok(())
}

pub fn typos(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("TYPOS-CLI");

    let typos_path = local_bin().join("typos");
    let typos_cli_installed = typos_path.exists() || typos_path.with_extension("exe").exists();
    if !typos_cli_installed {
        anyhow::bail!("`typos-cli` binary is missing (check::install step was skipped?)");
    }

    cmd!(sh, "{typos_path}").run()?;


    println!("All good!");

    Ok(())
}

pub fn install(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("TYPOS-CLI-INSTALL");

    if !is_installed(sh, "typos") {
        // Install in debug because it's faster to compile and we don't need execution speed anyway.
        // typos-cli version is pinned so we don’t get different versions without intervention.
        cmd!(
            sh,
            "{CARGO} install --debug --locked --root {LOCAL_CARGO_ROOT} typos-cli@{TYPOS_CLI_VERSION}"
        )
        .run()?;
    }

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

pub fn lock_files(sh: &Shell) -> anyhow::Result<()> {
    let _s = Section::new("CHECK-LOCKS");

    // Note that we can’t really use the --locked option of cargo, because to
    // run xtask, we need to compile it using cargo first, and thus the lock
    // files are already "refreshed" as far as cargo is concerned. Instead,
    // this task will check for modifications to the lock files using git-status
    // porcelain. The side benefit is that we can check for npm lock files too.

    const LOCK_FILES: &[&str] = &[
        "Cargo.lock",
        "fuzz/Cargo.lock",
        "web-client/iron-remote-gui/package-lock.json",
        "web-client/iron-svelte-client/package-lock.json",
    ];

    let output = cmd!(sh, "git status --porcelain --untracked-files=no")
        .args(LOCK_FILES)
        .read()?;

    if !output.is_empty() {
        cmd!(sh, "git status").run()?;
        anyhow::bail!("one or more lock files are changed, you should commit those");
    }

    println!("All good!");

    Ok(())
}

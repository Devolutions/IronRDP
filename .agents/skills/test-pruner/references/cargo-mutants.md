# cargo-mutants mechanics

cargo-mutants keeps per-test results only in its logs, so the kill matrix is parsed from them.
These notes target cargo-mutants 27.1.

## Run

Scope with `--file '<package-dir>/src/<module>/**'` for a module, `--re '<fn>|<fn>'` for a pull request's changed or critical functions, or `--in-diff <diff>` for its whole diff; each mutant costs an incremental build, so scope narrowly.
Count the mutants first by adding `--list` to the same scope, and estimate the run as mutants × (incremental build + clean scoped test time), divided by `--jobs` only without `--in-place`, which rejects it; narrow `--re` when it exceeds the budget.
A test-only diff yields no mutants under `--in-diff`, so scope with `--re` to the production functions the changed tests call, including unchanged ones.
Package directories vary; take the path from `cargo metadata` rather than assuming `crates/<name>`.

```sh
cargo mutants --file '<path>' --test-package <test-package> --in-place --baseline skip --timeout <s> --build-timeout <s> -- <cargo-test-args>
```

The baseline phase ignores `--test-package` and tests the mutated package, so it fails when the tests live in a separate test crate.
Run the scoped tests once by hand instead and keep the log, then skip the baseline and set both timeouts to a few times the measured test run and incremental build.
Arguments after `--` go to `cargo test`, and a second `--` reaches the test binary: `-- --test <target> -- <module>::` runs only that module's tests of one test target.
Confirm the clean scoped run executes tests before trusting any result; a filter that matches nothing passes silently and yields an empty kill matrix.
Scope every run to one test binary with `--test <name>` or `--lib`; the kill matrix keys tests by harness name, which can repeat across binaries.
Pass `--features <f>` for feature-gated code; discovery ignores `cfg` conditions, so mutants behind a disabled feature are still generated, compile away, and appear missed rather than absent, and a survivor there is a scoping error, not a coverage gap.
Pass `--output <scratch-dir>` so `mutants.out` stays out of the checkout.

The default mode copies the tree and cold-builds the workspace; `--in-place` reuses the warm `target/` and is much cheaper.
It mutates the worktree during the run, so read code with `git show HEAD:<path>` meanwhile, run no other cargo command against the same `target/`, and check `git status` afterwards.

Outputs under `mutants.out/`: `caught.txt`, `missed.txt`, `unviable.txt`, `outcomes.json`, `log/` with one log per mutant, and `diff/` with one patch per mutant.
Keep the same test scope and flags for every run; changing test scope makes mutants caught by other tests appear missed.
Within one run, key mutants by their full name from `outcomes.json`; stripping coordinates merges distinct mutants.
Across runs, coordinates shift whenever an edit above a function adds or removes lines, such as a test module in the middle of a file or a source comment, so match mutants by file, function, and replacement, and compare diff text with hunk headers removed when several share that key.
Do not use `--iterate`; it skips mutants caught in previous runs, the opposite of a rerun.
Read `unviable.txt`: when a function returns through a `Result` type alias, cargo-mutants cannot build its `Ok` replacement and drops it there without warning, so add that mutant by hand; for a check, "always `Ok`" is the mutant that matters most.
Patches in `mutants.out/diff/` carry the mutant description in their `+++` line; to apply one, rewrite that line to the path and use `git apply -p0`.

## Kill matrix

Each log holds the test harness output for its mutant, with one `ok` or `FAILED` line per test.
A test caught a mutant when its line reads `FAILED`, so parse the logs instead of rerunning tests.
Key the matrix on `outcomes.json`, which maps each mutant name to its log; log file names do not distinguish mutants at the same position.
A run is complete only when every test of the clean run has an `ok` or `FAILED` line; a hung test or an aborted process leaves lines missing while earlier `FAILED` lines look valid, so rerun such a mutant's tests one at a time.
Run from the directory that contains `mutants.out`:

```powershell
$expected = Select-String -Path <clean-run-log> -Pattern '^test (\S+)(?: - should panic)? \.\.\. (ok|FAILED)$' | ForEach-Object { $_.Matches[0].Groups[1].Value }
(Get-Content -Raw mutants.out/outcomes.json | ConvertFrom-Json).outcomes |
    Where-Object { $_.scenario.Mutant -and $_.summary -in 'CaughtMutant', 'Timeout' } |
    ForEach-Object {
        $mutant = $_.scenario.Mutant.name
        $results = Select-String -Path "mutants.out/$($_.log_path)" -Pattern '^test (\S+)(?: - should panic)? \.\.\. (ok|FAILED)$'
        $seen = $results | ForEach-Object { $_.Matches[0].Groups[1].Value }
        $failed = $results | Where-Object { $_.Matches[0].Groups[2].Value -eq 'FAILED' } | ForEach-Object { $_.Matches[0].Groups[1].Value }
        if ($_.summary -eq 'CaughtMutant' -and $failed -and -not (Compare-Object @($expected) @($seen))) { $failed | ForEach-Object { "$mutant`t$_" } }
        else { "$mutant`t<$($_.summary): incomplete per-test results, rerun tests individually>" }
    }
```

For assertion-level evidence, read the `panicked at <file>:<line>` location of each failure; an assertion whose line never appears across its test's kills catches nothing the others miss.

## Rerun

Run the scoped tests on the clean tree first; with `--baseline skip`, a suite broken by a test edit marks every mutant caught.
Rerun only the mutants the removed or changed tests caught: apply their patches from `mutants.out/diff/`, or narrow `--re` to their functions.
Any mutant that moved from caught to missed without an approved deletion explaining it means revert.

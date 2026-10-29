# Stryker.NET mechanics

Stryker.NET lists the tests that killed each mutant in its JSON report, but only when bail is disabled, so the kill matrix is read from the report.
These notes target Stryker.NET 5.0.0 with xUnit v2 on the VsTest runner.

## Install

In the solution directory, `dotnet new tool-manifest` then `dotnet tool install dotnet-stryker --version 5.0.0`; the manifest it writes is a repository file, not output, so leave it untracked unless the pull request adopts it.

## Run

Run the clean suite first with a TRX log, because the report names tests by id and the TRX carries the id with the display name:

```powershell
dotnet test <TestProject>.csproj --logger "trx;LogFileName=clean.trx" --results-directory <scratch-dir>/trx
```

Confirm the TRX lists the tests you expect before trusting any result, because `dotnet test` passes when it discovers none, as with a wrong project or filter.

Then, from the test project directory:

```powershell
dotnet stryker --mutate "**/<Module>/*.cs" --disable-bail --break-on-initial-test-failure --reporter json --reporter cleartext --output <scratch-dir> --skip-version-check
```

`--mutate` takes several globs, `!<glob>` excludes, and `<file>.cs{<start>..<end>}` limits to a character span; Stryker mutates the whole project in memory before filtering, so compile-error warnings about files outside the scope are noise.
`--disable-bail` runs every covering test to completion; by default Stryker aborts a mutant's run at the first failure, and `killedBy` then holds only the tests that had failed by then, a partial list that makes kills look unique.
`--break-on-initial-test-failure` stops the run when the clean suite fails, which a test edit can cause, because a matrix built over failing tests proves nothing.
For a change's diff, `--since:<branch, tag, or full SHA>` selects its mutants; a revision expression such as `HEAD~1` or an abbreviated SHA is rejected.
A test-only diff still yields mutants under `--since`: every mutant covered by a test in a changed test file counts as changed.
Pass `--project <Name>.csproj` only when the test project references more than one project, and `--solution <path>.sln` when it targets .NET Framework, where Stryker requires it.
Coverage analysis, `perTest` by default, runs only the tests covering each mutant, so the matrix spans the whole test project without scoping tests and a unique kill is unique across the project; restricting to a test class needs `test-case-filter` in `stryker-config.json`, since the 5.0.0 command line rejects it as a flag.
Stryker builds the test project itself and writes a `.gitignore` of `*` into `--output`, so the output stays untracked wherever it is.
Build and the initial run take a few minutes before the first mutant, and a whole library of a few thousand lines can run for most of an hour, so `--mutate` a module, a file, or a span.
Stryker logs how many mutants it will test before the first one runs; estimate the run from that count, the clean TRX run's duration, and `--concurrency`, and stop it if the estimate exceeds the budget.
Timeouts are computed per mutant from the initial run, and `additional-timeout` and `timeout-ratio` are config-file options.

## Kill matrix

`<scratch-dir>/reports/mutation-report.json` holds `files[<path>].mutants[]` with `id`, `mutatorName`, `replacement`, `location`, `status` (`Killed`, `Survived`, `NoCoverage`, `Timeout`, `CompileError`, `Ignored`), and `coveredBy` and `killedBy` lists of test ids, and `testFiles[<path>].tests[]` with `id` and `name`.
`name` is the method's qualified name without arguments, so every row of a `[Theory]` shares one name; `id` is the VsTest test case id, which the TRX carries as `UnitTest/@id` with the display name and arguments.
Key on the id throughout: display names collide when xUnit truncates long string arguments.

```powershell
$report = Get-Content -Raw <scratch-dir>/reports/mutation-report.json | ConvertFrom-Json
[xml]$trx = Get-Content -Raw <scratch-dir>/trx/clean.trx
$name = @{}
foreach ($t in $trx.TestRun.TestDefinitions.UnitTest) { $name[$t.id] = $t.name }
$report.files.PSObject.Properties | Where-Object Name -like '*<Module>*' | ForEach-Object {
    foreach ($m in $_.Value.mutants | Where-Object status -eq 'Killed') {
        foreach ($id in $m.killedBy) { "$($m.id)`tL$($m.location.start.line) $($m.mutatorName): $($m.replacement)`t$id`t$($name[$id])" }
    }
}
```

Read kills only from `killedBy`; `coveredBy` lists the tests that ran against a mutant, not those that failed.
The script lists `Killed` mutants only; attribute a `Timeout` by applying it by hand, as below, and running its `coveredBy` tests one at a time before counting it caught.
One row with an argument xUnit cannot serialize makes the whole theory one test id, so the matrix cannot tell its rows apart; `test-writer`'s C# forms avoid this.
Stryker keeps no per-test output, so for assertion-level evidence apply the mutant by hand from `location` and `replacement`, run the killing test with `dotnet test --filter "FullyQualifiedName~<Class>.<Method>"`, and read the failing assertion from the stack trace.
Mutant ids are assigned in order over the whole project, so an edit anywhere can renumber them, and lines shift with edits above; across runs match mutants by file, `mutatorName`, `replacement`, and the text of the mutated line in `files[<path>].source`, and mutants sharing all four by their order in the file, which test and comment edits leave unchanged.

## Rerun

Refresh the clean run and its TRX before each rerun.
Rerun only the mutants the removed or changed tests caught: apply one or two by hand as above, or narrow `--mutate` to the spans that hold them.
For a change's diff, measure and rerun with `--mutate` over the files or spans a `--since` run mutated, never comparing `--since` runs: a test edit changes which mutants `--since` selects, so a lost kill drops out instead of surviving.
Do not use `--with-baseline`; it reuses results for mutants it considers unchanged, the opposite of a rerun.
Identical settings include `mutation-level` and any `test-case-filter`, since either changes the mutant population or the tests that can kill it.

## Pitfalls

Verify a surprising status by applying the mutant by hand: Stryker can report as survived a mutant in a static initializer, whose value is computed before the mutant switches on, or a replacement that would not compile on its own, and a machine at full load turns slow kills into timeouts.

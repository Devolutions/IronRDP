# C# forms

The repository's C# tests are xUnit v2; each rule in `SKILL.md` takes these forms there.

Properties use CsCheck: compose `Gen<T>` with LINQ and `Sample` it; each type's generator lives in one static class of the test project.
CsCheck rather than FsCheck, because its generators shrink without per-type shrinkers, so a composed generator costs one declaration, and it adds no FSharp.Core to a C# test project.
Releases after CsCheck 2.14.1 need .NET 6 or later, so a .NET Framework test project pins 2.14.1, the last on .NET Standard 2.0.
A failing `Sample` prints a seed; replay it with `seed:` or the `CsCheck_Seed` environment variable, and pin the failure by keeping the shrunk value in a regression test rather than the seed, since a generator change moves the seed's meaning.

Snapshots are written by hand: the expected value inline or, past a screen, in a sibling file copied into the test output.
Nothing rewrites a stale value, so edit each by hand after reading it.

Cases are `[Theory]` with `[InlineData]` rows for literal data, whose display name carries the arguments, so a failure names the row.
Other data goes through `[MemberData(nameof(<Member>))]` whose rows hold only values xUnit can serialize, such as a string key the test resolves to its data, because one row it cannot serialize turns the whole theory into one test case that no filter or mutation report can split.
A cross product is a `[MemberData]` member that yields it.
A regression test is a separate `[Fact]`.

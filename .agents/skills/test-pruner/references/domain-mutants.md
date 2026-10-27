# Domain mutants

## Apply

Neither mutation tool takes user-defined operators, so write each domain mutant as a patch.
Take it, or rebuild a challenger's patch, with `git diff --output=<file>` from an edit you then revert, since a hand-written hunk header is easily wrong and a Windows PowerShell redirect re-encodes the patch as UTF-16 with CRLF, which `git apply` rejects.
`git apply` it, run the scoped tests, and `git apply -R` it; caught means a test failed, and a patch that does not build proves nothing until it does.
Patches are pull request evidence, not repository files.

## Catalog

Codec catalog:

- Swap endianness on one field read or write.
- Length prefix off by one in either direction.
- Remove one bounds check or remaining-length test.
- Wrong padding count, or padding read but not skipped; accept non-zero pad bits.
- Swap two adjacent fields of the same type.
- Flip one bit in a flags field.
- Write one byte fewer than the size computation reports.
- Return success (`Ok`, `true`) on a malformed-input branch that returns an error or throws.
- Accept a reserved or out-of-range enum value.

Async and resource catalog:

- Remove or shorten a deadline.
- Detach a task instead of aborting it, or hold a handle that neither aborts on drop nor cancels on dispose where one is required.
- Return early before a drain or flush.
- Swallow an error as success.
- Skip recording a state flag.
- Swap two status or close codes.

Decision and validation catalog, for each security or policy decision: remove it, weaken it, key it on the wrong identity, or apply it to the wrong object.

- Always return success from the check.
- Accept any candidate by dropping a filter.
- Look up by the wrong key, such as a name hash instead of a thumbprint.
- Stop after the first candidate.
- Ignore one input source, such as caller-provided roots.
- Skip verification of a candidate's chain.
- Swap error precedence or report the wrong error.

# Two-lens critique

Run both lenses on a complete draft before freezing, ideally as an independent reviewer that has not seen the design conversation.
Report with severities and section references; the author adjudicates each finding in writing.

## Lens A: overkill

For each mechanism ask what breaks without it: two modes or encodings that could be one, a tombstone or knob serving a case nobody has, a configurable value that could be fixed, an admin or debug surface only one product or only tests need.
Keep what has a named failure; cut the rest, or move it to the intent or document of the party that owns it when it is that party's own realisation.

## Lens B: gaps

Put every state-changing operation through each question.

1. **Lost response.** The request succeeded but the reply never arrived; can the client retry with no side effects and recover the result?
2. **Deleted in between.** The object the retry references was deleted or exhausted meanwhile; does the retry still recover, and what stops the deleted thing from doing more than that?
3. **Concurrency.** Two identical requests race; does exactly one win, is the loser handed the winner's result, and is uniqueness enforced by the store rather than by a check?
4. **Expiry mid-flow.** A token, certificate, or grace window expires between steps; does the flow strand a recoverable state?
5. **Trusting the client's declaration.** Does the verifier accept the client's own list of what it covered, or require exactly what the profile mandates?
6. **Make-before-break.** Does a rotation or renewal close the old path before the new one is proven to work?
7. **Order versus precedence.** Does the text fix an evaluation order where it only needs to fix which outcome wins, or leave two outcomes equally valid for one input?
8. **Bounds.** Are sizes, counts, and nesting capped, and does an aggregate cap agree with the per-item caps?
9. **Malformed error bodies.** What does the client do when the error response itself is wrong?
10. **Second copy.** Is any list, mapping, or encoding defined in two places?

## For each new rule

- Was this written to fit one implementation?
- Would a reader with only this document and the cited standards produce the same bytes?
- What test fails if an implementation ignores it?

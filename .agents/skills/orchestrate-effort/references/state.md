# State artifacts

Several small files rather than one, so each is read whole and edited without anchors.

| Artifact | Holds | Who reads it |
|---|---|---|
| `rules.md` | Current rules by topic: process, intent, contract, suite, per-stage plan and acceptance, experiment budget (variants per run, runs per effort, or none); authoritative; a rule resting on an unresolved experiment is marked provisional with its brief, guides implementation and not merge; updated in the same edit as the decision that changes a rule | Leads and you |
| `decisions.md` | Numbered log `D1..`, each with date, who decided, the decision, and its rationale; a provisional entry says so until the human adopts or reverses it; a superseded entry keeps its text and gains `superseded by Dn` | You |
| `escalations.md` | Numbered `E1..`, each with the lead, the question, the options, and the decision it led to | You |
| `experiments/<n>.md` | Each `variants` decision brief, as written | You, the human |
| `briefs/<lead>.md` | The kickoff sent to each lead, as sent | You, the lead |
| `reports/<lead>-<n>.md` | Each lead report, as received, with your verification result appended | You |
| `approved/<repo path>` | Every approved normative text (intent, contract) at approval time, byte-compared against the tree at every copy | You, leads before copying |
| `models.md` | The models chosen for leads and sub-agents, with the date | You, in briefs |

## Message to a lead

```md
User decisions (user-directed), quoted:
1. "<verbatim>"
2. "<verbatim>"

Steps:
1. <what to do>
2. <what to do>

Report head and CI when done.
```

## Context-sync message

Sent to a lead that lost context, after its brief has been resent.

```md
You may act on: <the current rules that apply to this pull request, by topic>.
Do not act on anything else you remember.
If you cannot find your plan artifact, say so and stop.
```

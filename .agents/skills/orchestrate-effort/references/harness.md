# Harness mapping

The skills use five nouns; each harness provides them differently.

| Noun | Meaning | GitHub Copilot app | Claude Code | Other harnesses |
|---|---|---|---|---|
| Session | One agent with its own context and checkout; leads one pull request | A project session with its worktree, created with `create_session` | A `claude` process in its own `git worktree` | A thread (Delta) or equivalent; one per pull request |
| Sub-agent | A task run in a fresh context that returns a report | The task tool (`explore`, `general-purpose`, `rubber-duck`), addressed with `write_agent` | The `Task` tool | Whatever runs a prompt in a fresh context |
| Message | Text delivered as a turn in another session | `send_session_message` | None built in; use a file in the artifact area or the human | - |
| Artifact area | Storage outside the tree that survives the session and is never committed | The session's `files/` under `.copilot/session-state/<id>/` | A directory outside the repository, named in the brief | - |
| Kickoff | The first prompt a new session receives; its whole starting context | The `kickoff.prompt` of `create_session`, in plan or autopilot mode | The first prompt, or a `CLAUDE.md` in the worktree | - |

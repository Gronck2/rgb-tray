# 0001. Use the dev-pipeline plugin

- Status: accepted
- Date: 2026-11-21

## Context

Work on RGB Tray is done with Claude Code. Non-trivial changes need a repeatable flow: a recorded task and plan,
implementation, tests, review and a quality gate before a pull request, with the project's own rules (CI gates,
device safety, settings compatibility) applied every time.

## Decision

Non-trivial work goes through the `dev-pipeline` plugin (manager -> coder -> tester || code-reviewer -> qa-mr).
Everything specific to this project lives in the `## Pipeline contract` section of `CLAUDE.md` and in `docs/`.
Branch model: `main` only, it is both the base and the PR target; branches are `task/NNNN-slug` or `docs/<slug>`.
Pull requests are merged only by the owner, as merge commits, and only with green CI (required check `build`).
`ROADMAP.md` stays the owner's task list; `docs/tasks/` holds only the pipeline's task files.

## Consequences

- The plugin must be installed and enabled on the machine; sessions load its agents at start, so open sessions
  need a restart (or `/reload-plugins`) after it changes.
- No same-named agents in `.claude/agents/` unless a deliberate override is needed.
- The gates are the CI commands; they are run before every commit.

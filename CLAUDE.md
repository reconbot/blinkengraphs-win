# blinkengraphs

Project memory lives in the repo: decisions `docs/adr/`, plan `docs/PLAN.md`, research `docs/research/`. Record learnings there.

The spec repo, reconbot/blinkengraphs, authors the vocabulary, rules, vectors, fixtures, tokens, defaults and icon; this repo implements it. It is checked out at `spec/` as a submodule on a release tag. A rule this app needs starts as a spec PR, unless it is a departure for this app's normal use ([spec ADR 0002](https://github.com/reconbot/blinkengraphs/blob/main/docs/adr/0002-platforms-may-depart-for-their-normal-use.md)): then it is an ADR here naming the rule it replaces, and the conformance tests skip any vector it voids, by name. `cargo test -p conformance` runs every spec vector against this code; bump the submodule in a PR when the spec releases (spec `docs/sync.md`).

Defaults: `config::defaults` embeds `spec/defaults.json` and merges `defaults.override.json` over it (spec `rules/defaults.md`). Override a default there, with the reason in an ADR.

Skills (`mattpocock-skills`): `domain-modeling` when a term or decision changes, `codebase-design` when shaping a module interface, `tdd` for features and fixes, `writing-for-agents` for any doc or CLAUDE.md edit.

Comments carry only what code can't: the why, the Win32 gotcha, the source URL. Prose earns each sentence.

Commits follow Conventional Commits; release-please builds the changelog from them.

Commits, PRs and comments stay free of session links and personal or employer details; the repo is public.

Never force-push `main` once the repo has a release.

Release PRs (release-please) merge only after the owner has reviewed them and said so. Feature PRs merge when tests pass.

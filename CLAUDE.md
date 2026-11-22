# blinkengraphs

Project memory lives in the repo: decisions `docs/adr/`, plan `docs/PLAN.md`, research `docs/research/`. Record learnings there.

Vocabulary, Verdict rules, test vectors, design tokens and shared defaults live in the spec repo, reconbot/blinkengraphs, checked out at `spec/` as a submodule on a release tag. Rule changes start there; regenerate its `vectors/`, `fixtures/` and `tokens/` with `cargo run -p vectors -- <spec checkout>`. CI regenerates against the submodule and expects no diff; bump the submodule in a PR when the spec releases (spec `docs/sync.md`).

Defaults: `config::defaults` embeds `spec/defaults.json` and merges `defaults.override.json` over it (spec `rules/defaults.md`). Override a default there, with the reason in an ADR.

Skills (`mattpocock-skills`): `domain-modeling` when a term or decision changes, `codebase-design` when shaping a module interface, `tdd` for features and fixes, `writing-for-agents` for any doc or CLAUDE.md edit.

Comments carry only what code can't: the why, the Win32 gotcha, the source URL. Prose earns each sentence.

Commits follow Conventional Commits; release-please builds the changelog from them.

Commits, PRs and comments stay free of session links and personal or employer details; the repo is public.

Never force-push `main` once the repo has a release.

Release PRs (release-please) merge only after the owner has reviewed them and said so. Feature PRs merge when tests pass.

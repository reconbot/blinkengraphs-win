# blinkengraphs

Project memory lives in the repo: decisions `docs/adr/`, plan `docs/PLAN.md`, research `docs/research/`. Record learnings there.

Vocabulary, Verdict rules, test vectors and design tokens live in the spec repo, reconbot/blinkengraphs. Rule changes start there; regenerate its `vectors/` and `fixtures/` with `cargo run -p vectors -- <spec checkout>`.

Skills (`mattpocock-skills`): `domain-modeling` when a term or decision changes, `codebase-design` when shaping a module interface, `tdd` for features and fixes, `writing-for-agents` for any doc or CLAUDE.md edit.

Comments carry only what code can't: the why, the Win32 gotcha, the source URL. Prose earns each sentence.

Commits follow Conventional Commits; release-please builds the changelog from them.

Commits, PRs and comments stay free of session links and personal or employer details; the repo is public.

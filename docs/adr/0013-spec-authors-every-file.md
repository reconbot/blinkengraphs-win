# The spec authors every file; this repo passes its vectors

Until spec v1.6.0 this repo generated the spec's `vectors/`, `fixtures/` and `tokens/`, and CI checked conformance by regenerating them and expecting no diff. The spec now authors all of them, so a rule change lands there first, prose and vectors together, and every platform implements it (spec `CLAUDE.md`, issue #25).

- `tools/vectors` became `tools/conformance`: tests that read each spec vector's input and assert this code gives its `expected`. The spec job runs `cargo test -p conformance`.
- Values the sketch vectors carry as exact bits compare by bits. Their decimal `value` is a label, and serde_json parses it up to an ULP off.
- Our fixtures and dark palette are tested equal to the spec's scenarios and base tokens until they are read from the spec directly.
- The spec job now fails when `spec/` is not on a release tag; before, `echo "$(git describe …)"` swallowed the error.

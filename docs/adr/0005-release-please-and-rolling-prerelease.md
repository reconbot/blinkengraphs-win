# Versioned releases via release-please; every push to main updates a rolling prerelease

Tagged releases are cut by release-please from Conventional Commits. Every push to `main` also rebuilds and overwrites a `main-latest` prerelease. Public download links work without a GitHub login and don't expire, which Actions artifacts don't offer. Binaries get GitHub build-provenance attestations (`actions/attest`), which `gh attestation verify` can check.

Attestations are not Authenticode, so SmartScreen will still show "unknown publisher". Authenticode signing via SignPath Foundation (free for open source) is deferred until the app is shared beyond its author.

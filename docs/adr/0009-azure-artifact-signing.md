# Sign releases with Azure Artifact Signing under the owner's identity

Browsers block unsigned downloads on SmartScreen reputation. No certificate removes the first-download warning, but a signature lets reputation accrue to a validated publisher rather than to each new exe ([research 05](../research/05-signing-and-installer.md)).

Azure Artifact Signing (formerly Trusted Signing), Basic tier, with individual identity validation. The publisher shown is the owner's legal name. The release workflow signs the exes and the MSI with the official GitHub Action, authenticating through OIDC federated credentials, so no signing secret lives in GitHub.

## Considered Options

- **SignPath Foundation.** Free, but the publisher shown is "SignPath Foundation", each release needs manual approval, and acceptance of a young project is uncertain.
- **OV certificate in a cloud HSM.** Costs more and needs more key management for the same SmartScreen effect. EV stopped bypassing SmartScreen in 2024.
- **Microsoft Store.** The only route without a first-download warning, but Store policy puts the Taskbar Strip at risk (ADR 0010).

## Consequences

The certificates live 72 hours and are reissued continually; reputation is tied to the validated identity. Reports of reputation resetting when Microsoft rotated intermediates in 2026 are low confidence.

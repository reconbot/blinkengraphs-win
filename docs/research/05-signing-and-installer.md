# 05 — Code signing and installer format for M3

Researched 2026-11-21. Two coupled questions:
1. Which code signing fixes or minimises the browser block on the downloaded exe?
2. Which installer format serves Intune, per-user installs without admin, and autostart, and leaves room for the M4 LocalSystem service?

Constraints:
- [ADR 0002](../adr/0002-no-admin-by-default.md): never require admin.
- [ADR 0005](../adr/0005-release-please-and-rolling-prerelease.md): release-please releases, a rolling `main-latest` prerelease, build attestations, SignPath deferred.
- [ADR 0008](../adr/0008-elevated-extras-via-companion-service.md): the M4 service ships as a separate per-machine MSI.
- [ADR 0001](../adr/0001-tray-icons-supported-strip-unsupported.md): the Taskbar Strip re-parents into `Shell_TrayWnd`, an undocumented technique.
- Windows 11 x64 and ARM64, one ~6 MB exe, public repo, MIT OR Apache-2.0.
- The owner runs Entra and Intune. Cost of about $10/month is no objection (owner, 2026-11-21).

Citations such as [S1] point to the Sources list. Confidence markers, as in 02 and 04:
- **(verified)**: read in a primary source.
- **(inferred)**: my reasoning from the sources.
- **(low confidence)**: weak, dated, community-only or contradictory evidence.

---

## 0. What blocked the download

- "Isn't commonly downloaded" is the wording of two different systems:
  - **Edge SmartScreen** checks a download against "a list of well-known and popular downloads by Microsoft Edge users" and "warns you if your download is not on this list" [S20] (verified).
  - **Chrome Safe Browsing** has its own block categories ("Dangerous", "Suspicious", "Unverified", "Insecure") [S21] (verified).
- Everything in §1 is about Microsoft SmartScreen. Authenticode signing feeds SmartScreen's publisher signal [S1]. Whether it moves Chrome's verdict is not documented anywhere I found **(low confidence)**.
- **Open question for the owner:** which browser showed the block? If it was Chrome, test Chrome separately after signing.
- **Intune-deployed installs skip this check.** SmartScreen runs on files "marked as originating from the Internet" [S19]. Lawrence (ex-Microsoft, SmartScreen) notes that "most internal software deployment systems do not result in SmartScreen checks" [S19] (verified as his statement). An Intune delivery therefore needs signing for trust and policy, not for download reputation (inferred).

## 1. Q1 — Code signing

### 1.1 How SmartScreen reputation works in 2026

- **Two signals:** publisher reputation (is the file signed, and is the certificate's publisher known?) and file-hash reputation [S1] (verified).
- **Unsigned files** start at zero on every release: "Reputation cannot transfer from previous versions unless both were signed using the same publisher identity" [S1] (verified).
- **EV no longer bypasses SmartScreen.** The Trusted Root Program dropped EV-specific OIDs in 2024. "No certificate type (OV or EV) grants an immediate bypass" [S3]. EV is "Same as OV since 2024" [S2] (verified).
- **No published threshold.** Reputation "can take several weeks and hundreds of clean installs from a wide audience" [S1] (verified). Lawrence: "Microsoft has not documented any thresholds" [S19].
- **Consumer endpoints have no manual route.** "There is no need (or mechanism) to manually submit a file for SmartScreen reputation review for consumer endpoints" [S1] (verified).
- **Microsoft Store distribution** is the only documented way to show no warning on first download [S1][S2] (verified). See §1.5.
- **Smart App Control** on Windows 11 "will block execution of unsigned files unless the file has a positive reputation". It applies to all executables, not only downloads [S1] (verified). Signing is the only lever here too.

### 1.2 Azure Artifact Signing (formerly Trusted Signing)

| Aspect | Finding |
|---|---|
| Name and status | Renamed from Trusted Signing to **Artifact Signing** and generally available. The announcement is dated 2026-01-12 [S10] (verified). Docs, CLI (`az artifact-signing`) and the action all use the new name [S4][S9]. The retail price API still lists the meters as "Trusted Signing" [S8] |
| Eligibility, Public Trust | **Organizations:** USA, Canada, EU, UK, Australia, New Zealand, Japan, South Korea, Singapore, Switzerland, Norway and Israel. **Individuals:** USA or Canada only [S4] (verified, page updated 2026-11-20). An older page still lists only US, CA, EU and UK for organizations [S2], so the list is growing |
| Validation | Organizations: legal entity, business identifier, a non-shared mailbox on the entity's domain, and ID verification of a named person. Takes 1–30 business days [S4]. Individuals: government ID through Microsoft Entra Verified ID and AU10TIX. The Azure billing account must be of type Individual, and its name and address must match the ID [S4] (verified). Free, trial and sponsored subscriptions are not accepted [S5] |
| Publisher shown | Fixed to the validated legal name, with no custom CN or O [S5]. For an individual, the city, state or province, and country also appear on the certificate [S4] (verified). The owner's legal name becomes the public publisher, and an employer-validated identity would put the employer's name on a personal public repo (inferred; CLAUDE.md keeps employer details out) |
| Price | Basic $9.99/month for 5,000 signatures. Premium $99.99/month for 100,000. Overage $0.005 per signature [S8] (verified via the Azure retail price API; the pricing page renders placeholders). Billing is not pro-rated [S5] |
| Certificate lifetime | "Renewed daily and are valid for only 72 hours". Timestamping (`http://timestamp.acs.microsoft.com`) keeps signatures valid afterwards [S6][S7] (verified) |
| Durable identity | Each certificate profile has a custom EKU `1.3.6.1.4.1.311.97.…` that stays constant across renewals. It can pin trust in App Control for Business (WDAC) policies [S6] (verified). For an Intune fleet this beats pinning a 72-hour thumbprint (inferred) |
| SmartScreen | "Reputation builds over time; initial warnings expected" [S2]. "Signing consecutive releases with a consistent publisher/signing identity lets publisher reputation build" [S2] (verified). A Microsoft moderator says reputation attaches "to the validated identity and persist[s] across the cert rotations" [S11] **(low confidence: forum answer)** |
| Known 2026 regression | New intermediate CAs (EOC/AOC CA 03, CA 04), introduced in March 2026, started without reputation. Identically signed builds then warned [S12][S13]. A maintainer reported a fix on 2026-04-10. Reports recurred in October 2026; Support called it a bug and on 2026-11-19 said it "has now been addressed" [S12] **(low confidence: issue thread and relayed Support statements)**. Microsoft documents no way to pin an intermediate CA and no propagation timeline [S13] |
| Root programme | Artifact Signing CAs "are scheduled for removal from the Common CA Database (CCADB)". Microsoft "no longer includes these CAs in the Trusted Root Program, they remain on the Windows Platform Trust List", so Windows keeps trusting them [S6] (verified). See §1.5 for the Store consequence |
| GitHub Actions | Official `azure/artifact-signing-action@v2`. It recommends OIDC through `azure/login` (`id-token: write`, already granted in `release.yml`). It runs on Windows runners only (windows-2025, windows-2022), not on Windows Arm runners [S9] (verified). The identity needs the "Artifact Signing Certificate Profile Signer" role [S9] |
| ARM64 | SignTool signs "all file types that SignTool supports" [S5]. Signing ARM64 PE files from the x64 runner works because Authenticode is architecture-neutral **(inferred; spike it)** |
| MSI / MSIX | Any SignTool type, MSI and MSIX included [S5]. For MSIX, the manifest `Publisher` must equal the certificate subject, or signing fails with 0x8007000b [S5] (verified) |

### 1.3 SignPath Foundation (free OSS certificates)

| Aspect | Finding |
|---|---|
| Requirements | OSI licence with no commercial dual-licensing; no proprietary components; "actively maintained"; "already be released in the form that should be signed"; functionality documented on the download page; MFA for every team member; defined Authors, Reviewers and Approvers; a "Code signing policy" section on the home page and release pages that names SignPath and includes a privacy statement [S14] (verified). MIT OR Apache-2.0 qualifies (inferred) |
| Review | SignPath checks project "reputation and control" once. "For executable programs … we require a certain verifiable reputation". No arbitration exists, and SignPath is "under no obligation to accept your project" [S14] (verified). A young project with few users may be refused (inferred) |
| Policy risk | "No hacking tools" bars "security diagnosis tools that actively scan … e.g. by identifying unprotected network ports" [S14] (verified). ICMP, TCP and UDP Hop Traces are reachability diagnostics, not vulnerability scanning, so they probably pass. Raw sockets in M4 could draw questions **(low confidence)** |
| Publisher shown | "The code signing certificate is issued to SignPath Foundation". The publisher is SignPath Foundation, not the project or the owner [S14] (verified). Reputation is shared with every other Foundation-signed project. Whether that helps a new project's first downloads is undocumented **(low confidence)** |
| Per-release approval | "Every release needs manual approval for signing" [S14] (verified). That cannot cover a signed `main-latest` on every push. Only tagged releases would be signed (inferred) |
| GitHub Actions | `signpath/github-action-submit-signing-request@v3`. The input artifact goes through `actions/upload-artifact` v4+. Origin metadata comes from GitHub, and OSS projects must build on GitHub-hosted runners [S16] (verified) |
| Formats | PE, MSI, MSIX and APPX, with nested ("deep") signing of PE files inside an MSI [S17] (verified) |
| Cost | Free [S15] |
| SmartScreen | Not addressed by SignPath. Microsoft's general rules apply: a consistent publisher builds reputation [S1] |

### 1.4 Traditional OV or EV certificate

- **Cost and lifetime:** OV costs $150–300/year; EV costs $400+/year and is "no longer recommended specifically for SmartScreen bypass" [S2] (verified).
- **Key storage:** the private key must live on an HSM or token, so CI needs a cloud HSM product [S2] (verified).
- **Validity cap:** certificates issued from 2026-03-01 last at most 460 days (CA/B Forum ballot CSC-31) [S18] (verified as adopted; secondary sources give the date).
- **Renewal may reset reputation.** One ISV renewed an EV certificate with the same subject and CA, and SmartScreen reputation "did not carry over" [S11] **(low confidence: one forum report)**.
- **SmartScreen:** "functionally equivalent to Azure Artifact Signing" [S2] (verified). It is the right option only when the owner is ineligible for Artifact Signing.

### 1.5 Do the Store or winget change the picture?

- **Store, MSIX:** Microsoft re-signs the package, and users "never see a SmartScreen warning" [S1][S2] (verified). The blockers are MSIX itself (§2.2) and Store policy:
  - **Policy 10.2.8** requires "supported methods" to "modify the user's Windows experience", and names "undocumented or unsupported APIs in unsupported ways" as unsupported [S24] (verified). The Taskbar Strip's re-parenting into `Shell_TrayWnd` is undocumented (ADR 0001), so a Store build should probably ship tray-only (inferred).
  - **Policy 10.2.4:** dependence on non-Microsoft NT services "is not allowed" except case by case [S24] (verified). M4's service does not fit the Store.
- **Store, MSI/EXE (policy 10.2.9):** "No SmartScreen prompts during Store install" [S2]. The publisher hosts the installer, which must be silent, standalone, behind a versioned URL, and signed with a certificate that "chains up to … a CA that is part of the Microsoft Trusted Root Program" [S23][S24] (verified).
  - **Open question:** Artifact Signing CAs are leaving the Trusted Root Program and remain only on the Windows Platform Trust List [S6]. Whether Partner Center still accepts them for 10.2.9 is not documented **(low confidence; ask Partner Center support before relying on it)**.
- **Store apps in Intune:** the "Microsoft Store app (new)" type keeps apps updated, but "any app that has an ARM64 installer isn't supported" [S28] (verified). That cuts out the ARM64 build.
- **winget:**
  - The community repo's validation flags installer URLs with bad SmartScreen reputation [S40] **(low confidence: a fork's copy of winget-pkgs Troubleshoot.md)**.
  - MSIX packages must be signed to enter the community repo [S38] (verified).
  - winget writes the Mark of the Web on downloads [S39], so a winget install is not a clean way around SmartScreen **(low confidence on whether it then triggers a prompt)**.
  - winget does not grant reputation. It is a channel, not a fix (inferred).

### 1.6 WDSI submission and build attestations

- **WDSI file submission:** the "Software developer" persona is for "software providers wanting to validate detection of their products". Microsoft Defender SmartScreen is a selectable product [S22] (verified).
  - Microsoft documents it as a route only for enterprise admins and as a fallback when prompts persist [S1][S5].
  - Lawrence suggests the developer flow "to make the Defender Cloud aware of your new certificate" [S19].
  - In November 2026, Microsoft Support suggested it per release as a workaround for the Artifact Signing regression [S12] **(low confidence)**.
  - Verdict: cheap and optional, with no documented effect on download reputation.
- **GitHub build attestations:** no Microsoft source lists any SmartScreen signal beyond file hash and Authenticode publisher [S1][S3]. Attestations are Sigstore provenance, not Authenticode, so they do nothing for SmartScreen (inferred, consistent with ADR 0005). Keep them for `gh attestation verify` and supply-chain review.

### 1.7 Q1 comparison

| | Artifact Signing | SignPath Foundation | OV certificate |
|---|---|---|---|
| Eligibility | Individual: US or CA. Organization: listed countries [S4] | Accepted OSS projects with existing reputation [S14] | Worldwide [S2] |
| Publisher shown | Owner's legal name and location [S4] | "SignPath Foundation" [S14] | Owner or entity |
| First-download SmartScreen | Warning until reputation builds [S2] | Same rules [S1]; shared-publisher effect unknown | Warning until reputation builds [S2] |
| Reputation continuity | Validated identity, across 72-hour certificates [S11]; broke on CA rotations in 2026 [S12] | Shared Foundation certificate | Per certificate; may reset at 460-day renewal [S11][S18] |
| GitHub fit | Official action with OIDC; Windows x64 runner [S9] | Official action; manual approval per release [S14][S16] | Cloud HSM plus SignTool; secrets to manage |
| Signs every `main` push | Yes (5,000/month) [S8] | No | Yes |
| Intune and App Control | Durable EKU for WDAC pinning [S6] | Standard certificate | Standard certificate |
| MSIX publisher | Owner's validated subject | `SignPath Foundation` subject | Owner's subject |

## 2. Q2 — Installer format

### 2.1 What Intune offers

- **Win32 app (`.intunewin`):**
  - **Install behavior:** System or User. "User context refers to only a particular user" [S26] (verified).
  - **User-targeted installs** fail "if the Win32 app requires device admin privileges … the standard user … doesn't have" [S26] (verified).
  - **Detection:** MSI product code (with an optional version check), file, registry or script [S26].
  - **Updates:** supersedence (update or replace), up to 10 nodes [S26].
  - **Platforms:** x64 and ARM64 supported [S25].
  - Installs must be silent [S26].
- **Line-of-business app:** accepts `.msi`, `.msix` and `.msixbundle`.
  - "For dual mode apps, select the desired context" [S27] (verified).
  - It takes one command-line argument; beyond that, use Win32 [S27].
  - Don't mix LOB and Win32 apps during Autopilot ESP [S25].
- **Microsoft Store app (new):** Store catalog only, with ARM64 installers unsupported [S28] (verified).
- **Enterprise App Catalog:** curated by Microsoft and part of the Intune Suite add-on [S29] (verified). A vendor cannot self-publish into it.
- **Community winget:** no native Intune app type deploys it; it needs a Win32 wrapper (inferred from [S28][S29]).

### 2.2 MSIX (full package)

- **Per-user, no admin:** packages install per user [S42] (verified).
- **Uninstall:** the cleanest. Removal deletes the package folder and the redirected AppData and registry writes [S42] (verified).
- **Install path changes on update.** Packages live in `C:\Program Files\WindowsApps\<package_full_name>`, read-only [S42] (verified).
- **Autostart:** `windows.startupTask` with `Enabled="true"` needs no consent call for packaged desktop apps. Even so, "the user must either launch the app at least once, or … enable startup functionality … in Settings". After that the user controls it from Settings or Task Manager [S43] (verified). A Required Intune deployment therefore won't autostart until the first launch.
- **Restrictions:**
  - No writes to HKLM or the install directory [S41].
  - HKCU and AppData writes are virtualized per package [S42] (verified). Settings written today to real HKCU or AppData would land in a private store.
- **Taskbar Strip, tray icons and global hotkeys:**
  - The full-trust (`mediumIL`) process is not in an app container [S42] (verified).
  - No MSIX document forbids `SetParent` into Explorer's window, `Shell_NotifyIcon` or `RegisterHotKey`, so all three should work **(inferred; spike it)**.
  - The Store would likely reject the Strip under policy 10.2.8 [S24].
- **Tray icon GUIDs:** a tray icon registered with `guidItem` is bound to the binary's path. Moving the binary breaks it, except when "both the original and moved binary files are Authenticode-signed by the same company" [S46] (verified). blinkengraphs uses the `tray-icon` crate, and whether that crate sets `NIF_GUID` is not checked. MSIX's versioned path makes this a live risk only if it does.
- **Services:**
  - MSIX supports LocalSystem, LocalService and NetworkService services. Not per-user services [S41].
  - A package with a service "will require admin privileges to install" (Windows 10 2004+), plus a restricted capability [S44] (verified).
  - M4 could ship as a separate per-machine MSIX, but a per-user main package can't carry it (inferred).
- **Intune:** MSIX LOB, or Win32 wrapping `Add-AppxPackage` [S27].
- **Signing:** the package publisher must equal the certificate subject [S5]. With SignPath, the package publisher would be "SignPath Foundation" (inferred).
- **Build:** `makeappx` and `signtool` ship with Windows SDK 10.0.26100 on `windows-latest` [S36] (verified). No Rust crate in this survey emits MSIX: cargo-packager lists only NSIS and WiX [S34].

### 2.3 Package with external location ("sparse" package)

- **What it is:** a signed identity-only MSIX that an existing installer registers against its install directory. It unlocks "startup tasks", notifications and file associations [S45] (verified).
- **Registration:** `Add-AppxPackage -ExternalLocation` or `AddPackageByUriAsync` per user; staging plus provisioning for per-machine [S45].
- **Runtime:** with `TrustLevel="mediumIL"`, `RuntimeBehavior="win32App"` and `unvirtualizedResources`, the app keeps running as a normal Win32 app [S45] (verified).
- **Signing:** it needs a production certificate [S45].
- **Verdict:** blinkengraphs needs none of these features today (a Run key covers autostart). Keep it as an add-on to an MSI if notifications or a StartupTask are wanted later (inferred).

### 2.4 Dual-mode WiX MSI

- **Mechanism:**
  - `ALLUSERS=2` with `MSIINSTALLPERUSER=1` gives a per-user default.
  - Per-user installs go to `%LocalAppData%\Programs` with no UAC prompt; per-machine installs go to Program Files with a prompt [S30] (verified).
  - Registry rows with Root `-1` resolve to HKCU or HKLM to match [S30]. That handles a `…\CurrentVersion\Run` value in either mode.
  - Later repairs, updates and removals reuse the original context [S30].
- **WiX authoring:** `Package/@Scope="perUserOrMachine"` "sets the ALLUSERS property to 2 and MSIINSTALLPERUSER property to 1". WiX v7 adds `perMachineOrUser` (per-machine default) [S31] (verified).
- **Rules for the per-user-capable package:** no elevated custom actions, no system-folder writes, and "Do not install any services" [S30] (verified). M4's service must ship as a separate MSI, which ADR 0008 already decided.
- **Intune:** fits every route.
  - MSI LOB with a dual-mode context choice [S27].
  - Win32 in User or System context, detected by MSI product code, updated by supersedence or by the MSI's own MajorUpgrade [S26].
  - The M4 MSI becomes a separate Win32 app in System context. It can be a dependency or an independent assignment [S26] (inferred).
- **Uninstall:** removes what the MSI installed. Settings the app writes itself (HKCU, AppData) stay unless the MSI authors removal (inferred).
- **Build:**
  - WiX 7.0.1 is current [S32-nuget]. It is a .NET tool; `windows-latest` preinstalls only WiX 3.14 [S36] (verified).
  - **Licence:** WiX v6+ carries the Open Source Maintenance Fee: "organizations that generate more than $10,000 in annual revenue" must sponsor. v7 enforces EULA acceptance (`-acceptEula wix7`) [S32] (verified). Whether a revenue-free personal OSS project owes anything is not stated **(low confidence; read the OSMF terms)**. WiX v5 predates the fee [S32].
  - **cargo-wix:** supports "v4+", with v3 as the default. Its last release was 0.3.9 on 2025-03-13 [S33] (verified). Calling `wix build -arch x64|arm64` directly on a hand-written `.wxs` avoids depending on a lagging wrapper (inferred).
- **Signing:** sign the exe, build the MSI, then sign the MSI. Artifact Signing [S5] and SignPath (nested signing) [S17] both handle MSI.

### 2.5 Inno Setup or NSIS exe installers

- **Inno Setup:**
  - **Per-user install:** `PrivilegesRequired=lowest` "will not request to be run with administrative privileges" [S35].
  - **Dual mode:** `PrivilegesRequiredOverridesAllowed=commandline dialog` adds `/ALLUSERS` and `/CURRENTUSER` [S35] (verified).
  - **ARM64:** supported [S35].
  - **Runner:** 6.7.1 is preinstalled on `windows-latest` [S36] (verified).
  - **Licence:** commercial users are "asked" to buy a licence [S35].
- **NSIS:** cargo-packager builds NSIS with an `install_mode` for all users or the current user, and accepts a custom `sign_command` [S34] (verified).
- **Intune:** Win32 only. No MSI LOB route, no product-code detection (use file or registry rules), and silent switches must be passed [S26] (inferred).
- **M4:** a second exe installer, or a service install in the exe installer's admin mode. Either is weaker than an MSI with `ServiceInstall` for Intune detection and repair (inferred).

### 2.6 Plain exe or zip through winget's portable type

- **What winget does:**
  - Portable installs go to `%LOCALAPPDATA%/Microsoft/WinGet/Packages/` (user scope) or `%PROGRAMFILES%/WinGet/Packages/` (machine scope) [S37] (verified).
  - `Scope` is `user` or `machine`; `arm64` is a valid architecture [S38].
  - `purgePortablePackage` defaults to `false`, so uninstall can leave files behind [S37] (verified).
- **No admin needed** for user scope [S37].
- **Autostart and shortcuts** become the app's job: it writes its own HKCU Run value, which winget won't remove (inferred).
- **Intune:** no native route [S28]. It needs a Win32 app wrapping `winget` or the exe itself.
- **Verdict:** fine for developers. As the primary format it pushes installer work into the app (inferred).
- **Better winget use:** publish a manifest that points at the signed MSI (`InstallerType: wix`, both scopes) [S38] (inferred).

### 2.7 Autostart options

| Mechanism | Per-user, no admin | Notes |
|---|---|---|
| HKCU `…\CurrentVersion\Run` | Yes | Runs "every time the user logs on". Command line ≤260 characters. Windows may delay it [S47] (verified). The app can own the toggle in its settings |
| HKLM `…\Run` | Needs admin to write | Applies to all users. A per-machine MSI can write it; the user can disable it in Task Manager (inferred) |
| MSIX or sparse `StartupTask` | Yes | Needs one launch or a Settings toggle; the user controls it [S43] |
| Task Scheduler at logon | Possible | Adds nothing over Run for a medium-IL app. Research 04 and ADR 0008 rejected the elevated variant |

Recommended: the app owns an HKCU Run value behind a settings toggle; a per-machine install also writes HKCU from the app on first launch, so each user can opt out (inferred). The MSI removes the value on uninstall through `RemoveRegistry` for the installing user. Values belonging to other users of a per-machine install stay behind (inferred).

### 2.8 Q2 comparison

| | Dual-mode MSI | MSIX | Inno or NSIS | winget portable |
|---|---|---|---|---|
| Intune routes | MSI LOB (dual mode) or Win32, User or System [S26][S27] | MSIX LOB or Win32 [S27] | Win32 only | Win32 wrapper only |
| Detection and update | MSI product code; MajorUpgrade or supersedence [S26] | Package version [S27] | File or registry rule; supersedence | Script |
| Per-user, no admin | Yes [S30] | Yes [S42] | Yes [S35] | Yes [S37] |
| Autostart | Run key | StartupTask after first launch [S43] | Run key | App writes Run key |
| Strip, tray, hotkeys | Unaffected (plain Win32) | Probably works; Store risk [S24] | Unaffected | Unaffected |
| Uninstall | Clean for authored items | Cleanest [S42] | Script-dependent | Partial by default [S37] |
| GitHub build | WiX 7 via dotnet tool; OSMF question [S32] | makeappx + manifest by hand [S36] | Inno preinstalled [S36] | Nothing to build |
| M4 service | Separate per-machine MSI with `ServiceInstall` (ADR 0008) | Separate admin-installed MSIX [S44] | Separate installer | Not applicable |

## 3. Recommendations

### Q1 — Signing, ranked

1. **Artifact Signing, individual Public Trust identity**, if the owner is located in the US or Canada and accepts their legal name and city on every binary.
   - Sign in the existing `build` job after `cargo build`, with `azure/artifact-signing-action` and OIDC.
   - Sign both `main-latest` and tagged releases with one certificate profile, so the EKU and identity stay constant.
   - Reasons: the official GitHub path, Entra-native, a durable EKU for the owner's own App Control policies, and Microsoft's recommended non-Store option [S2][S6][S9].
   - Expect warnings for weeks [S1]. Watch for intermediate-CA regressions [S12].
2. **SignPath Foundation**, if Artifact Signing is out of reach, or if the owner won't publish their name.
   - Costs: manual approval per release (tagged releases only), the publisher reads "SignPath Foundation", and acceptance is uncertain for a young executable project [S14].
3. **OV certificate in a cloud HSM**, only if neither of the above is available. Renewal every ≤460 days may restart reputation [S11][S18].
   - EV is not worth buying for SmartScreen [S2].

All options: keep the attestations, and expect them to do nothing for SmartScreen. Submit early releases to WDSI as an optional extra.

What would change this ranking:
- **Owner outside the US or Canada, without a legal entity:** Artifact Signing is out, so SignPath or OV moves up.
- **The owner forms an entity** (for example a sole-member LLC): organization validation then shows the entity name instead of a personal one [S4].
- **A tray-only Store MSIX is acceptable:** the Store removes SmartScreen entirely [S1], and Q1 then matters only for the direct download.
- **Recurring 2026-style CA-rotation regressions:** the case for a traditional CA strengthens.

### Q2 — Installer, ranked

1. **Dual-mode WiX MSI** (`Scope="perUserOrMachine"`), one per architecture, signed inside and out.
   - It fits every Intune route, gives per-user installs without admin, and gives clean detection and upgrades.
   - It is the format ADR 0008 already chose for M4, so main app and service share tooling.
   - Publish the same MSIs to winget with both scopes.
2. **Inno Setup with `PrivilegesRequired=lowest` and overrides.**
   - The fallback if WiX licensing (OSMF) or WiX authoring cost proves unacceptable.
   - Intune works through Win32 with file or registry detection.
3. **MSIX.** Only if Store distribution becomes the goal, and then tray-only (policy 10.2.8). Startup needs a first launch, and settings get virtualized.
   - Keep a **sparse package** in reserve as an MSI add-on if package identity is ever needed.
4. **winget portable** as the only format: no Intune path, and autostart and cleanup become app code.

What would change this ranking:
- **WiX OSMF applies** and the owner won't sponsor: pin WiX v5 or switch to Inno.
- **A spike shows `tray-icon` uses `NIF_GUID`:** the per-user versus per-machine path difference needs per-scope GUIDs. Signing relaxes the moved-binary case [S46].
- **Intune Store app support for ARM64:** this lifts the Store's managed-fleet blocker [S28].

## 4. Spikes and questions before committing

1. **Owner:** which browser blocked the download? Where does the owner live, and will they publish their legal name as publisher?
2. Sign an ARM64 exe and an MSI from `windows-latest` with Artifact Signing. Confirm `signtool verify /pa` on an ARM64 machine.
3. Build a minimal `perUserOrMachine` MSI with WiX 7. Install it per-user from a standard account through Intune (Win32, User context), then per-machine (System context). Check detection and a MajorUpgrade in each.
4. Check whether `tray-icon` 0.26 sets `NIF_GUID`.
5. Ask Partner Center whether Artifact Signing still satisfies policy 10.2.9's Trusted Root Program chain after the CCADB removal.

## Sources

- [S1] SmartScreen reputation for Windows app developers (2026-05-04): https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation
- [S2] Code signing options for Windows app developers (2026-10-29): https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options
- [S3] Current status of Windows app distribution features (reviewed August 2026): https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/distribution-feature-status
- [S4] Quickstart: Set up Artifact Signing (updated 2026-11-20): https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart
- [S5] Artifact Signing FAQ (2026-11-17): https://learn.microsoft.com/en-us/azure/artifact-signing/faq
- [S6] Artifact Signing certificate management (2026-09-30): https://learn.microsoft.com/en-us/azure/artifact-signing/concept-certificate-management
- [S7] Set up signing integrations to use Artifact Signing: https://learn.microsoft.com/en-us/azure/artifact-signing/how-to-signing-integrations
- [S8] Azure retail price API, "Trusted Signing" meters (queried 2026-11-21): https://prices.azure.com/api/retail/prices?$filter=contains(serviceName,'Signing') ; pricing page: https://azure.microsoft.com/en-us/pricing/details/artifact-signing/
- [S9] Azure/artifact-signing-action README: https://github.com/Azure/artifact-signing-action
- [S10] "Simplifying Code Signing for Windows Apps: Artifact Signing (GA)", Microsoft Security blog, 2026-01-12: https://techcommunity.microsoft.com/blog/microsoft-security-blog/-/4482789
- [S11] Microsoft Q&A, "Does SmartScreen reputation persist across Artifact Signing certificate rotation" (moderator answer 2026-10-15): https://learn.microsoft.com/en-us/answers/questions/6037566/does-smartscreen-reputation-persist-across-artifac
- [S12] Azure/artifact-signing-action issue #128, SmartScreen after CA migration: https://github.com/Azure/artifact-signing-action/issues/128
- [S13] Microsoft Q&A, "users see SmartScreen on each release since intermediate CA rotated": https://learn.microsoft.com/en-us/answers/questions/5954185/artifact-signing-users-see-smartscreen-on-each-rel
- [S14] SignPath Foundation code of conduct and conditions (marked draft): https://signpath.org/terms
- [S15] SignPath Foundation: https://signpath.org/
- [S16] SignPath, GitHub trusted build system: https://docs.signpath.io/trusted-build-systems/github
- [S17] SignPath artifact configuration reference: https://docs.signpath.io/artifact-configuration/reference
- [S18] CA/B Forum Ballot CSC-31, maximum validity reduction: https://cabforum.org/2025/11/17/ballot-csc-31-maximum-validity-reduction
- [S19] Eric Lawrence, "Best Practices for SmartScreen AppRep", 2024-11-15 (ex-Microsoft, secondary): https://textslashplain.com/2024/11/15/best-practices-for-smartscreen-apprep/
- [S20] Microsoft Support, SmartScreen in Microsoft Edge: https://support.microsoft.com/en-us/microsoft-edge/what-is-smartscreen-and-how-can-it-help-protect-me-1c9a874a-6826-be5e-45b1-67fa445a74c8
- [S21] Google Chrome Help, "Google Chrome blocks some downloads": https://support.google.com/chrome/answer/6261569
- [S22] Microsoft Security Intelligence file submission: https://www.microsoft.com/en-us/wdsi/filesubmission
- [S23] Store app package requirements for MSI/EXE: https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msi/app-package-requirements
- [S24] Microsoft Store Policies 7.20 (2026-09-14), §10.2.4, §10.2.8, §10.2.9: https://learn.microsoft.com/en-us/windows/apps/publish/store-policies
- [S25] Intune, Win32 app management: https://learn.microsoft.com/en-us/intune/app-management/deployment/win32
- [S26] Intune, Add and assign Win32 apps (2026-11-11): https://learn.microsoft.com/en-us/intune/app-management/deployment/add-win32
- [S27] Intune, Add a Windows line-of-business app: https://learn.microsoft.com/en-us/intune/app-management/deployment/add-lob-windows
- [S28] Intune, Add Microsoft Store apps (2026-06-25): https://learn.microsoft.com/en-us/intune/app-management/deployment/add-microsoft-store
- [S29] Intune, Enterprise App Catalog app: https://learn.microsoft.com/en-us/intune/app-management/deployment/add-enterprise-catalog-app
- [S30] Windows Installer, Single Package Authoring: https://learn.microsoft.com/en-us/windows/win32/msi/single-package-authoring
- [S31] WiX, PackageScopeType: https://docs.firegiant.com/wix/schema/wxs/packagescopetype/
- [S32] WiX, Open Source Maintenance Fee: https://docs.firegiant.com/wix/osmf/ ; [S32-nuget] WiX versions on NuGet (7.0.1 latest, queried 2026-11-21): https://api.nuget.org/v3-flatcontainer/wix/index.json
- [S33] cargo-wix README: https://github.com/volks73/cargo-wix ; crates.io (0.3.9, 2025-03-13): https://crates.io/crates/cargo-wix
- [S34] cargo-packager README: https://github.com/crabnebula-dev/cargo-packager ; WindowsConfig: https://docs.rs/cargo-packager/latest/cargo_packager/config/struct.WindowsConfig.html ; NsisConfig: https://docs.rs/cargo-packager/latest/cargo_packager/config/struct.NsisConfig.html
- [S35] Inno Setup PrivilegesRequired: https://jrsoftware.org/ishelp/topic_setup_privilegesrequired.htm ; PrivilegesRequiredOverridesAllowed: https://jrsoftware.org/ishelp/topic_setup_privilegesrequiredoverridesallowed.htm ; overview: https://jrsoftware.org/isinfo.php
- [S36] GitHub runner image Windows Server 2025 with VS 2026, `windows-latest` (image 20261116): https://github.com/actions/runner-images/blob/main/images/windows/Windows2025-VS2026-Readme.md ; labels: https://github.com/actions/runner-images/blob/main/README.md
- [S37] winget `settings` (portable roots, purge): https://learn.microsoft.com/en-us/windows/package-manager/winget/settings
- [S38] winget manifest schema 1.12.0, installer: https://github.com/microsoft/winget-pkgs/blob/master/doc/manifest/schema/1.12.0/installer.md
- [S39] winget-cli issue #6232 (MOTW written on download): https://github.com/microsoft/winget-cli/issues/6232
- [S40] winget-pkgs Troubleshoot.md (fork mirror; upstream path not confirmed): https://github.com/renovate-bot/winget-pkgs/blob/master/Troubleshoot.md
- [S41] MSIX, Prepare to package a desktop application: https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-prepare
- [S42] MSIX, How packaged desktop apps run on Windows (2026-09-30): https://learn.microsoft.com/en-us/windows/msix/desktop/desktop-to-uwp-behind-the-scenes
- [S43] StartupTask class: https://learn.microsoft.com/en-us/uwp/api/windows.applicationmodel.startuptask
- [S44] MSIX, Converting an installer with services: https://learn.microsoft.com/en-us/windows/msix/packaging-tool/convert-an-installer-with-services
- [S45] Grant package identity by packaging with external location (2026-10-03): https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/grant-identity-to-nonpackaged-apps
- [S46] NOTIFYICONDATAW, `guidItem` troubleshooting: https://learn.microsoft.com/en-us/windows/win32/api/shellapi/ns-shellapi-notifyicondataw
- [S47] Run and RunOnce registry keys: https://learn.microsoft.com/en-us/windows/win32/setupapi/run-and-runonce-registry-keys

### Known gaps

- **Not documented anywhere I found:**
  - Whether SmartScreen keys publisher reputation on the Artifact Signing identity, the profile EKU or the intermediate CA. Microsoft's statements conflict with observed behaviour [S11][S12][S13].
  - Whether a shared "SignPath Foundation" publisher helps a new project's first downloads.
  - Whether Chrome's download protection weighs Authenticode.
- **Not fetched:**
  - NSIS MultiUser docs (sourceforge returned 403).
  - The OSMF terms' definition of revenue.
  - The upstream winget-pkgs Troubleshoot.md.
  - The `NSISInstallerMode` enum page.
- **Low confidence:**
  - Store acceptance of Artifact Signing under policy 10.2.9.
  - Whether winget-delivered installers trigger SmartScreen prompts.
  - Whether `SetParent` into `Shell_TrayWnd` works from a packaged process.
  - Whether SignPath would accept a network-diagnostics tool.

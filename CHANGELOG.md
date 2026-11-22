# Changelog

## [3.0.0](https://github.com/reconbot/blinkengraphs-win/compare/v2.0.0...v3.0.0) (2026-11-22)


### ⚠ BREAKING CHANGES

* history.db starts empty; Minutes from earlier versions are dropped, and the rollup_days setting is replaced by archive_days.

### Features

* every Link is measured on its own interface; History, stats and graphs per Link ([#48](https://github.com/reconbot/blinkengraphs-win/issues/48)) ([49ae234](https://github.com/reconbot/blinkengraphs-win/commit/49ae2343fb7491d701014a161d092c742e0ac255)), closes [#40](https://github.com/reconbot/blinkengraphs-win/issues/40)
* History is Raw plus Archives in SQLite; graphs draw one source per range ([#44](https://github.com/reconbot/blinkengraphs-win/issues/44)) ([723019e](https://github.com/reconbot/blinkengraphs-win/commit/723019e4449f759b76c03462cf64df34ecddf51f)), closes [#40](https://github.com/reconbot/blinkengraphs-win/issues/40)
* Pauses: suspend, resume, idle rest, shutdown and kills are not measured, and Rounds they cut record nothing ([#45](https://github.com/reconbot/blinkengraphs-win/issues/45)) ([6484669](https://github.com/reconbot/blinkengraphs-win/commit/6484669b811ccc41c703a803ecc1f5a540c4eae4)), closes [#40](https://github.com/reconbot/blinkengraphs-win/issues/40)
* spec 5.1.0: stats from now, Resuming Verdict, tie rule, and pure Pause, Archive and graph-range cores ([#42](https://github.com/reconbot/blinkengraphs-win/issues/42)) ([a04dd04](https://github.com/reconbot/blinkengraphs-win/commit/a04dd047da9467d44560809b06e56200c1de401d)), closes [#40](https://github.com/reconbot/blinkengraphs-win/issues/40)
* TCP and UDP Probes are a checkbox with a port beside it ([#47](https://github.com/reconbot/blinkengraphs-win/issues/47)) ([94280d0](https://github.com/reconbot/blinkengraphs-win/commit/94280d0aa38463559b82668d4979dd272c1efda8))

## [2.0.0](https://github.com/reconbot/blinkengraphs-win/compare/v1.5.0...v2.0.0) (2026-11-22)


### ⚠ BREAKING CHANGES

* history and RTTs follow spec 3.0.0: fixed-beat rounds, one percentile rank, whole-ms RTTs, minutes written once complete, graph seam where storage ends ([#38](https://github.com/reconbot/blinkengraphs-win/issues/38))

### Features

* history and RTTs follow spec 3.0.0: fixed-beat rounds, one percentile rank, whole-ms RTTs, minutes written once complete, graph seam where storage ends ([#38](https://github.com/reconbot/blinkengraphs-win/issues/38)) ([2bbb58a](https://github.com/reconbot/blinkengraphs-win/commit/2bbb58a82ffca8ec437d399aad6b266a0dac70f5))

## [1.5.0](https://github.com/reconbot/blinkengraphs-win/compare/v1.4.0...v1.5.0) (2026-11-22)


### Features

* every default comes from the merged defaults; Graph aggregate p95; graphs name their aggregate ([#32](https://github.com/reconbot/blinkengraphs-win/issues/32)) ([749e31c](https://github.com/reconbot/blinkengraphs-win/commit/749e31c6ea1a6b3ccba48c5a88531d55726c230f))
* live Captive Portal check feeds the Verdict ([#34](https://github.com/reconbot/blinkengraphs-win/issues/34)) ([709545e](https://github.com/reconbot/blinkengraphs-win/commit/709545e2d970829a385d23ebbc02b0ee3d236f9a))
* pass spec v1.7.0 vectors read from the spec; Captive Portal Finding and Public Address reading ([#30](https://github.com/reconbot/blinkengraphs-win/issues/30)) ([f50b995](https://github.com/reconbot/blinkengraphs-win/commit/f50b99576c7211b2bbd5be1c4cc7c28e68621225))
* silent upgrades relaunch the app; software Panel without OpenGL 2.0 ([#24](https://github.com/reconbot/blinkengraphs-win/issues/24)) ([4d03254](https://github.com/reconbot/blinkengraphs-win/commit/4d03254e5bf955e1dfccb23fb0188b2c813e3963))
* Theme colors come from the spec's tokens; an explicit Light or Dark applies to Meters too ([#31](https://github.com/reconbot/blinkengraphs-win/issues/31)) ([a04d530](https://github.com/reconbot/blinkengraphs-win/commit/a04d530547083a2b5414554b939c3592a4120e7c))
* VPN Fault Domain in the Verdict (no Windows VPN detection yet) ([#22](https://github.com/reconbot/blinkengraphs-win/issues/22)) ([dadeb72](https://github.com/reconbot/blinkengraphs-win/commit/dadeb729c2258509f5a8f03a8c35abfc7a026cdf))

## [1.4.0](https://github.com/reconbot/blinkengraphs-win/compare/v1.3.0...v1.4.0) (2026-11-22)


### Features

* defaults from the spec's defaults.json with defaults.override.json merged over it ([#19](https://github.com/reconbot/blinkengraphs-win/issues/19)) ([eb3f313](https://github.com/reconbot/blinkengraphs-win/commit/eb3f313efe1473a9034a8eececebea9f5a4b1104))
* MSI installer for per-user or per-machine installs, with the spec's brand icon ([#18](https://github.com/reconbot/blinkengraphs-win/issues/18)) ([a56dbdd](https://github.com/reconbot/blinkengraphs-win/commit/a56dbddd26d783c7f207f84c47350410642f13a8))
* Offline Fault Domain, IPv6 Probes and Hop Trace, per-family Verdict rule, Hop names; pin spec v1.4.0 ([#21](https://github.com/reconbot/blinkengraphs-win/issues/21)) ([5220517](https://github.com/reconbot/blinkengraphs-win/commit/5220517777f61877b5b53a49e7bf9e493f8cf73e))

## [1.3.0](https://github.com/reconbot/blinkengraphs-win/compare/v1.2.0...v1.3.0) (2026-11-21)


### Features

* All history range and CSV export of every stored minute ([#12](https://github.com/reconbot/blinkengraphs-win/issues/12)) ([14acf3a](https://github.com/reconbot/blinkengraphs-win/commit/14acf3a96dbd74be3ef5f645badf70d4b3cff662))


### Bug Fixes

* save recent history on quit, show the running copy on relaunch, log crashes ([#14](https://github.com/reconbot/blinkengraphs-win/issues/14)) ([70a5efe](https://github.com/reconbot/blinkengraphs-win/commit/70a5efebeddf3d7707bda24a4be96bca3813850f))

## [1.2.0](https://github.com/reconbot/blinkengraphs-win/compare/v1.1.0...v1.2.0) (2026-11-21)


### Features

* configurable history length with a disk estimate ([#11](https://github.com/reconbot/blinkengraphs-win/issues/11)) ([822c6e3](https://github.com/reconbot/blinkengraphs-win/commit/822c6e3369d6d5c3d778a185ead73e721f62500d))
* spec v1 exports (probe stats, hop trace, defaults), gateway-unmeasured Verdict; split M3/M4 ([#8](https://github.com/reconbot/blinkengraphs-win/issues/8)) ([32bcaf6](https://github.com/reconbot/blinkengraphs-win/commit/32bcaf6e1dbebef1cd604c17b8a1392bf5dfa4c4))
* themes, colors, per-Probe window and aggregate, graph aggregate from per-minute histograms ([#10](https://github.com/reconbot/blinkengraphs-win/issues/10)) ([01dba62](https://github.com/reconbot/blinkengraphs-win/commit/01dba62cb78d097481bf141f0b91d47988a3cbc6))

## [1.1.0](https://github.com/reconbot/blinkengraphs-win/compare/v1.0.0...v1.1.0) (2026-11-21)


### Features

* spec split, Probe Group tables with UDP/ports, Panel fixes from hardware testing ([#6](https://github.com/reconbot/blinkengraphs-win/issues/6)) ([606db34](https://github.com/reconbot/blinkengraphs-win/commit/606db3454d46f7ce6bdb1f845327604529fc55f0))

## [1.0.0](https://github.com/reconbot/blinkengraphs-win/compare/v0.1.0...v1.0.0) (2026-11-21)


### Features

* M0 skeleton, plan, CI and releases ([#1](https://github.com/reconbot/blinkengraphs-win/issues/1)) ([bc5a2a6](https://github.com/reconbot/blinkengraphs-win/commit/bc5a2a683dcfed5ba7be87dea4c78557aae5f7f6))
* v1.0 — Tray Icons, Probes, Panel, Taskbar Strip, History ([#2](https://github.com/reconbot/blinkengraphs-win/issues/2)) ([4e80b6e](https://github.com/reconbot/blinkengraphs-win/commit/4e80b6ec582b9207d8b6aeeb617355e08fec4166))


### Performance Improvements

* footprint budgets, SHA-pinned actions, cross-platform proposal ([#4](https://github.com/reconbot/blinkengraphs-win/issues/4)) ([f1cf4a2](https://github.com/reconbot/blinkengraphs-win/commit/f1cf4a2de91336fa30f5dff1a730fbe96b55644a))

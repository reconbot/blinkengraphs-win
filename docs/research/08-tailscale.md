# 08 — Tailscale on Windows, and what it does to Links

Researched 2026-11-23. Question: when Tailscale runs on Windows, does each Link still measure what ADR 0020 says it does? This covers no exit node, an exit node with LAN access off, an exit node with LAN access on, and MagicDNS. Tailscale's docs give the behaviour; its Windows client source gives the mechanism. Source line numbers are at commit `af9bfa0` [S1].

Markers as in 07: **(verified)** means read in a primary source, **(inferred)** means my reasoning from the sources, **(low confidence)** means `tools/linkcheck` must settle it.

## 1. What Tailscale changes

**Without an exit node, Tailscale is not a Link.** The adapter carries `100.64.0.0/10`, `fd7a:115c:a1e0::/48` and any accepted subnet routes, but no Gateway and no default route, so `probe::links()` skips it (inferred from [S2] §routes and `links()`).

**With an exit node, Tailscale carries the default route.**
- `tailscaled` adds `0.0.0.0/0` and `::/0` to its adapter [S3, local.go:6960-6976] (verified).
- When the routes include a default, it sets the adapter's interface metric to 0 with automatic metric off [S4, ifconfig_windows.go:466-491] (verified).
- The adapter then wins the default route. The physical adapter keeps its own default route and Gateway at a higher metric (inferred).
- The app therefore follows the Tailscale Link, classed `Vpn` if its `IfType` is 53, `IF_TYPE_PROP_VIRTUAL` (low confidence). Its Gateway Probes come from the first non-tunnel Link (ADR 0020).

**LAN access off: the LAN is routed into the tunnel.** Tailscale adds every non-Tailscale, non-loopback, non-Hyper-V subnet on the machine to the tunnel's routes, "so that we do not leak any traffic" [S3, local.go:6982-6990] (verified). Unpinned traffic to `192.168.1.1` goes to the exit node, which cannot reach it (inferred).

**A WFP kill switch blocks the physical adapters.** When the routes include a default, `tailscaled` starts a child process (`tailscaled /firewall`) [S5, router_windows.go:168-182, 336-391] (verified). That process installs WFP rules [S6, firewall.go:148-191] (verified):
- permits at weight 15 for `tailscaled` itself, anything on the Tailscale interface, loopback, and **any TCP or UDP to remote port 53** on any interface;
- permits at weight 12 for DHCPv4, DHCPv6 and NDP;
- a block of everything else at weight 0.

LAN access on adds permits for the machine's own subnets (`LocalRoutes`) and their link-local multicast and broadcast [S6, firewall.go:195-253; S3, local.go:6984-6986] (verified). The Internet stays blocked off the tunnel.

**MagicDNS.**
- With "Use Tailscale DNS" on and no split domains, Tailscale sets `100.100.100.100` as the adapter's DNS server and adds an NRPT catch-all rule (`.`), making it the only resolver [S7, manager_windows.go:407-466] (verified).
- With split domains, only those domains get NRPT rules [S7] (verified).
- `100.100.100.100` is routed into the tunnel and answered by `tailscaled` [S8] (verified).
- So `system-dns` becomes `100.100.100.100` whenever the Tailscale adapter carries the default route, which is the adapter the app reads it from (inferred).

## 2. What that does to the app

| State | Followed Link | Its Probes | Verdict Gateway Probes (from the Wi-Fi or Ethernet Link) |
|---|---|---|---|
| Tailscale on, no exit node | the physical Link | normal | normal |
| Exit node, LAN access off | Tailscale | through the exit node | **blocked by WFP**: ICMP to the Gateway is neither the tunnel nor port 53 |
| Exit node, LAN access on | Tailscale | through the exit node | allowed: the Gateway is in `LocalRoutes` |

Every column below is inferred from §1 and unverified on a machine.

**The likely bug: exit node with LAN access off.**
- Every Probe pinned to the physical Link is blocked, except DNS (port 53 is allowed on any interface).
- How the block surfaces is unknown: an immediate error (`WSAEACCES` 10013 for sockets, or an `IP_*` status for ICMP), or a silent drop that times out (low confidence).
- If it times out, the physical Link shows 100% loss on everything but DNS.
- The Verdict, fed that Link's Gateway Probes, would blame the router while the network works.
- The spec's rule is that a Probe the OS refuses is `denied`, never loss (`CONTEXT.md` Probe; `rules/probe-stats.md`). A WFP block is that case if Windows reports it as an error.

**Exit node, LAN access on.** The Gateway answers, but `1.1.1.1` pinned to the physical Link is still blocked. That Link's Internet Probes would show as lost, though only its Panel line shows them, not the Verdict.

**Without an exit node, nothing changes.** Only subnet routes and the tailnet ranges go to the tunnel.

## 3. What `tools/linkcheck` measures

`tools/linkcheck` reads the error codes that the app's Transport discards. Each run prints:
- the adapters with `IfType`, metric and weak host send;
- the app's Links, the followed one and the Gateway-Probe Link;
- `GetBestRoute2`, unconstrained and constrained to each Link;
- ICMP, TCP 443 and DNS, pinned per Link and unpinned, with the Win32 error for each failure;
- Tailscale's state from its CLI, and the effective NRPT rules.

On Tailscale it answers:
1. Whether the WFP block shows as an error (and which) or as a timeout. This decides whether it can be `denied`.
2. Whether `IfType` is 53, so the Link is classed `Vpn`.
3. Whether `GetBestRoute2` constrained to the physical Link still answers through it under an exit node (research 07 §4).
4. Whether ICMP from the physical Link's source address follows the source or the default route (research 07 §3).

`linkcheck --resume` reports, per wake, how far each clock moved and how long each Link took to answer. That covers research 07 §1 (the unbiased clock under Modern Standby) and the settle time in ADR 0019.

### How to run it

Download `linkcheck.exe` from the CI artifact `blinkengraphs-x86_64-pc-windows-msvc` (or `aarch64`), or run `cargo run --release -p linkcheck --`. It needs no admin rights. Run it once per state, saving each report:

```
linkcheck --label "Tailscale off"                  > 1-off.md
linkcheck --label "Tailscale on, no exit node"     > 2-on.md
linkcheck --label "exit node, LAN access off"      > 3-exit.md
linkcheck --label "exit node, LAN access on"       > 4-exit-lan.md
linkcheck --resume 3 --label "Wi-Fi, lid close"    > 5-resume.md
```

Reports hide public and Tailscale addresses, NRPT domains and the device bytes of MACs. `--raw` keeps them, so don't post a `--raw` report.

## 4. What the results decide

- **The block shows as an error.** Record it as `denied` on Windows: the Probe is not recorded, and the Panel says the VPN blocks that Link. This needs `Transport` to return why a send failed, not just `None`.
- **The block times out.** The app can't tell it from loss per Probe. It would have to infer the block from the VPN state, for example by skipping the physical Link's non-DNS Probes while a full-tunnel VPN carries the default route and that Link has no permitted route to the Target. That is a spec question: whether a Link a VPN firewalls off is measured at all.
- Either way, the Verdict's Gateway Probes under an exit node with LAN access off measure nothing useful. The Verdict should treat them as absent, not lost.

## Sources

- [S1] tailscale/tailscale at `af9bfa07c3425b144330f6fa61bc0221f930dd95` (2026-11-22). https://github.com/tailscale/tailscale/tree/af9bfa07c3425b144330f6fa61bc0221f930dd95
- [S2] Tailscale, "Subnet routers". https://tailscale.com/docs/features/subnet-routers
- [S3] `ipn/ipnlocal/local.go`, exit-node routes and `LocalRoutes`. https://github.com/tailscale/tailscale/blob/af9bfa07c3425b144330f6fa61bc0221f930dd95/ipn/ipnlocal/local.go#L6960-L6992
- [S4] `wgengine/router/osrouter/ifconfig_windows.go`, metric 0 with a default route. https://github.com/tailscale/tailscale/blob/af9bfa07c3425b144330f6fa61bc0221f930dd95/wgengine/router/osrouter/ifconfig_windows.go#L466-L491
- [S5] `wgengine/router/osrouter/router_windows.go`, the kill-switch child process. https://github.com/tailscale/tailscale/blob/af9bfa07c3425b144330f6fa61bc0221f930dd95/wgengine/router/osrouter/router_windows.go#L168-L182
- [S6] `wf/firewall.go`, the WFP rules. https://github.com/tailscale/tailscale/blob/af9bfa07c3425b144330f6fa61bc0221f930dd95/wf/firewall.go#L148-L253
- [S7] `net/dns/manager_windows.go`, primary versus split DNS and the NRPT catch-all. https://github.com/tailscale/tailscale/blob/af9bfa07c3425b144330f6fa61bc0221f930dd95/net/dns/manager_windows.go#L407-L466
- [S8] Tailscale, "MagicDNS". https://tailscale.com/docs/features/magicdns
- [S9] Tailscale, "Exit nodes (route all traffic)": LAN access is off by default and blocks local devices. https://tailscale.com/docs/features/exit-nodes

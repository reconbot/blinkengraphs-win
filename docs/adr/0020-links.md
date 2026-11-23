# Every Link is measured on its own interface

Spec 5.0.0 measures each active Link (Ethernet, Wi-Fi, a VPN) on its own, keeps its History, stats and baselines apart, and has the Verdict judge the Followed Adapter's Link (spec `rules/verdict.md`, `CONTEXT.md` Link). Windows does it this way:

- **A Link is an adapter that is up and reaches beyond itself.** It has a Gateway, or it carries a default route on-link, as a VPN may. `GetAdaptersAddresses(GAA_FLAG_INCLUDE_GATEWAYS)` lists them. The engine reads them every 10 s with the adapters, and re-plans the Prober only when they change.
- **A `LinkId` is the adapter's NET_LUID, its Gateway and the Gateway's MAC address.** The Gateway is the IPv4 one, else the IPv6 one, else none. A NET_LUID survives a restart, unlike an interface index. The MAC comes from the neighbor table (`GetIpNetEntry2`, else `ResolveIpNetEntry2`; no admin) and is kept for the process once read, so two homes on `192.168.1.1` keep separate History (spec 6.1.0). History keys name the Link this way (`probe:icmp 1.1.1.1 on 0006000000000000@192.168.1.1/00-1a-2b-3c-4d-5e`).
- **Every Probe Group runs once per Link** (`engine::groups_on`). Each Round carries its Link.
  - TCP, UDP and DNS sockets set `IP_UNICAST_IF` (index in network byte order) or `IPV6_UNICAST_IF` (host order).
  - ICMP uses `IcmpSendEcho2Ex` and `Icmp6SendEcho2` with the Link's source address.
  - A Link's Gateway Probe goes to its own Gateway.
- **A Target goes out on a Link only if that Link routes it.** `GetBestRoute2` is constrained to the Link's NET_LUID, and the route must leave through it. Otherwise the Probe records nothing, as for an unreachable IP version. A split-tunnel VPN therefore measures only what it carries.
- **History follows one Link.** That is the pinned adapter's Link, or else the one carrying the default route (spec #50). The Verdict, the Meters and `History::now` speak for it, and the Panel draws every Link, or one picked. Its changes mark the 1m, 15m and 1h graphs.
- **A Link that appears or changes waits the settle time.** Its Rounds in `[change, change + settle)` are dropped, while the other Links go on; it is not a Pause (spec gap 2).
- **A VPN is a followed Link whose kind is `Vpn`.** The Verdict then sets `via_vpn`. A tunnel has no Gateway of its own, so the Verdict's Gateway Probes are those of the first Link that is not a tunnel (spec gap 1). This rests on the unverified `IfType` classing below.
- **No Links listed** happens off Windows, in tests and on screenshots. Probes then go where the OS routes them, keyed with no Link, as before.

## Not yet verified on a machine (research 07)

- Whether `GetBestRoute2` constrained to an interface without a route errors or answers with another interface. Checking the returned interface covers both cases.
- Whether a source address keeps ICMP on its interface through a full-tunnel VPN, or with a VPN that filters through WFP rather than routes.
- Whether every VPN adapter shows as `IF_TYPE_PROP_VIRTUAL`, `IF_TYPE_PPP` or `IF_TYPE_TUNNEL`. Only the Link's kind depends on it.

## Not yet done

- Throughput is still the Followed Adapter's alone. Per-Link throughput Metrics are a follow-up.
- Hop statistics and Link changes are kept in memory, not in Raw on disk, so a restart starts them over.

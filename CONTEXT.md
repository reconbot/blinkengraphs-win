# blinkengraphs

A Windows 11 taskbar system monitor that answers two questions at a glance: "why is my computer slow?" and "why isn't my network responding?"

## Language

### Measuring the machine

**Section**:
One of the five things blinkengraphs watches: CPU, Memory, Network, Disk, Latency. Each Section has one Meter and one area in the Panel.
_Avoid_: category, widget, module

**Metric**:
One named series of numbers within a Section, such as CPU total, commit charge, or download throughput. The user switches each Metric on or off for each Surface.
_Avoid_: counter, stat, sensor

**Sample**:
The values of every enabled Metric at one instant.
_Avoid_: reading, tick, datapoint

**Memory Pressure**:
How close the machine is to running out of committable memory, together with how hard it is paging. It is not "% of RAM used."
_Avoid_: memory usage, RAM %

**Followed Adapter**:
The network adapter whose traffic the Network Section shows: either the one that currently carries the default route, or one the user pinned.
_Avoid_: interface (overloaded), NIC

### Measuring the network

**Target**:
A host that blinkengraphs measures reachability to, such as the Gateway, 1.1.1.1, or 8.8.8.8.
_Avoid_: host, endpoint, server

**Gateway**:
The first-hop router on the Followed Adapter's default route.
_Avoid_: router, default gateway IP

**Probe**:
One kind of reachability measurement against one Target, over one protocol: ICMP echo, TCP handshake, or UDP DNS query. A Probe yields a round-trip time or a loss.
_Avoid_: ping (only ICMP is a ping), check, test

**Probe Group**:
A set of Probes that run together on one cadence the user chooses.
_Avoid_: schedule, batch

**Hop Trace**:
A continuous traceroute towards a Target that keeps latency and loss for every Hop, in the style of mtr.
_Avoid_: traceroute (implies one-shot), mtr (name of a tool)

**Hop**:
One router on the path to a Target, identified by its TTL distance and the address that answered.
_Avoid_: node, router

### Explaining

**Verdict**:
A plain-language conclusion about the current Fault Domain, such as "Gateway OK, 1.1.1.1 losing 12% → ISP problem", derived only from rules.
_Avoid_: diagnosis, alert, insight

**Fault Domain**:
Where the trouble is: Local Load, LAN, ISP, DNS, Remote, ICMP Filtering, or Self-Saturation (your own traffic filling the link).
_Avoid_: root cause, failure type

**Process Snapshot**:
A list of the top processes per Section, taken only while the Panel is open.
_Avoid_: blame log, process history

### Showing

**Meter**:
The always-visible small graph of one Section's enabled Metrics.
_Avoid_: icon (that's one Surface), sparkline, gauge

**Surface**:
Where Meters appear: Tray Icons or the Taskbar Strip.
_Avoid_: display, view

**Tray Icons**:
The Surface where each Meter is its own notification-area icon.
_Avoid_: systray, menu-bar item

**Taskbar Strip**:
The Surface where all Meters sit side by side, docked inside the taskbar. When it is active, a single Menu Icon stays in the tray.
_Avoid_: deskband, toolbar, overlay

**Panel**:
The detail window that opens from any Meter or the hotkey. It shows the Verdict, history graphs, Hop Traces, and the Process Snapshot.
_Avoid_: popup, dashboard, flyout

**History**:
Every Sample, Probe result and Hop statistic kept for later viewing: recent Samples at full resolution in memory, older ones as Rollups on disk.
_Avoid_: log, timeline

**Rollup**:
A one-minute summary (min, mean, max, loss) of a Metric or Probe.
_Avoid_: aggregate, bucket

**Elevated Extras**:
Measurements that only work when blinkengraphs runs as Administrator: bytes per process on the network, and UDP Hop Traces.
_Avoid_: admin mode, pro features

use super::{from_wide, to_wide};
use crate::calc::NetCounters;
use crate::{AdapterChoice, AdapterInfo};
use std::net::{Ipv4Addr, Ipv6Addr};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::NO_ERROR;
use windows_sys::Win32::NetworkManagement::IpHelper::{
    ConvertInterfaceAliasToLuid, FreeMibTable, GetBestRoute2, GetIfEntry2, GetIfTable2,
    IF_TYPE_SOFTWARE_LOOPBACK, IF_TYPE_TUNNEL, MIB_IF_ROW2, MIB_IF_TABLE2, MIB_IPFORWARD_ROW2,
};
use windows_sys::Win32::NetworkManagement::Ndis::NET_LUID_LH;
use windows_sys::Win32::Networking::WinSock::{
    AF_INET, AF_INET6, IN_ADDR, IN_ADDR_0, IN6_ADDR, IN6_ADDR_0, SOCKADDR_IN, SOCKADDR_IN6,
    SOCKADDR_INET,
};

const ROUTE_PROBE_V4: Ipv4Addr = Ipv4Addr::new(1, 1, 1, 1);
const ROUTE_PROBE_V6: Ipv6Addr = Ipv6Addr::new(0x2606, 0x4700, 0x4700, 0, 0, 0, 0, 0x1111);

/// LUID and counters of the Followed Adapter.
pub fn followed(choice: &AdapterChoice) -> Option<(u64, NetCounters)> {
    let luid = match choice {
        AdapterChoice::FollowDefaultRoute => default_route_luid()?,
        AdapterChoice::Pinned(alias) => luid_of_alias(alias)?,
    };
    counters(luid)
}

/// One adapter's counters by NET_LUID.
pub fn counters(luid: u64) -> Option<(u64, NetCounters)> {
    // GetIfEntry2 reads one row by InterfaceLuid; cheaper than GetIfTable2 every second.
    let mut row = MIB_IF_ROW2 {
        InterfaceLuid: NET_LUID_LH { Value: luid },
        ..Default::default()
    };
    if unsafe { GetIfEntry2(&mut row) } != NO_ERROR {
        return None;
    }
    Some((
        luid,
        NetCounters {
            rx: row.InOctets,
            tx: row.OutOctets,
            errors: row
                .InErrors
                .wrapping_add(row.InDiscards)
                .wrapping_add(row.OutErrors)
                .wrapping_add(row.OutDiscards),
        },
    ))
}

pub fn adapters() -> Vec<AdapterInfo> {
    let default = default_route_luid();
    let mut table: *mut MIB_IF_TABLE2 = null_mut();
    if unsafe { GetIfTable2(&mut table) } != NO_ERROR {
        return Vec::new();
    }
    let rows = unsafe {
        std::slice::from_raw_parts(
            (&raw const (*table).Table).cast::<MIB_IF_ROW2>(),
            (*table).NumEntries as usize,
        )
    };
    let list = rows
        .iter()
        .filter_map(|r| {
            let luid = unsafe { r.InterfaceLuid.Value };
            let carries_default_route = default == Some(luid);
            (carries_default_route || listed(r)).then(|| AdapterInfo {
                alias: from_wide(&r.Alias),
                description: from_wide(&r.Description),
                carries_default_route,
            })
        })
        .collect();
    unsafe { FreeMibTable(table.cast()) };
    list
}

fn listed(r: &MIB_IF_ROW2) -> bool {
    // Bit 1 of InterfaceAndOperStatusFlags is FilterInterface: an NDIS filter (WFP, QoS) stacked on a real adapter.
    // https://learn.microsoft.com/windows/win32/api/netioapi/ns-netioapi-mib_if_row2
    let filter = r.InterfaceAndOperStatusFlags._bitfield & 0b10 != 0;
    !filter && r.Type != IF_TYPE_SOFTWARE_LOOPBACK && r.Type != IF_TYPE_TUNNEL
}

fn default_route_luid() -> Option<u64> {
    let v4 = SOCKADDR_INET {
        Ipv4: SOCKADDR_IN {
            sin_family: AF_INET,
            sin_port: 0,
            sin_addr: IN_ADDR {
                S_un: IN_ADDR_0 {
                    S_addr: u32::from_ne_bytes(ROUTE_PROBE_V4.octets()),
                },
            },
            sin_zero: [0; 8],
        },
    };
    let v6 = SOCKADDR_INET {
        Ipv6: SOCKADDR_IN6 {
            sin6_family: AF_INET6,
            sin6_addr: IN6_ADDR {
                u: IN6_ADDR_0 {
                    Byte: ROUTE_PROBE_V6.octets(),
                },
            },
            ..Default::default()
        },
    };
    [v4, v6].iter().find_map(|dest| {
        let mut route = MIB_IPFORWARD_ROW2::default();
        let mut source = SOCKADDR_INET::default();
        let status = unsafe { GetBestRoute2(null(), 0, null(), dest, 0, &mut route, &mut source) };
        (status == NO_ERROR).then_some(unsafe { route.InterfaceLuid.Value })
    })
}

fn luid_of_alias(alias: &str) -> Option<u64> {
    let wide = to_wide(alias);
    let mut luid = NET_LUID_LH::default();
    let status = unsafe { ConvertInterfaceAliasToLuid(wide.as_ptr(), &mut luid) };
    (status == NO_ERROR).then_some(unsafe { luid.Value })
}

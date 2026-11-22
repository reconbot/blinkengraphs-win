//! Shared defaults from the pinned spec (`spec/defaults.json`) with this platform's
//! `defaults.override.json` merged over them, per the spec's `rules/defaults.md`.

use serde::Deserialize;
use serde_json::Value;

const SHARED: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../spec/defaults.json"
));
const OVERRIDE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../defaults.override.json"
));

/// Merges `over` onto `base`: objects merge key by key, a `null` member removes the key,
/// and anything else replaces the base value whole.
pub fn merge(base: Value, over: Value) -> Value {
    match (base, over) {
        (Value::Object(mut b), Value::Object(o)) => {
            for (k, v) in o {
                if v.is_null() {
                    b.remove(&k);
                } else {
                    let merged = merge(b.remove(&k).unwrap_or(Value::Null), v);
                    b.insert(k, merged);
                }
            }
            Value::Object(b)
        }
        (_, over) => over,
    }
}

/// The shared defaults with this platform's overrides applied.
pub fn merged() -> Value {
    let shared = serde_json::from_str(SHARED).expect("spec/defaults.json is valid JSON");
    let over = serde_json::from_str(OVERRIDE).expect("defaults.override.json is valid JSON");
    merge(shared, over)
}

#[derive(Debug, Deserialize)]
pub struct Defaults {
    pub primary: model::Probe,
    pub probe_group: ProbeGroupDefaults,
    pub max_timeout_ms: u64,
    pub dns_query_name: String,
    pub hop_trace: HopTraceDefaults,
    pub history: HistoryDefaults,
    pub system_interval_ms: u64,
    pub appearance: crate::Mode,
    pub graph_aggregate: model::Aggregate,
    pub readout_aggregate: model::Aggregate,
    pub captive_portal_url: String,
    pub public_address: PublicAddressDefaults,
}

/// Spec `rules/public-address.md`.
#[derive(Debug, Deserialize)]
pub struct PublicAddressDefaults {
    pub dns_name: String,
    pub dns_server: String,
    /// One URL per IP version; each host has addresses of that version only, so no platform
    /// pins the address family (spec `rules/public-address.md`).
    pub https_url_ipv4: String,
    pub https_url_ipv6: String,
    pub refresh_s: u64,
}

#[derive(Debug, Deserialize)]
pub struct ProbeGroupDefaults {
    pub name: String,
    pub latency_ms: u64,
    pub dns_ms: u64,
    pub trace_ms: u64,
    pub targets: Vec<crate::TargetRow>,
}

#[derive(Debug, Deserialize)]
pub struct HopTraceDefaults {
    pub max_ttl: u8,
    pub full_sweep_every_s: u64,
    pub hop_window: usize,
    /// Show Hop names from reverse DNS (spec `rules/hop-trace.md`, "Names").
    pub reverse_dns: bool,
}

#[derive(Debug, Deserialize)]
pub struct HistoryDefaults {
    pub raw_minutes: u32,
    pub recent_window_s: u64,
    /// A1, A2, A3 in order (spec `rules/history.md`, Archives).
    pub archives: Vec<ArchiveDefaults>,
    /// How long a Pause outlasts the resume. Windows' own, from `defaults.override.json` (ADR 0019).
    pub settle_s: f64,
}

#[derive(Debug, Deserialize)]
pub struct ArchiveDefaults {
    pub step_s: u64,
    pub rows: u64,
}

pub fn get() -> Defaults {
    serde_json::from_value(merged()).expect("merged defaults match the spec's shape")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn objects_merge_key_by_key() {
        let got = merge(
            json!({"a": {"x": 1, "y": 2}, "b": 3}),
            json!({"a": {"y": 20}}),
        );
        assert_eq!(got, json!({"a": {"x": 1, "y": 20}, "b": 3}));
    }

    #[test]
    fn arrays_and_scalars_replace_whole() {
        let got = merge(
            json!({"list": [1, 2], "n": 1}),
            json!({"list": [3], "n": "x"}),
        );
        assert_eq!(got, json!({"list": [3], "n": "x"}));
    }

    #[test]
    fn null_removes_a_key_and_new_keys_are_added() {
        let got = merge(json!({"a": 1, "b": 2}), json!({"a": null, "c": 3}));
        assert_eq!(got, json!({"b": 2, "c": 3}));
    }

    #[test]
    fn empty_override_changes_nothing() {
        let base = json!({"a": {"b": [1]}});
        assert_eq!(merge(base.clone(), json!({})), base);
    }

    #[test]
    fn embedded_defaults_parse() {
        let d = get();
        assert_eq!(d.probe_group.name, "Default");
        assert!(
            d.probe_group
                .targets
                .iter()
                .any(|t| t.target.to_string() == "9.9.9.9")
        );
        let steps: Vec<u64> = d.history.archives.iter().map(|a| a.step_s).collect();
        assert_eq!(steps, [300, 3600, 86_400]);
    }
}

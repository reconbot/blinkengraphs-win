//! Every color blinkengraphs draws with, so Meters, the Strip and the Panel agree.

use model::Section;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const fn hex(v: u32) -> Self {
        Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }
}

impl From<Rgb> for String {
    fn from(c: Rgb) -> String {
        format!("#{:02x}{:02x}{:02x}", c.0, c.1, c.2)
    }
}

impl TryFrom<String> for Rgb {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        s.strip_prefix('#')
            .filter(|h| h.len() == 6)
            .and_then(|h| u32::from_str_radix(h, 16).ok())
            .map(Rgb::hex)
            .ok_or(format!("expected #rrggbb, got {s:?}"))
    }
}

impl Serialize for Rgb {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&String::from(*self))
    }
}

impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Rgb::try_from(String::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}

/// The background a color sits on: the taskbar for Meters, the Panel's mode for the Panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Shade {
    #[default]
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Preset {
    #[default]
    Default,
    HighContrast,
    /// Okabe–Ito hues, distinguishable with the common color-vision deficiencies.
    ColorBlind,
}

impl Preset {
    pub const ALL: [Preset; 3] = [Preset::Default, Preset::HighContrast, Preset::ColorBlind];

    pub fn label(self) -> &'static str {
        match self {
            Preset::Default => "Default",
            Preset::HighContrast => "High contrast",
            Preset::ColorBlind => "Color-blind safe",
        }
    }
}

/// User choices layered over a Preset.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Overrides {
    pub sections: BTreeMap<Section, Rgb>,
    /// Keyed by the Probe's display form, e.g. "icmp 1.1.1.1".
    pub probes: BTreeMap<String, Rgb>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    pub shade: Shade,
    sections: [Rgb; 5],
    pub upload: Rgb,
    pub loss: Rgb,
    pub ok: Rgb,
    pub warn: Rgb,
    pub crit: Rgb,
    series: [Rgb; 8],
    probes: BTreeMap<String, Rgb>,
    pub bg: Rgb,
    pub card: Rgb,
    pub card_edge: Rgb,
    pub faint: Rgb,
    pub field: Rgb,
    pub text: Rgb,
    pub text_weak: Rgb,
}

impl Palette {
    pub fn new(preset: Preset, shade: Shade, overrides: &Overrides) -> Self {
        let light = shade == Shade::Light;
        let h = Rgb::hex;
        let pick = |dark: u32, lite: u32| h(if light { lite } else { dark });
        let (sections, upload, loss, ok, warn, crit, series) = match preset {
            Preset::Default => (
                [
                    pick(0x4fc3f7, 0x0277bd),
                    pick(0x9ccc65, 0x388e3c),
                    pick(0x7ab8ff, 0x1565c0),
                    pick(0xb39ddb, 0x5e35b1),
                    pick(0xffd54f, 0xb26a00),
                ],
                pick(0xff9be1, 0xc2185b),
                pick(0xff3b3b, 0xc62828),
                pick(0x4ade80, 0x2e7d32),
                pick(0xffb340, 0xd97706),
                pick(0xff5a5a, 0xd32f2f),
                [
                    pick(0xffd54f, 0xb26a00),
                    pick(0x4fc3f7, 0x0277bd),
                    pick(0xf48fb1, 0xc2185b),
                    pick(0x9ccc65, 0x388e3c),
                    pick(0xb39ddb, 0x5e35b1),
                    pick(0xff8a65, 0xd84315),
                    pick(0x4dd0e1, 0x00838f),
                    pick(0xe0e0e0, 0x424242),
                ],
            ),
            Preset::HighContrast => (
                [
                    pick(0x00e5ff, 0x005b9f),
                    pick(0x76ff03, 0x1b5e20),
                    pick(0x82b1ff, 0x0d47a1),
                    pick(0xea80fc, 0x6a1b9a),
                    pick(0xffff00, 0x8d6e00),
                ],
                pick(0xff4081, 0xad1457),
                pick(0xff1744, 0xb71c1c),
                pick(0x00e676, 0x1b5e20),
                pick(0xffab00, 0xbf360c),
                pick(0xff1744, 0xb71c1c),
                [
                    pick(0xffff00, 0x8d6e00),
                    pick(0x00e5ff, 0x005b9f),
                    pick(0xff4081, 0xad1457),
                    pick(0x76ff03, 0x1b5e20),
                    pick(0xea80fc, 0x6a1b9a),
                    pick(0xff9100, 0xbf360c),
                    pick(0x18ffff, 0x006064),
                    pick(0xffffff, 0x000000),
                ],
            ),
            // https://jfly.uni-koeln.de/color/ (Okabe & Ito)
            Preset::ColorBlind => (
                [
                    h(0x56b4e9),
                    h(0x009e73),
                    h(0x0072b2),
                    h(0xcc79a7),
                    pick(0xf0e442, 0xb8860b),
                ],
                h(0xd55e00),
                h(0xd55e00),
                h(0x009e73),
                h(0xe69f00),
                h(0xd55e00),
                [
                    pick(0xf0e442, 0xb8860b),
                    h(0x56b4e9),
                    h(0xcc79a7),
                    h(0x009e73),
                    h(0x0072b2),
                    h(0xe69f00),
                    h(0xd55e00),
                    pick(0xe0e0e0, 0x424242),
                ],
            ),
        };
        let mut p = Palette {
            shade,
            sections,
            upload,
            loss,
            ok,
            warn,
            crit,
            series,
            probes: overrides.probes.clone(),
            bg: pick(0x12151c, 0xf3f4f7),
            card: pick(0x1b202a, 0xffffff),
            card_edge: pick(0x2a313d, 0xd9dde4),
            faint: pick(0x202631, 0xeef0f4),
            field: pick(0x0e1116, 0xe8ebf0),
            text: pick(0xe6eaf0, 0x1b1f27),
            text_weak: pick(0x8b95a5, 0x5b6473),
        };
        for (s, c) in &overrides.sections {
            p.sections[index(*s)] = *c;
        }
        p
    }

    pub fn section(&self, s: Section) -> Rgb {
        self.sections[index(s)]
    }

    /// A Probe's chosen color, else the `i`th series color.
    pub fn probe(&self, probe: &str, i: usize) -> Rgb {
        self.probes
            .get(probe)
            .copied()
            .unwrap_or(self.series[i % self.series.len()])
    }

    pub fn series(&self, i: usize) -> Rgb {
        self.series[i % self.series.len()]
    }
}

fn index(s: Section) -> usize {
    Section::ALL.iter().position(|x| *x == s).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips() {
        let c = Rgb::try_from("#4fc3f7".to_string()).unwrap();
        assert_eq!(c, Rgb(0x4f, 0xc3, 0xf7));
        assert_eq!(String::from(c), "#4fc3f7");
        assert!(Rgb::try_from("4fc3f7".to_string()).is_err());
    }

    #[test]
    fn default_dark_matches_spec_tokens() {
        let p = Palette::new(Preset::Default, Shade::Dark, &Overrides::default());
        assert_eq!(p.section(Section::Cpu), Rgb::hex(0x4fc3f7));
        assert_eq!(p.section(Section::Latency), Rgb::hex(0xffd54f));
        assert_eq!(p.crit, Rgb::hex(0xff5a5a));
    }

    #[test]
    fn overrides_win() {
        let mut o = Overrides::default();
        o.sections.insert(Section::Cpu, Rgb::hex(0x123456));
        o.probes.insert("icmp 1.1.1.1".into(), Rgb::hex(0xabcdef));
        let p = Palette::new(Preset::ColorBlind, Shade::Light, &o);
        assert_eq!(p.section(Section::Cpu), Rgb::hex(0x123456));
        assert_eq!(p.probe("icmp 1.1.1.1", 3), Rgb::hex(0xabcdef));
        assert_eq!(p.probe("icmp 8.8.8.8", 1), p.series(1));
    }
}

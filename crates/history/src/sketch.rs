//! Per-minute log-linear histogram (layout "BG-LL5", docs/research/06-quantile-sketch.md).
//!
//! Index and representative values use only bit extraction and `+ − × ÷`, so every platform
//! computes identical bits; `ln` has unspecified precision in Rust and differs across libms.

use std::collections::BTreeMap;

/// Sub-buckets per power of two: relative error ≤ 1/65 ≈ 1.54 %.
const SUB: i64 = 32;
const MIN_EXP: i64 = -10;
pub const BUCKETS: u16 = 1600;

/// Bucket index for `v`, or `None` for the zero bucket (`v < 2^-10`, including 0 and negatives).
pub fn index(v: f64) -> Option<u16> {
    if v.is_nan() || v < 1.0 / 1024.0 {
        return None;
    }
    let b = v.to_bits();
    let e = ((b >> 52) & 0x7ff) as i64 - 1023;
    let m = ((b >> 47) & 31) as i64;
    Some(((e - MIN_EXP) * SUB + m).clamp(0, BUCKETS as i64 - 1) as u16)
}

/// Harmonic midpoint of the bucket, `2·lo·hi/(lo+hi)`, as in DDSketch.
pub fn rep(i: u16) -> f64 {
    let e = i as i64 / SUB + MIN_EXP;
    let m = i as i64 % SUB;
    let unit = pow2(e - 5);
    let lo = (32 + m) as f64 * unit;
    let hi = (33 + m) as f64 * unit;
    2.0 * lo * hi / (lo + hi)
}

fn pow2(e: i64) -> f64 {
    f64::from_bits(((e + 1023) as u64) << 52)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sketch {
    pub zero: u64,
    pub counts: BTreeMap<u16, u64>,
}

impl Sketch {
    pub fn add(&mut self, v: f64) {
        match index(v) {
            None => self.zero += 1,
            Some(i) => *self.counts.entry(i).or_default() += 1,
        }
    }

    /// Integer addition, so any merge order gives the same sketch.
    pub fn merge(&mut self, o: &Sketch) {
        self.zero += o.zero;
        for (i, c) in &o.counts {
            *self.counts.entry(*i).or_default() += c;
        }
    }

    pub fn n(&self) -> u64 {
        self.zero + self.counts.values().sum::<u64>()
    }

    /// The value at rank `⌊per_mille·(n−1)/1000⌋`, clamped to the exact `[min, max]`. No interpolation.
    pub fn quantile(&self, per_mille: u64, min: f64, max: f64) -> Option<f64> {
        let n = self.n();
        if n == 0 {
            return None;
        }
        let r = per_mille * (n - 1) / 1000;
        if r < self.zero {
            return Some(0f64.clamp(min, max));
        }
        let mut seen = self.zero;
        for (i, c) in &self.counts {
            seen += c;
            if seen > r {
                return Some(rep(*i).clamp(min, max));
            }
        }
        unreachable!("rank below n")
    }

    /// `uleb128(zero)`, then per non-empty bucket ascending `uleb128(gap)`, `uleb128(count)`,
    /// where gap is `index − previous − 1` and previous starts at −1.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        leb(&mut out, self.zero);
        let mut prev: i64 = -1;
        for (i, c) in &self.counts {
            leb(&mut out, (*i as i64 - prev - 1) as u64);
            leb(&mut out, *c);
            prev = *i as i64;
        }
        out
    }

    pub fn decode(mut b: &[u8]) -> Option<Sketch> {
        let mut s = Sketch {
            zero: unleb(&mut b)?,
            ..Sketch::default()
        };
        let mut prev: i64 = -1;
        while !b.is_empty() {
            let i = prev + 1 + unleb(&mut b)? as i64;
            let c = unleb(&mut b)?;
            if i >= BUCKETS as i64 {
                return None;
            }
            s.counts.insert(i as u16, c);
            prev = i;
        }
        Some(s)
    }
}

fn leb(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn unleb(b: &mut &[u8]) -> Option<u64> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let (&byte, rest) = b.split_first()?;
        *b = rest;
        v |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some(v);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_boundaries() {
        let below = f64::from_bits((1.0f64 / 1024.0).to_bits() - 1);
        assert_eq!(index(1.0 / 1024.0), Some(0));
        assert_eq!(index(below), None);
        assert_eq!(index(0.0), None);
        assert_eq!(index(-1.0), None);
        assert_eq!(index(1.0), Some(320));
        assert_eq!(index(f64::from_bits(1f64.to_bits() - 1)), Some(319));
        assert_eq!(index(33.0 / 32.0), Some(321));
        assert_eq!(index(2f64.powi(40)), Some(BUCKETS - 1));
        assert_eq!(index(1e30), Some(BUCKETS - 1));
    }

    #[test]
    fn rep_within_bucket_and_error_bound() {
        for i in [0u16, 1, 319, 320, 321, 1000, BUCKETS - 2] {
            let r = rep(i);
            assert_eq!(index(r), Some(i), "rep({i}) = {r} lands in its bucket");
        }
        let mut worst = 0f64;
        let mut v = 0.002;
        while v < 1e10 {
            let err = (rep(index(v).unwrap()) - v).abs() / v;
            worst = worst.max(err);
            v *= 1.0137;
        }
        assert!(worst <= 1.0 / 65.0 + 1e-12, "{worst}");
    }

    #[test]
    fn merge_is_order_free_and_round_trips() {
        let mk = |vals: &[f64]| {
            let mut s = Sketch::default();
            vals.iter().for_each(|v| s.add(*v));
            s
        };
        let a = mk(&[0.0, 1.0, 12.5, 12.6, 300.0]);
        let b = mk(&[5.0, 5.0, 1e9]);
        let c = mk(&[0.5, 80.0]);
        let mut x = a.clone();
        x.merge(&b);
        x.merge(&c);
        let mut y = c.clone();
        y.merge(&a);
        y.merge(&b);
        assert_eq!(x, y);
        assert_eq!(Sketch::decode(&x.encode()), Some(x.clone()));
        assert_eq!(x.n(), 10);
    }

    #[test]
    fn quantiles_use_lower_rank_and_clamp() {
        let mut s = Sketch::default();
        for v in 1..=100 {
            s.add(v as f64);
        }
        let q = |pm| s.quantile(pm, 1.0, 100.0).unwrap();
        let close = |a: f64, b: f64| (a - b).abs() / b <= 1.0 / 65.0;
        assert!(close(q(500), 50.0), "{}", q(500));
        assert!(close(q(950), 95.0), "{}", q(950));
        assert!(close(q(990), 99.0), "{}", q(990));
        assert_eq!(q(1000), 100.0);
        assert_eq!(Sketch::default().quantile(500, 0.0, 0.0), None);
    }
}

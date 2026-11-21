# 06 — A mergeable quantile sketch for per-minute rollups

Researched 2026-11-21. Question: what should `crates/history` store per minute per key so that graph buckets over 1 h, 24 h and 7 d (1 to ~25 minutes each) can show **p50, p95, p99, avg and max** accurately? The Swift apps must reproduce every result bit for bit from a written rule and JSON test vectors.

Constraints:
- [ADR 0007](../adr/0007-footprint-is-a-feature.md): exe ≤ 7 MiB, working set < 40 MB, CPU < 1%. A new dependency must justify its bytes.
- Today `rollup` holds min, mean, max and loss per `(key TEXT, minute)` for 7 days. RAM holds raw 1 s samples for 15 min (`crates/history/src/lib.rs`).
- At most 60 samples per key per minute. Values are ≥ 0: CPU %, bytes/s (0 to 1e10), ms, counts, RTT ms. A lost sample is `None`, never a value.

Citations such as [S1] point to Sources at the end. Markers as in 02 and 04:
- **(verified)**: read in a primary source.
- **(inferred)**: my reasoning from the sources.
- **(measured)**: from the simulation in §3.
- **(low confidence)**: weak or missing evidence.

---

## 1. The decisive constraint: determinism across languages

- Merging is only order-independent if it is integer addition over a *fixed* bucket layout. DDSketch: "any two sketches using the same value for γ can be merged by simply summing up the buckets that share an index" [S1] (verified). Integer addition is associative and commutative, so the merge order cannot change the result (inferred).
- t-digest cannot meet this. Its authors say a merged digest "will not necessarily be the same as if we had computed a t-digest tX∪Y from all of the original data at once", and that the resulting loss of ordering "makes it difficult to compute rigorous error bounds" [S7] (verified). ClickHouse documents its t-digest as "nondeterministic", with a result that "depends on the order of running the query" [S12] (verified).
- **Logarithms are not reproducible.** Rust documents `f64::ln`/`exp`/`powf` with "The precision of this function is non-deterministic. This means it varies by platform, Rust version, and can even differ within the same execution" [S9] (verified). OpenTelemetry's exponential histogram allows a mapping "with an expected difference of at most 1 from the correct result" and says "Defining an exact mapping function is out of scope" [S8] (verified). So any index computed with `ln(v)/ln(γ)`, which covers DDSketch's logarithmic mapping [S2] and OTel at positive scales [S8], can put a value near a boundary into a different bucket in Rust and in Swift (inferred). Test vectors would then fail at random.
- **Basic arithmetic is reproducible.** Rust rounds `+ − × ÷` with IEEE 754 roundTiesToEven [S10]. Swift's `FloatingPoint` "implements the addition operation defined by the IEEE 754 specification", and likewise for the other basic operations [S11] (verified). A rule that uses only bit extraction, integer arithmetic and `+ − × ÷` therefore produces identical bits in both languages (inferred).

**Consequence:** take the bucket index from the float's exponent and mantissa bits (log-linear, as in HDR Histogram and the paper's "DDSketch (fast)" [S1][S4]), not from a logarithm.

## 2. Options compared

Size figures are from §3 (20 keys × 7 d, SQLite after `VACUUM`).

| Option | Bytes/min/key on disk | 20 keys × 7 d | Error on p50/p95/p99 per graph bucket | Merge order-independent? | Spec in prose / Swift |
|---|---|---|---|---|---|
| Today (min, mean, max, loss) | 46 | 8.8 MB | No quantiles | Yes | Trivial |
| **Raw samples** (60 × f32 blob) | 257 | 49 MB | Exact | Yes (concatenate, sort) | Trivial; 5.6× today's disk |
| **Per-minute p50/p95/p99** + min/mean/max | 57 | 11 MB | Exact at 1-min buckets. Over 15-min buckets, max-of-p95 overshoots: median error 19–596% depending on key class, worst ×25 934 (bytes/s). Mean-of-p95 is no better (§3) | Yes, but the combined answer is wrong | Trivial |
| **Log-linear fixed layout, 32 sub-buckets/octave** (recommended) | 94 (blob mean 56 B) | 18 MB | ≤ 1.54% relative, any bucket width; measured max 1.54% | **Yes**, integer counts | ~80 lines; bit ops only |
| Log-linear, 16 sub-buckets/octave | 79 (blob 44 B) | 15 MB | ≤ 3.03%; measured max 3.03% | Yes | Same rule |
| DDSketch α = 1%, log mapping, sparse | 96 | 18 MB | ≤ 1% [S1] | Counts yes; *index* depends on `ln` [S9] | Needs `ln`, so vectors may flake near boundaries |
| OTel exponential histogram, scale 5 (base 2^(1/32)) | ≈ DDSketch (inferred) | ≈ 18 MB | (b−1)/(b+1) ≈ 1.08% (inferred from base [S8]) | Counts yes; index ±1 allowed [S8] | Exact only at scale ≤ 0 or for powers of two [S8] |
| HDR Histogram | Dense counts; V2 encoding is compressed (not measured) | — | "Value quantization … no larger than 1/1,000th" at 3 significant digits [S4] | Yes [S1] | Integers only [S4]: scale each unit to an integer (µs, bytes, 0.01 %) |
| t-digest (compression 100) | Up to ~100 centroids × (mean, weight) (inferred) | — | No relative-error bound [S1][S7] | **No** [S7][S12] | Hard: the scale function, buffering and ties all need specifying |

### 2.1 DDSketch [S1][S2]
- **Layout:** γ = (1+α)/(1−α). Bucket i counts γ^(i−1) < x ≤ γ^i, and the bucket's value is 2γ^i/(γ+1). That is α-accurate for every quantile, with "full" mergeability [S1] (verified).
- **Collapsing:** past m buckets the lowest are merged together. "For α = 0.01, a sketch of size 2048 can handle values from 80 microseconds to 1 year" [S1] (verified). Each minute holds ≤ 60 samples and the value range is bounded, so collapsing never triggers here (inferred).
- **Zeros and negatives:** the paper keeps "a special bucket for zero" and a mirrored sketch for negatives [S1] (verified). We need only the zero bucket.
- **Rust crate:** `sketches-ddsketch` 0.4.2 (2026-10-25), Apache-2.0, 16 KB crate, only optional `serde` [S3]. Defaults: α = 0.01, 2048 bins, min 1e-9. It indexes with `floor(ln(v)/ln γ)` to match Java, *not* the paper's ceiling [S2] (verified). Its `sum` is an f64 running total, so the merge order changes its last bits (inferred).
- **Swift:** no DDSketch port found. Datadog ships Go, Java, Python, JS and Rust [S20] (**low confidence**: one search).

### 2.2 HDR Histogram [S4][S5][S6]
- **Layout:** "exponentially increasing bucket value ranges (the parallel of the exponent portion of a floating point number) with each bucket containing a fixed number … of linear sub-buckets" [S4] (verified). This is the log-linear idea, but over **integers** with a configured `highestTrackableValue` [S4].
- **Reading a quantile:** the Rust crate takes rank `ceil(q·n)` and returns the bucket's *highest* equivalent value [S5] (verified). That deliberately overstates, by up to one bucket width (inferred).
- **Rust crate:** `hdrhistogram` 7.6.0 (2026-07-18), MIT/Apache-2.0, 323 KB crate. Default features pull in `flate2`, `nom`, `base64` and `crossbeam-channel` [S3] (verified). That is heavy for ADR 0007, because every feature brings its own dependencies.
- **Swift:** `HdrHistogram/hdrhistogram-swift` exists, Apache-2.0, "a subset of the functionality supported by the Java implementation" [S6] (verified).
- **Fit:** the layout idea fits; the library does not. Integer scaling per unit and a dense array per minute are wasted work when a minute holds 60 samples (inferred).

### 2.3 t-digest [S7], Redis [S18][S19], ClickHouse [S12]
- **Not order-independent**, as §1 shows. Rejected.
- **Redis Stack:** `TDIGEST.CREATE … [COMPRESSION]`, where "100 is a common value … and also the default". `TDIGEST.MERGE` is O(N·K) over centroids [S19] (verified). Redis does not document determinism or the serialized size in the pages read (**low confidence** on both). Its format is not worth copying: the merge result depends on the order of inputs [S7].
- **Rust crate:** `tdigest` 1.0.1 (2026-09-07), Apache-2.0 [S3].

### 2.4 ClickHouse aggregate states (prior art) [S12]–[S17]
| Function | Algorithm | Deterministic | State size | Worth copying? |
|---|---|---|---|---|
| `quantile` | Reservoir sampling, up to 8192, with an RNG. "The result is non-deterministic" [S16] | No | ≤ 8192 values | No |
| `quantileTDigest` | t-digest. "nondeterministic" [S12] | No | "log(n)" [S12] | No |
| `quantileDD` | DDSketch. Size ≈ log(max/min)/relative_accuracy [S13] | Not stated (**low confidence**) | As stated | Layout yes, `ln` index no (§1) |
| `quantileTiming` | Exact when there are ≤ 5670 values or values < 1024 ms; otherwise rounded to 16 ms; clamped at 30 000 [S14] | **Yes**: "does not depend on the query processing order" [S14] | Not documented (**low confidence**) | Its idea is: fixed buckets give determinism. Its linear 16 ms buckets do not fit bytes/s |
| `quantileExact` | All values, partially sorted; O(n) memory [S15] | Yes | O(n) | Equals the raw option |

`-State` stores "an intermediate state of the aggregation", and `-Merge` "combines the states to finish aggregation" [S17] (verified). That is the shape we need: store a state per minute and merge at read time. The binary layouts of these states are not documented as a portable format in the pages read (**low confidence**). Do not copy them (inferred).

### 2.5 Exact alternatives
- **Raw for 7 d:** 49 MB on disk (measured), 5.6× today. Every 7-day graph sorts about 600 k values per key (inferred). Exact, but it spends disk to avoid an 80-line rule.
- **Per-minute percentiles:** you cannot get a merged percentile from stored percentiles. Max-of-p95 overstates badly when one minute bursts (§3). The current code's `percentile()` helper would be reused, but the answer is wrong at every zoom wider than 1 minute.

## 3. Measurement (synthetic, inferred realism)

The script lives in the session scratchpad and is not committed. Method:
- 20 keys × 10 080 minutes × 60 samples, f32:
  - 3 × % (diurnal, lognormal noise, 1% spikes);
  - 4 × bytes/s (20% zeros, idle minutes, 2% bursts to ~1e8);
  - 3 × ms (40% idle minutes);
  - 2 × Poisson counts with rare 5000 spikes;
  - 8 × RTT (lognormal around 0.4–150 ms, 3% bloat ×2–20, 1% loss).
- Tables `WITHOUT ROWID`, page size 4096, `VACUUM`ed. Python 3 sqlite 3.53.1.
- Error is |estimate − exact| / exact, with the exact value taken by the same rank rule on the raw samples.

Results (measured):
- **Non-empty buckets per minute** at 32/octave: mean 27, p95 50, max 59. At 16/octave: mean 20.
- **Log-linear sketch error:** median 0.55%, max 1.54% for every key class, every bucket width (1, 4, 15 min) and p50/p95/p99. That equals the analytic bound in §4.
- **Max-of-p95 over 15-min buckets,** median and worst error:

  | Key class | Median | Worst |
  |---|---|---|
  | % | 19% | ×9 |
  | RTT | ×3.7 | ×13 |
  | ms | 78% | ×2.7 |
  | bytes/s | ×6 | ×25 934 |

  Mean-of-p95 is also off, by 1–29% median and up to ×2390.
- **Read CPU,** Rust std-only, `opt-level = "s"`, Xeon 2.1 GHz. Decoding blobs into a dense `[u32; 1600]`, then reading p50/p95/p99, takes:
  - 1 h at 1-min buckets: **0.27 ms** per key;
  - 24 h at 4-min buckets: **1.1 ms**;
  - 7 d at 15-min buckets: **3.1 ms**.

  SQLite read time is excluded.

Synthetic data is noisier than a calm machine, so real sketches should be smaller (**low confidence** until measured on real rollups).

## 4. Recommendation

**Store a fixed-layout, log-linear histogram per key per minute, plus exact n, lost, sum, min and max.** Write it from scratch: about 80 lines in Rust and in Swift. Add no crate.

### 4.1 Bucket layout ("BG-LL5")
- **Input:** a sample `v`. Use the f32 as recorded, widened to f64; widening is exact (inferred). NaN is never recorded, because a lost sample increments `lost`.
- **Zero bucket:** `v < 2^-10` (≈ 0.000977), including 0, −0, negatives and subnormals. In ms that is under 1 µs; in % it is under 0.001%.
- **Otherwise,** with `b = v.to_bits()` (Swift `v.bitPattern`):
  - `e = ((b >> 52) & 0x7FF) − 1023` (Swift: `v.exponent`)
  - `m = (b >> 47) & 31`, the top 5 mantissa bits
  - `index = clamp((e + 10) · 32 + m, 0, 1599)`
- **Range:** [2^-10, 2^40) ≈ [0.001, 1.1e12]. That covers RTT 0.1 ms–60 s, %, counts and bytes/s up to 1e10 in **one layout for every unit**, so no per-unit tables are needed. Larger values land in bucket 1599, and the exact `max` column still holds them.
- **Bucket bounds,** both exact in binary64 (inferred: the values are dyadic):
  - `lo = (32 + m) · 2^(e−5)`
  - `hi = (33 + m) · 2^(e−5)`
- **Representative value:** `rep = 2·lo·hi / (lo + hi)`. This is DDSketch's `2γ^i/(γ+1)` with γ = hi/lo [S1], computed with `× + ÷` only. Relative error ≤ (hi−lo)/(hi+lo) = 1/(65 + 2m) ≤ **1.54%** (inferred; measured max 1.54%).
- **Resolution:** 32 sub-buckets per octave, 1 600 indices, so an index fits in two varint bytes. 16/octave saves about 15% of disk and doubles the error to 3.03% (measured). Choose 32.

### 4.2 SQLite encoding
```sql
CREATE TABLE key (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE);
CREATE TABLE rollup (
  key INTEGER NOT NULL, minute INTEGER NOT NULL,   -- unix seconds, minute-aligned
  n INTEGER NOT NULL, lost INTEGER NOT NULL,       -- samples recorded / lost
  sum REAL, min REAL, max REAL,                    -- NULL when n = 0
  sk BLOB NOT NULL,                                -- see below; empty when n = 0
  PRIMARY KEY (key, minute)) WITHOUT ROWID;
```
- **`sk`:**
  - `uleb128(zero_count)`, then for each non-empty bucket in ascending index order, `uleb128(index − prev − 1)` followed by `uleb128(count)`, with `prev` starting at −1.
  - Canonical: no empty buckets. That makes the blob bytes themselves a test vector.
  - Bump `PRAGMA user_version` if the layout ever changes, instead of spending a version byte in every row.
- **`sum`:** the f64 sum of the minute's samples, added in arrival order.
- **Integer `key`:** stops repeating the key text in every row.

### 4.3 Aggregates for a graph bucket of minutes M
- **Merge:** add `zero_count` and the per-index counts as integers, in any order; the result is identical (inferred).
- **n:** Σn. **Loss** = Σlost / Σ(n + lost).
- **max / min:** the max of `max` and min of `min`. Exact.
- **avg:** Σsum / Σn. Add the sums in **ascending minute order**, because f64 addition is not associative.
- **pX for X in per-mille (500, 950, 990):**
  1. Rank `r = ⌊X·(Σn − 1) / 1000⌋` in u64 integer arithmetic. This is the paper's lower quantile `⌊1 + q(n−1)⌋` [S1], 0-based.
  2. If `r < zero_count`, return `0`.
  3. Otherwise walk the buckets upward, accumulating counts. At the first bucket where the cumulative count exceeds `r`, return `clamp(rep, min, max)`.
  4. No interpolation, so every output is a bit-exact f64 for the vectors.
- **1 h views** may serve the newest 15 min from the raw RAM ring, applying the same rank rule to sorted raw values. Vectors should then also cover "raw vs sketch differ by ≤ 1.54%".

### 4.4 Test vectors to add to the spec repo
- Index boundaries: `2^-10`, `nextDown(2^-10)`, `1.0`, `nextDown(1.0)`, `33/32`, `nextDown(33/32)`, `1e10`, `2^40`, `0`, `−1`.
- Blob bytes for a few minutes.
- Merged quantiles for a multi-minute bucket, written as f64 hex bits.
- One vector that merges the same minutes in two different orders.

### 4.5 Expected cost
- **Disk:** about 94 B per row on noisy synthetic data, so **≈ 18 MB for 20 keys × 7 d**, versus 8.8 MB today and 49 MB for raw (measured). Real data should be smaller (**low confidence**).
- **Write CPU:** once a minute per key, sort ≤ 60 values or count them into ≤ 60 buckets. Negligible (inferred).
- **Read CPU:** ≈ 3 ms per key for a 7-day view, ≈ 60 ms for all 20 keys, only while the Panel is open (measured, plus SQLite I/O). Cache per bucket so a redraw does not recompute it.
- **Exe:** no dependency. Well under 10 KB of code (inferred).
- **RAM:** one `[u32; 1600]` (6.4 KB) per bucket being merged (inferred).

---

## Sources
- [S1] Masson, Rim, Lee. "DDSketch: A Fast and Fully-Mergeable Quantile Sketch with Relative-Error Guarantees", PVLDB 12(12), 2019: https://arxiv.org/abs/1908.10693 (PDF https://arxiv.org/pdf/1908.10693). Read: Def. 1, §2.1 Lemma 2, Alg. 3–4, §2.2, the "DDSketch (fast)" mapping in §4, related work on HDR and t-digest.
- [S2] sketches-ddsketch source, `src/config.rs` and `src/ddsketch.rs`: https://github.com/mheffner/rust-sketches-ddsketch/blob/master/src/config.rs
- [S3] crates.io API, versions, licenses, sizes and dependencies (read 2026-11-21): https://crates.io/crates/sketches-ddsketch , https://crates.io/crates/hdrhistogram , https://crates.io/crates/tdigest
- [S4] HdrHistogram README (internals, footprint, integer range): https://github.com/HdrHistogram/HdrHistogram
- [S5] HdrHistogram_rust `value_at_quantile`: https://github.com/HdrHistogram/HdrHistogram_rust/blob/master/src/lib.rs
- [S6] hdrhistogram-swift: https://github.com/HdrHistogram/hdrhistogram-swift , https://swiftpackageindex.com/HdrHistogram/hdrhistogram-swift
- [S7] Dunning, Ertl. "Computing Extremely Accurate Quantiles Using t-Digests", §"Merging independent t-digests": https://arxiv.org/abs/1902.04023
- [S8] OpenTelemetry metrics data model, ExponentialHistogram (scale, mapping, producer expectations): https://github.com/open-telemetry/opentelemetry-specification/blob/main/specification/metrics/data-model.md#exponentialhistogram
- [S9] Rust `f64::ln`, "Unspecified precision": https://doc.rust-lang.org/std/primitive.f64.html#method.ln (source: https://github.com/rust-lang/rust/blob/master/library/std/src/num/f64.rs)
- [S10] Rust `f64` primitive docs, roundTiesToEven for `+ − × ÷`: https://doc.rust-lang.org/std/primitive.f64.html
- [S11] Swift stdlib `FloatingPoint` (IEEE 754 basic operations): https://github.com/swiftlang/swift/blob/main/stdlib/public/core/FloatingPoint.swift , https://developer.apple.com/documentation/swift/floatingpoint
- [S12] ClickHouse quantileTDigest: https://clickhouse.com/docs/sql-reference/aggregate-functions/reference/quantiletdigest
- [S13] ClickHouse quantileDD: https://clickhouse.com/docs/sql-reference/aggregate-functions/reference/quantileddsketch
- [S14] ClickHouse quantileTiming: https://clickhouse.com/docs/reference/functions/aggregate-functions/quantileTiming
- [S15] ClickHouse quantileExact: https://clickhouse.com/docs/reference/functions/aggregate-functions/quantileExact
- [S16] ClickHouse quantile: https://clickhouse.com/docs/sql-reference/aggregate-functions/reference/quantile
- [S17] ClickHouse aggregate function combinators (-State, -Merge): https://clickhouse.com/docs/sql-reference/aggregate-functions/combinators
- [S18] Redis t-digest overview: https://redis.io/docs/latest/develop/data-types/probabilistic/t-digest/
- [S19] Redis TDIGEST.CREATE and TDIGEST.MERGE: https://redis.io/docs/latest/commands/tdigest.create/ , https://redis.io/docs/latest/commands/tdigest.merge/
- [S20] Datadog sketches implementations (no Swift found): https://github.com/DataDog/sketches-go , https://github.com/DataDog/sketches-java , https://github.com/DataDog/sketches-py , https://github.com/DataDog/sketches-js
- [S21] SQLite record format (REAL is 8 bytes; integers 1–8 bytes): https://www.sqlite.org/fileformat2.html#record_format

### Known gaps
- **Not measured:** real-machine rollups. Sizes in §3 come from synthetic data.
- **Not documented in the pages read:** the serialized sizes and binary formats of the ClickHouse and Redis states; whether `quantileDD` is deterministic.
- **Not verified:** that no Swift DDSketch exists (one search). Apple libm's `log` accuracy; it is moot, because the recommended rule never calls it.

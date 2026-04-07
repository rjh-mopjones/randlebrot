---
issue: 43
title: Split fBm into coarse + detail at micro scale
crates: [rb_noise]
modifies:
  - crates/rb_noise/src/derived/mod.rs
  - crates/rb_noise/src/biome_map.rs
  - src/commands/launch.rs
removes:
  - src/commands/launch.rs::build_local_heightmap  # hack to delete
depends_on: []
---

## Goal

Fix flat heightmap at micro/chunk scale. The fBm's high-frequency octaves exist but normalization crushes them to 0.07% of output. Split the read: coarse shape from octaves 0-11, local texture from octaves 12+, normalize independently, combine with terrain-type amplitude budget.

## Root Cause

The fBm loop normalizes by total amplitude across ALL octaves. At persistence 0.59, octave 12+ contributes <0.1% after normalization:

| Octave | Cycles/chunk | Amplitude | % of output |
|--------|-------------|-----------|-------------|
| 0      | 0.01        | 1.000     | 42.0%       |
| 12     | 40.96       | 0.002     | 0.07%       |
| 16     | 655.36      | 0.0002    | 0.008%      |

Diagnostic evidence (`randlebrot debug-level`):
```
(512,256): heightmap span 0.013, unique blocks @500x: 7
(200,180): heightmap span 0.005, unique blocks @500x: 3
```

## Implementation

### 1. New function in `crates/rb_noise/src/derived/mod.rs`

```rust
use noise::{NoiseFn, OpenSimplex};

pub fn derive_micro_heightmap(
    base_heightmap: f64,
    wx: f64,
    wy: f64,
    detail_noise: &OpenSimplex,  // created ONCE outside pixel loop
) -> f64 {
    let start_freq = 0.01 * 2.0_f64.powi(12); // octave 12 = 40.96
    let mut value = 0.0;
    let mut amp = 1.0;
    let mut freq = start_freq;
    let mut max_amp = 0.0;

    for _ in 0..6 { // octaves 12-17
        value += detail_noise.get([wx * freq, wy * freq]) * amp;
        max_amp += amp;
        amp *= 0.5;
        freq *= 2.0;
    }
    let detail = value / max_amp; // [-1, 1] independently

    // Thresholds from actual heightmap data (seed 42):
    let budget = if base_heightmap > 0.03 { 0.25 }       // mountains
                 else if base_heightmap > -0.01 { 0.15 }  // hills/coast
                 else if base_heightmap > -0.025 { 0.10 }  // plains
                 else { 0.05 };                             // ocean/frozen

    (base_heightmap + detail * budget).clamp(-1.0, 1.0)
}
```

### 2. Wire into `crates/rb_noise/src/biome_map.rs`

Outside pixel loop (near other strategy constructions):
```rust
let detail_noise = OpenSimplex::new(seed.wrapping_add(50));
```

Inside pixel loop, after `let hm = derived::derive_heightmap(...)`:
```rust
let hm = if detail_level >= 3 {
    derived::derive_micro_heightmap(hm, wx, wy, &detail_noise)
} else {
    hm
};
```

### 3. Remove hack from `src/commands/launch.rs`

Delete `build_local_heightmap` function entirely. Replace all calls:
```rust
// Delete: let heightmap = build_local_heightmap(&biome_map);
// Replace: let heightmap = biome_map.heightmap.clone();
```

## Verification

```bash
# Before fix:
randlebrot debug-level mountains-test | grep "heightmap:"
# heightmap: span: 0.004549, unique blocks (@500x): 3

# After fix — regenerate level and check:
randlebrot generate level my-world 200,180 mountains-test --force
randlebrot debug-level mountains-test | grep "heightmap:"
# Expected: span > 0.05, unique blocks (@500x) > 20
```

## Boundary Test

```rust
#[test]
fn chunk_boundary_heights_match() {
    // Generate two adjacent chunks
    let bm_a = BiomeMap::generate_meso_full_with_backend(
        42, 200.0, 180.0, 1.0, 512, 512.0, 3, None, Cpu, None, None);
    let bm_b = BiomeMap::generate_meso_full_with_backend(
        42, 201.0, 180.0, 1.0, 512, 512.0, 3, None, Cpu, None, None);

    // Chunk A's rightmost column == Chunk B's leftmost column
    // (same world coordinates, different chunks)
    let a_edge = bm_a.heightmap[511]; // x=511, y=0 in chunk A
    let b_edge = bm_b.heightmap[0];   // x=0, y=0 in chunk B
    assert!((a_edge - b_edge).abs() < 1e-10, "chunk boundary seam: {a_edge} vs {b_edge}");
}
```

## Constraints

- Macro/meso views (detail_level < 3) MUST produce identical output
- World Rules tests must pass: `nothing_green_above_45c`, `no_vegetation_*`
- Same seed + same coordinates = same height (deterministic)
- OpenSimplex created ONCE, not per-pixel

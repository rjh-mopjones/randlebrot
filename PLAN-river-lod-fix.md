# Fix: Rivers Multiply Unnaturally When Zooming In

## Problem

When scrolling/zooming in from macro → meso → micro, rivers proliferate instead of gaining detail. Each zoom level generates an **independent river network** from scratch on its local heightmap patch, producing entirely new rivers rather than refining existing ones.

### Root Cause

`generate_region_with_sub_stellar()` (biome_map.rs:799) creates a fresh `RiverGenerator` and runs D8 flow accumulation on just the local 512×512 heightmap — with no knowledge of the macro-level river network. The accumulation threshold `0.0005 × total_pixels` stays constant relative to grid size, so each zoom level produces a similar *density* of rivers, but they're all different rivers.

Meanwhile, `generate_meso_full()` (biome_map.rs:1005) already has the right pattern: it accepts `macro_map: Option<&BiomeMap>`, uses `carve_river_channels()` to stamp macro rivers into the meso heightmap, and calls `generate_with_macro_flow_climate()` to seed D8 accumulation from parent rivers. But `generate_region_with_sub_stellar()` doesn't use any of this infrastructure.

### Affected Code Paths

| Method | File:Line | Has macro river seeding? | Used by |
|--------|-----------|--------------------------|---------|
| `BiomeMap::generate()` | biome_map.rs:~300 | ✅ Uses `RiverNetwork` globally | Macro world gen |
| `BiomeMap::generate_meso_full()` | biome_map.rs:1005 | ✅ `carve_river_channels` + `generate_with_macro_flow_climate` | Macro tile pregen (main.rs:500), meso/micro tile streaming (main.rs:783) |
| `BiomeMap::generate_region_with_sub_stellar()` | biome_map.rs:799 | ❌ Independent `RiverGenerator` | `generate_region()` convenience wrapper |
| `BiomeMap::generate_region()` | biome_map.rs:784 | ❌ Delegates to above | External callers, examples |

## Implementation Plan

### Step 1: Add `macro_rivers` parameter to `generate_region_with_sub_stellar`

**File:** `crates/rb_noise/src/biome_map.rs`

Add an optional macro river context parameter to both `generate_region()` and `generate_region_with_sub_stellar()`:

```rust
pub fn generate_region_with_sub_stellar(
    seed: u32,
    world_x: f64,
    world_y: f64,
    world_size: f64,
    output_size: usize,
    world_height: f64,
    detail_level: u32,
    sub_stellar_x: f64,
    sub_stellar_y: f64,
    macro_rivers: Option<MacroRiverContext<'_>>,  // NEW
) -> Self
```

Define a small context struct (put it in `biome_map.rs` or `rivers.rs`):

```rust
/// Macro-level river data passed down to sub-region generation
/// so that meso/micro rivers are seeded from parent rivers.
pub struct MacroRiverContext<'a> {
    pub flow_grid: &'a [f64],      // macro BiomeMap.rivers
    pub grid_width: usize,         // macro BiomeMap.width
    pub grid_height: usize,        // macro BiomeMap.height
}
```

### Step 2: Wire up `carve_river_channels` + `generate_with_macro_flow_climate` in `generate_region_with_sub_stellar`

**File:** `crates/rb_noise/src/biome_map.rs` — the river section starting at line 885.

Replace:
```rust
// Current: independent river generation
let river_gen = RiverGenerator::for_map_size(SEA_LEVEL, output_size, output_size);
let rivers = river_gen.generate(&heightmap_vec, output_size, output_size);
```

With the same pattern used in `generate_meso_full()` (lines 1123–1153):
```rust
let river_gen = RiverGenerator::for_map_size(SEA_LEVEL, output_size, output_size);
let rivers = if let Some(ref macro_ctx) = macro_rivers {
    // Carve macro river channels into local heightmap so D8 follows them
    let mut carved_heightmap = heightmap_vec.clone();
    crate::rivers::carve_river_channels(
        &mut carved_heightmap,
        output_size, output_size,
        macro_ctx.flow_grid, macro_ctx.grid_width, macro_ctx.grid_height,
        world_x, world_y, world_size,
        SEA_LEVEL,
    );
    // Seed D8 accumulation from macro rivers + climate-aware thresholds
    river_gen.generate_with_macro_flow_climate(
        &carved_heightmap,
        output_size, output_size,
        macro_ctx.flow_grid, macro_ctx.grid_width, macro_ctx.grid_height,
        world_x, world_y, world_size,
        &light_level, &humidity,
    )
} else {
    // Fallback: no macro context, use climate-aware standalone generation
    river_gen.generate_climate_aware(&heightmap_vec, &light_level, &humidity, output_size, output_size)
};
```

**Note:** `generate_region_with_sub_stellar` currently doesn't have `light_level` or `humidity` as separate vecs — they're computed per-pixel in the loop but stored. Verify they're available as `Vec<f64>` before the river section. They are: `light_level` and `humidity` are pushed into vecs at lines 829/827.

### Step 3: Scale the accumulation threshold by detail level

**File:** `crates/rb_noise/src/rivers.rs` — `RiverGenerator::for_map_size()`

The current threshold is `0.0005 × total_pixels`, which produces similar river density at every scale. At higher detail levels (zoomed in), we're looking at a smaller world area with more pixels, so the threshold should increase to suppress tiny tributaries.

```rust
pub fn for_map_size_with_detail(sea_level: f64, width: usize, height: usize, detail_level: u32) -> Self {
    let total = width * height;
    // Base threshold scales with pixel count
    let base_threshold = ((total as f64) * 0.0005).max(25.0);
    // Higher detail = stricter threshold (fewer independent rivers)
    // detail_level 1 = macro (1×), 2 = meso (3×), 3 = micro (8×)
    let detail_multiplier = match detail_level {
        0 | 1 => 1.0,
        2 => 3.0,
        3 => 8.0,
        _ => (detail_level as f64).powi(2),
    };
    Self {
        sea_level,
        min_accumulation: (base_threshold * detail_multiplier) as u32,
    }
}
```

Then use `for_map_size_with_detail` in `generate_region_with_sub_stellar` instead of `for_map_size`.

### Step 4: Update `generate_region()` convenience wrapper

**File:** `crates/rb_noise/src/biome_map.rs`

The simple `generate_region()` wrapper should pass `None` for macro rivers (backward compatible):

```rust
pub fn generate_region(
    seed: u32,
    world_x: f64, world_y: f64,
    world_size: f64,
    output_size: usize,
    world_height: f64,
    detail_level: u32,
) -> Self {
    Self::generate_region_with_sub_stellar(
        seed, world_x, world_y, world_size, output_size,
        world_height, detail_level, 0.5, 1.0,
        None,  // no macro rivers = standalone generation
    )
}
```

### Step 5: Pass macro river context from callers

**File:** `src/main.rs` — wherever `generate_region` or `generate_region_with_sub_stellar` is called.

Currently the main meso/micro tile streaming path (main.rs:783) already uses `generate_meso_full_with_backend` and passes `macro_map`, so it already works. But verify there are no other callers of `generate_region` that should pass macro context.

Check: `crates/rb_noise/examples/save_debug_layers.rs` — this example already uses `generate_meso_full_with_backend`, so it's fine.

If any callers exist that use `generate_region` for zoomed views, they need to be updated to pass `MacroRiverContext` from the cached macro `BiomeMap`.

### Step 6: Update the `generate_biome_only` fast path

**File:** `crates/rb_noise/src/biome_map.rs:968`

`generate_biome_only()` skips rivers entirely (it only computes continentalness, temperature, biome). Verify it doesn't produce river biome tiles. If it does, it needs the same fix. If it only returns raw image data without biome overrides, it's fine as-is.

### Step 7: Test with `save_debug_layers` example

```bash
cargo run -p rb_noise --example save_debug_layers
```

Compare the macro and meso river layers visually. Rivers in the meso view should be a subset of (or refinement of) the macro rivers — not an independent network. Specifically:

- **Macro rivers should appear in the same positions at meso level**, just with more detail (meanders, width variation)
- **New tributaries at meso level should feed INTO macro rivers**, not appear independently
- **Arid regions should have fewer rivers at all zoom levels** (climate-aware threshold)
- **Total river pixel count at meso should NOT exceed** ~2× the macro count for the same world area

### Step 8: Remove micro zoom tier from the world map entirely

Micro-level zoom should NOT be accessible from the world map — it's reserved for playable levels only. The world map should stop at meso.

**File:** `src/main.rs`

**8a. Remove the Meso → Micro transition in `update_view_level()` (~line 626–637):**

Change:
```rust
DetailTier::Meso => {
    if scale > 0.6 {
        Some(DetailTier::Macro)
    } else if scale < 0.08 {
        Some(DetailTier::Micro)
    } else {
        None
    }
}
DetailTier::Micro => {
    if scale > 0.12 { Some(DetailTier::Meso) } else { None }
}
```

To:
```rust
DetailTier::Meso => {
    if scale > 0.6 {
        Some(DetailTier::Macro)
    } else {
        None
    }
}
// Micro tier is entered only via LevelLauncher, never from map zoom
DetailTier::Micro => {
    Some(DetailTier::Meso) // Always bounce back to meso if somehow here
}
```

**8b. Clamp the minimum zoom scale so you can't zoom past meso range.**

In the camera zoom handling system, clamp `projection.scale` to a minimum of `0.08` (the old meso→micro threshold) when in WorldGenerator or WorldMapEditor mode. This prevents the camera from zooming so far in that meso tiles become giant blurry sprites:

```rust
// In zoom input handling:
let min_scale = match app_mode {
    AppMode::LevelLauncher => 0.005, // Full zoom for playable level
    _ => 0.08,                        // Stop at meso for map views
};
projection.scale = projection.scale.clamp(min_scale, 10.0);
```

**8c. Remove micro tile dispatch from `enqueue_and_dispatch_tiles()` (~lines 731–751).**

The `DetailTier::Micro` branch in the tile dispatch loop generates micro tiles for the world map. Remove this branch entirely — micro tiles will instead be generated by the playable level chunk pipeline (see PLAN-playable-level.md).

**8d. Remove micro sprite pool and rendering from `manage_tile_sprites()` (~lines 1003–1070).**

Remove the micro sprite management from the world map renderer. Can keep the `MICRO_POOL_SIZE` and `micro_tiles` cache infrastructure in `TileCache` since the playable level system will need its own tile cache, but remove the world-map micro rendering path.

## Files to Modify

1. `crates/rb_noise/src/biome_map.rs` — Steps 1, 2, 4, 6
2. `crates/rb_noise/src/rivers.rs` — Step 3 (new `for_map_size_with_detail`)
3. `src/main.rs` — Steps 5, 8a, 8b, 8c, 8d (remove micro from world map)

## Files to Read (for context)

- `crates/rb_noise/src/rivers.rs` — `carve_river_channels` (~line 1303), `generate_with_macro_flow_climate` (~line 1445), `RiverGenerator::for_map_size` (~line 1405)
- `crates/rb_noise/src/biome_map.rs` — `generate_meso_full` (~line 1005) as the reference implementation

## Risk Assessment

- **Low risk:** Adding `MacroRiverContext` param with `Option` means all existing callers passing `None` behave identically to current code.
- **Medium risk:** The `carve_river_channels` function maps macro grid coords → meso pixel coords. If `generate_region_with_sub_stellar` uses different world coordinate conventions than `generate_meso_full`, the carving will land in wrong positions. Verify that `world_x`, `world_y`, `world_size` mean the same thing in both methods.
- **Low risk:** `for_map_size_with_detail` multipliers (3× for meso, 8× for micro) are initial guesses. May need tuning based on visual results.

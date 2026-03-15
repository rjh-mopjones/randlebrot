# Randlebrot — Terrain Generation Design Document

> Fractal noise as a compression algorithm for plausibility. Not generating a random world — filling in infinite detail for a handcrafted design.

---

## World Premise

Randlebrot takes place on a **tidally locked planet** — one hemisphere permanently faces its star, the other locked in eternal darkness. This is the primary driver of every climate and terrain decision in the system. There is no day/night cycle, no seasonal variation, no latitude-based temperature gradient in the traditional sense.

The map is oriented as follows:

- **South** — the sub-stellar point. Scorched desert. Deadly heat. The most hostile and resource-rich region.
- **North** — the anti-stellar point. Frozen wasteland. Perpetual darkness. Impassable.
- **Middle band** — the terminator crescent running east-west. The habitable zone. Where civilisation clings.

All climate logic, all biome distribution, all atmospheric circulation radiates from the sub-stellar point outward. There is no east, west, or centre framing — the world is radially symmetric around that southern anchor.

---

## Architecture Overview

The terrain system is built on two principles:

**Strategy Pattern** — every base noise layer is an independent `NoiseStrategy` implementation in `rb_noise/src/strategy/`. Each gets its own seeded `OpenSimplex` instance. They have no knowledge of each other.

**Derived Computation** — everything above the base layers is computed in `rb_noise/src/derived/mod.rs` by combining base layer outputs mathematically. Derived layers are never independent noise.

```
Base Layers (noise strategies — strategy/)
    ↓
Derived Layers (math — derived/mod.rs)
    ↓
Biome Classification (biome_splines.rs)
    ↓
BiomeMap (biome_map.rs — CPU or GPU path)
    ↓
Chunk Storage + LOD (chunk_hierarchy.rs)
```

Generation is **async** — `progress.rs` tracks per-layer completion for the UI progress bars shown in the F1 World Generator panel. The editor layer picker explicitly separates Base and Derived layers. Debug PNGs are written to `debug_layers/base/` and `debug_layers/derived/` subdirectories.

---

## Base Layers

These are the five independent noise inputs. Everything else in the system flows from combinations of these. All five strategy files exist in `rb_noise/src/strategy/`. `temperature.rs` also exists but is **legacy/deprecated** — temperature is now a derived layer computed from light level. `tidally_locked.rs` contains the old `LatitudeTemperatureStrategy` and is dead code pending removal.

### 1. Continentalness
**File:** `strategy/continentalness.rs`

16-octave fBm at large scale. The fundamental land/ocean mask. High values are continental crust, low values are ocean basin. Drives the coastline shapes and the broad continental shelf geometry. The highest-octave layer in the system — it needs to capture both the coarse continent shapes and fine coastal detail in a single pass.

### 2. Tectonic Stress
**File:** `strategy/tectonic.rs`

Voronoi plate generation with velocity vectors and density classification per plate. Each plate is either oceanic (density 0.3–0.5) or continental (density 0.6–1.0). Plate boundaries are classified by the relative motion of adjacent plates:

| Boundary Type | Mechanism | Terrain Effect |
|---|---|---|
| Continental Collision | Both plates continental, converging | Highest mountain ranges, broadest uplift |
| Subduction | Oceanic into continental, converging | Volcanic arc inland, ocean trench |
| Oceanic Subduction | Both oceanic, converging | Island arc, deep trench |
| Divergent | Plates pulling apart | Rift valleys, shield volcanism |
| Transform | Plates sliding past each other | Narrow fault zones, minimal elevation change |

**Domain warping is applied in two passes** before any Voronoi distance calculation. Raw Voronoi cells have mathematically straight edges — double-pass warping at different frequencies (large-scale bend + medium-scale fault offsets) breaks this into natural irregular boundary geometry. Warp magnitudes: 120 units (large) + 40 units (medium).

The stress field has two components: boundary stress (exponential falloff from warped boundary, falloff rate varies by boundary type) and interior stress (low-amplitude noise that gives plate interiors texture rather than uniform fill).

Transform boundaries use a tight falloff coefficient (0.08) — they are narrow shear zones and must not broadcast stress wide into the interior.

### 3. Humidity
**File:** `strategy/humidity.rs`

fBm noise combined with a light-level drying factor. On a tidally locked planet, atmospheric circulation is dominated by a permanent convection cell — hot air rises at the sub-stellar point, flows toward the terminator, dumps moisture as it cools, and returns. The intended result:

- **Day side (south):** low humidity — convection lifts moisture away, extreme heat dries surface
- **Terminator ring:** peak humidity — where moisture-laden air cools and precipitates
- **Night side (north):** cold trap — moisture freezes out, effectively zero humidity

The terminator ring bias should be implemented as `1.0 - |dist_from_substellar * 2.0 - 1.0|`, peaking at angular distance 0.5 from the sub-stellar point, blended 50/50 with raw noise. The current implementation uses ocean distance + light-level drying — **verify whether the full terminator ring bias is in place or still needs adding.**

### 4. Rock Hardness
**File:** `strategy/rock_hardness.rs`

3-octave fBm at medium scale. Represents the underlying geological resistance of the crust — ancient crystalline basement vs young sedimentary layers vs volcanic basalt. Not directly visible but drives erosion resistance, resource concentration, and soil character. Higher persistence (0.6) than other layers to give it a more blocky, geological feel.

### 5. Light Level
**File:** `strategy/light_level.rs`

The most important base layer for a tidally locked world. **Not traditional noise** — it is a geometric calculation of angular distance from the sub-stellar point, with a small noise perturbation for atmospheric scatter.

```
base_light = cos(angular_distance_from_substellar).max(0.0)
scatter    = noise * 0.05
light_level = (base_light + scatter).clamp(0.0, 1.0)
```

Output: 1.0 at the sub-stellar point (south), 0.0 at the terminator, 0.0 on the night side (dark side receives no geometric light). The scatter term blurs the terminator line — in reality the transition is gradual due to atmospheric refraction.

The sub-stellar point comes from `WorldDefinition` — it is a world constant, not a per-sample parameter.

**This replaces temperature as a base layer.** Temperature is derived from light level, not sampled independently.

---

## Derived Layers

Computed in `derived/mod.rs` after base layers are sampled. Never noise. Never stored as independent strategies.

### Peaks & Valleys
Derives from: Tectonic Stress × roughness noise

Mountains exist where plate tectonics produce them, not wherever noise happens to peak. A low-amplitude high-frequency roughness noise is gated by tectonic stress — maximum relief near active boundaries, minimal relief in stable plate interiors. Near coastlines (where continentalness tapers) relief is further reduced by `continentalness.sqrt()`.

### Heightmap
Derives from: Continentalness + Tectonic Stress + Peaks & Valleys

Three contributions combined:
- **Continental bias** (0.8 weight) — drives land vs ocean elevation baseline
- **Tectonic base** (0.5 weight) — broad uplift from plate collision
- **Relief** — Peaks & Valleys, tapered by continentalness near coasts

Result clamped to [-1.0, 1.0]. Negative = below sea level. The heightmap is computed once globally at 512×512 for river generation, then recomputed at full resolution per chunk.

### Temperature
Derives from: Light Level + Heightmap (lapse rate) + Humidity (advection)

```
base_temp  = light_level * 80.0 - 40.0   // -40°C dark side, +40°C sub-stellar
lapse_rate = height.max(0.0) * 30.0      // mountains are colder
hum_buffer = humidity * 5.0              // moisture moderates extremes
temperature = base_temp - lapse_rate + hum_buffer
```

Temperature is **never stored in chunks**. It is always computed on demand. This is intentional — storing it would require invalidating it whenever light level or heightmap changes, and it is cheap to recompute.

### Erosion
Derives from: Heightmap + Rock Hardness + Humidity

A process, not a layer in the traditional sense. Slope (proxied by absolute height value, replaced by real gradient where available) combined with humidity and inverse rock hardness. Soft rock + wet climate + steep slope = high erosion. Drives soil character and resource exposure.

### Aridity
Derives from: Temperature + Humidity

`(temperature / 40.0) * (1.0 - humidity)`, clamped to [0, 1]. High temperature and low humidity = desert. Used as a direct input to biome classification — distinguishes cold dry steppe from cold wet tundra, hot wet jungle from hot dry scrub.

### Volcanism
Derives from: Tectonic Stress (three independent sources)

Volcanism is **never mapped directly to plate boundary lines**. Each source has a distinct spatial signature:

**Subduction Arc** — offset 80–200 units inland from the trench on the continental side only. Bell-curve peak at 130 units. Broken into discrete clusters by a sparse noise mask (~30–40% coverage) to produce chains of stratovolcano peaks rather than a continuous band.

**Rift Volcanism** — narrow and tight along divergent boundaries. Falloff coefficient 0.12 (much tighter than convergent boundaries). Further broken by a fissure field mask (~30% coverage) producing isolated vent patches.

**Hotspot Volcanism** — 3–8 per world, entirely independent of boundary geometry. Gaussian blobs (radius 60–150 units) with internal texture noise. Positioned with minimum distance separation from each other and biased away from boundary zones so they read as distinct features.

Final volcanism: `(arc * 1.0 + rift * 0.5 + hotspot * 0.9).clamp(0.0, 1.0)`

### Precipitation Type
Derives from: Temperature + Humidity + Heightmap

Classifies whether precipitation falls as rain, snow, or nothing. Cold + high elevation + humidity → snow. Warm + humidity → rain. Hot + low humidity → no precipitation. Feeds directly into snowpack accumulation.

### Snowpack
Derives from: Precipitation Type + Temperature

Accumulation of persistent snow and ice. High in the north (permanent), present at high elevation in the terminator zone, absent on the day side. Affects terrain appearance and traversability.

### River Flow
Derives from: Heightmap (global pre-pass)

See River System section below.

### River Moisture
Derives from: River Flow paths

Cells near rivers get a moisture boost — small radius of elevated effective humidity around river channels. Drives vegetation density in otherwise dry areas.

### Biome
Derives from: Temperature + Humidity + Aridity + Heightmap → `biome_splines.rs`

Fed into the existing spline-based classifier. Biome bands are **radial from the sub-stellar point**, not horizontal latitude bands.

| Light Level | Zone | Dominant Biomes |
|---|---|---|
| > 0.7 | Deep day side | Scorched Desert, Bare Rock |
| 0.3 – 0.7 | Mid day side | Tropical Forest, Savanna |
| 0.1 – 0.3 | Terminator crescent | Forest, Wetland, Grassland |
| 0.05 – 0.1 | Night fringe | Tundra, Cold Steppe |
| < 0.05 | Deep night side | Frozen Wasteland, Eternal Dark |

### Vegetation Density
Derives from: Biome + River Moisture

Modulated by volcanism (active volcanic cells suppress local vegetation, surrounding fertile volcanic soil boosts it) and by snowpack (suppresses where snow is permanent).

### Soil Type
Derives from: Biome + Erosion + Rock Hardness

Classifies underlying soil character — thin rocky alpine soil, deep rich loam, volcanic regolith, permafrost, desert hardpan. Affects resource distribution and future agriculture systems.

### Resources
Derives from: Tectonic Stress + Rock Hardness + Erosion

Geologically grounded distribution. Collision zones concentrate metamorphic minerals. Subduction zones concentrate rare earth deposits near arc volcanism. Ancient stable cratons (old, hard rock, low erosion) expose deep basement minerals. High erosion exposes what the rock hardness layer would otherwise hide.

Resource types are defined in `rb_core/src/resource_type.rs` as a `ResourceType` enum (Iron, Gold, Timber, etc). Sparse storage per tile is handled by `rb_noise/src/resource_map.rs`. Distribution logic lives in `rb_noise/src/resource.rs`, which uses `strategy/resource.rs` for per-resource-type noise with geological bias baked in.

---

## Visualisation

`rb_noise/src/visualization.rs` contains the `NoiseLayer` enum and colour mapping functions for converting raw `f64` layer values to RGBA for display. The F1 editor panel has a layer picker that lets you view any individual layer — base or derived — as a coloured overlay on the map.

Debug PNG export: `cargo run -p rb_noise --example save_debug_layers`

Output structure:
```
debug_layers/
    aggregate.png          ← biome aggregate view
    base/
        continentalness.png
        tectonic.png
        light_level.png
        rock_hardness.png
    derived/
        temperature.png
        erosion.png
        peaks_valleys.png
        humidity.png
        rivers.png
```

When adding new layers, add them to both `NoiseLayer` in `visualization.rs` and `save_debug_layers.rs`.

---

## River System

`rb_noise/src/rivers.rs` already implements D8 flow accumulation, depression filling, and macro-seeded meso river generation. **Read this file before touching anything river-related.**

Rivers require a **global heightmap** to compute drainage — lazy chunk loading cannot produce consistent flow directions. Two-pass approach:

**Pass 1 — Global skeleton (runs once at world generation)**
1. Sample continentalness + tectonic at 512×512 coarse resolution
2. Derive coarse heightmap
3. Run Priority-Flood depression filling (Wang & Liu 2006)
4. Run D8 flow direction + drainage accumulation (sort cells high-to-low, accumulate downstream)
5. Extract river skeleton: cells where `drainage_area > 500.0`
6. Store as `RiverNetwork` keyed by MacroChunk coord — **immutable after this point**

**Pass 2 — Chunk generation**
When generating any chunk at any LOD level, query `RiverNetwork` for segments intersecting that chunk's bounds. Pass segments as locked constraints. The chunk carves its heightmap to match — rivers never move.

### LOD Consistency

The core invariant: a trunk river visible at macro zoom must appear in **exactly the same position** when zoomed into micro detail. Finer LOD levels only add tributaries and streams that feed into already-locked coarser geometry.

| LOD Level | Chunk Size | Drainage Threshold | Visible |
|---|---|---|---|
| Macro | 32×32 | 500.0 | Trunk rivers only |
| Meso | 64×64 | 50.0 | Tributaries feeding trunks |
| Micro | 128×128 | 5.0 | Streams, full carving |

River carving is smooth — deepest at channel centre, tapering to banks. Width and depth scale with drainage area (square root and log10 respectively).

**Invariants:**
- Trunk positions are locked forever once computed
- Finer levels add detail that feeds into locked coarser geometry — never the other way
- Terrain is carved to fit rivers — rivers never move to fit terrain
- Rivers terminate where `continentalness < 0.0`

---

## Chunk Hierarchy

Three levels of detail with LRU eviction, all in `rb_noise/src/chunk_hierarchy.rs`.

| Level | Size | Detail | Caches |
|---|---|---|---|
| MacroChunk | 32×32 | Lowest | Up to N MesoChunks |
| MesoChunk | 64×64 | Medium | Up to 16 MicroChunks |
| MicroChunk | 128×128 | Highest | — |

Every chunk at every level stores all **five base layers** as `Vec<f64>`. Temperature is not stored — always derived on demand. River segments are passed in as constraints at generation time and applied during heightmap computation.

`WorldChunks` is the top-level struct. It holds all five strategy trait objects, the global `RiverNetwork`, and the MacroChunk LRU cache.

---

## Layer Dependency Graph

```
Continentalness ──────────────────────────────────────┐
                                                       ├── Heightmap ──────────────────┐
Tectonic Stress ──── Peaks & Valleys ─────────────────┘                               │
               │                                                                        ├── River Flow
               └──── Volcanism (arc + rift)                                            │
                                                                                        ├── Temperature ──┐
Hotspots ─────────── Volcanism (hotspot)                                               │                 │
                                                       Light Level ────────────────────┘                 ├── Aridity
                                                                                                          │
Humidity ─────────────────────────────────────────────────────────────────────────────────────────────────┘
         │                                                                                                 │
         └──────────────────────────────────────────────────────────────── Precipitation Type             │
                                                                                    │                     │
Rock Hardness ──── Erosion ──── Soil Type                                           └── Snowpack          │
              │         │                                                                                  │
              │         └──── Resources                                                                    │
              │                                                                                            │
              └──────────────────────────────────── Biome ←──────────────────────────────────────────────┘
                                                       │
                                        River Moisture ─┤
                                                        └── Vegetation Density
```

---

## Seeding Convention

Each base layer gets its own `OpenSimplex` instance seeded from `WorldDefinition.seed`:

| Layer | Seed Offset |
|---|---|
| Continentalness | `seed.wrapping_add(0)` |
| Tectonic Stress | `seed.wrapping_add(1)` |
| Humidity | `seed.wrapping_add(2)` |
| Rock Hardness | `seed.wrapping_add(3)` |
| Light Level | `seed.wrapping_add(4)` |
| Warp noise A (tectonic) | `seed.wrapping_add(5)` |
| Warp noise B (tectonic) | `seed.wrapping_add(6)` |

Light Level uses `WorldDefinition.sub_stellar` — a world constant, not a noise seed. Default sub-stellar position: `(0.5, 1.0)` — southern centre of the map.

---

## GPU Acceleration

GPU compute shader acceleration is implemented and **enabled by default** via the `gpu` feature flag in `rb_noise/Cargo.toml`. The GPU path lives in `rb_noise/src/gpu/`:

- `context.rs` — `GpuNoiseContext`: wgpu device/queue management, layer dispatch
- `pipelines.rs` — WGSL compute shaders for each base layer
- `perm_table.rs` — permutation table for GPU noise

`biome_map.rs` dispatches to the GPU path when the feature is enabled, with CPU fallback otherwise. Any new base layer strategy must have a WGSL shader added to `pipelines.rs` following the existing pattern.

---

## What This System Does Not Do (Yet)

- Civilisations, settlements, roads, factions — entirely separate system, not part of terrain generation
- Authored data overrides (city tile-by-tile layout, landmark placement) — separate layer on top of procedural
- GPU shaders for Light Level and Rock Hardness — CPU path exists, WGSL shaders not yet added to `gpu/pipelines.rs`
- Full terminator ring bias in Humidity — current implementation uses ocean distance + light-level drying; full atmospheric circulation model pending verification
- Hotspot island chain simulation (Hawaii-style trailing chain as plate drifts over hotspot) — deferred
- Volcanism derived layer — three-source system (arc + rift + hotspot) designed but not yet implemented
- `tidally_locked.rs` (`LatitudeTemperatureStrategy`) — dead code, safe to delete once confirmed nothing references it

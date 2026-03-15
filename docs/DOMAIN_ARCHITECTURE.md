# Randlebrot — Three-Domain Architecture

> Defining the data contracts between TerrainGen, LifeGen, and SceneGen.

---

## The Problem

Right now `WorldDefinition` is a god object. It stores terrain parameters (seed, noise params, sea level, sub-stellar point) AND civilisation output (cities, factions, roads, trade routes, territory cache, cultures). `CivilizationGenerator` takes `&BiomeMap` directly and writes results into `&mut WorldDefinition`. There is no clean boundary between the dead planet and what life built on it.

Before building any new systems, we need to establish the interfaces between the three generation domains so that each can be developed, tested, and iterated independently.

---

## Domain Boundaries

### The Rule
> *"Would this exist if no living thing had ever touched the planet?"*
> Yes → TerrainGen. No → LifeGen. Only visible at street level → SceneGen.

### Domain 1: TerrainGen (`rb_noise`)
**Runs first. Output is immutable forever after.**

Already largely exists. Produces `BiomeMap` with all base and derived layers, river network, resource map. The missing piece is a clean **read-only query interface** that LifeGen can consume without reaching into `BiomeMap` field access.

**Owns:** continentalness, tectonic stress, humidity, rock hardness, light level, heightmap, temperature, erosion, peaks & valleys, aridity, volcanism, precipitation, snowpack, wind, rivers, biomes, vegetation density, soil type, resources.

**Output type:** `TerrainData` — a trait or struct that exposes terrain queries by world coordinate.

### Domain 2: LifeGen (`rb_world`)
**Runs second. Reads terrain, never writes back. Operates at meso resolution (8192×4096).**

Takes the pre-generated meso terrain tiles as input. The macro map is 1024×512, divided into 16×8 = 128 macro chunks of 64×64 world units each. Each macro chunk is pre-generated as a 512×512 pixel `BiomeMap` via `generate_meso_full`. The full stitched meso resolution is **8192×4096** (16×512 wide, 8×512 tall). This is the resolution LifeGen operates at — it reads the 128 cached `Arc<BiomeMap>` tiles that already exist in `TileCache` after the macro pre-generation phase.

LifeGen layers (habitability, provinces, etc.) are computed and stored at this same 8192×4096 resolution. Debug PNGs are saved to `debug_layers/lifegen/` following the same stitching pattern as terrain debug layers (downscaled 2× to 4096×2048 for file size).

Produces provinces, factions, settlement seeds, roads, trade network. Output is immutable after world gen (will mutate later via monthly political tick, but that's a future concern).

**Owns:** habitability scoring, navigation cost, resource desirability, provinces (Voronoi), faction territory, culture zones, settlement seeds (position + size class + hierarchy tier), road network (A* over terrain cost), trade DAG, prosperity scores.

**Output type:** `LifeGenData` — queryable by world coordinate and by province ID. Contains both continuous analysis grids (`Vec<f32>` at 8192×4096) and discrete indexed layers (province bitmap, faction assignment).

### Domain 3: SceneGen (`rb_scene` or `rb_world/scene_gen/`)
**Runs on demand when micro chunks are requested. Pure function.**

Takes `&TerrainData` + `&LifeGenData` + settlement seed → produces a 128×128 tile grid. No state, no cache, deterministic from inputs. This is a layout algorithm (spatial grammar, lot subdivision, building placement), not a noise system.

**Owns:** settlement footprint calculation, internal road skeleton, district zoning, lot subdivision, building placement, detail scatter, transition zones (settlement → wilderness falloff).

**Output type:** `TileGrid` — 128×128 array of tile data for a single micro chunk.

---

## Current State → Target State

### What moves where

| Current location | Contains | Target domain |
|---|---|---|
| `WorldDefinition.seed` | Terrain seed | TerrainGen params (stays) |
| `WorldDefinition.noise_params` | Noise octaves etc | TerrainGen params (stays) |
| `WorldDefinition.sea_level` | Sea level threshold | TerrainGen params (stays) |
| `WorldDefinition.sub_stellar` | Sub-stellar point | TerrainGen params (stays) |
| `WorldDefinition.cities` | Settlement list | LifeGen output (moves) |
| `WorldDefinition.factions` | Faction list | LifeGen output (moves) |
| `WorldDefinition.roads` | Road network | LifeGen output (moves) |
| `WorldDefinition.trade_routes` | Trade routes | LifeGen output (moves) |
| `WorldDefinition.cultures` | Culture list | LifeGen output (moves) |
| `WorldDefinition.territory_cache` | Territory ownership | LifeGen output (moves) |
| `WorldDefinition.regions` | Authored regions | WorldDefinition (stays — authored data) |
| `WorldDefinition.landmarks` | Authored landmarks | WorldDefinition (stays — authored data) |
| `BiomeMap` (all pub fields) | Raw layer vecs | Behind `TerrainData` interface |
| `CivilizationGenerator` | Settlement + faction gen | Restructured into LifeGen pipeline |
| `settlement_placement.rs` | Site scoring | LifeGen Phase 4 (settlement seeds) |
| `territory.rs` | Flood-fill expansion | LifeGen Phase 3 (faction territory) |
| `roads.rs` | A* pathfinding | LifeGen Phase 5 (road network) |

### WorldDefinition after cleanup

```rust
/// World parameters — the knobs you turn before generation.
/// Does NOT store generation output.
pub struct WorldDefinition {
    pub name: String,
    pub seed: u32,              // terrain seed
    pub civ_seed: u32,          // lifegen seed (NEW — separate for iteration)
    pub width: usize,
    pub height: usize,
    pub sea_level: f64,
    pub sub_stellar: (f64, f64),
    pub noise_params: NoiseParams,
    
    // Authored data (hand-placed, not generated)
    pub regions: Vec<Region>,
    pub landmarks: Vec<Landmark>,
}
```

Cities, factions, roads, trade routes, cultures, and territory cache all move to `LifeGenData`.

---

## Interface Definitions

### TerrainData (Domain 1 → Domain 2 boundary)

This is how LifeGen reads the dead planet. The primary implementation wraps the 128 pre-generated meso `BiomeMap` tiles from `TileCache`, providing a seamless 8192×4096 query surface.

```rust
/// Read-only terrain query interface.
/// Coordinates are in pixel space at meso resolution (0..8192, 0..4096).
/// Implementations translate to the appropriate tile + local offset.
pub trait TerrainQuery {
    fn width(&self) -> usize;   // 8192
    fn height(&self) -> usize;  // 4096
    
    // Point queries at meso pixel resolution
    fn heightmap_at(&self, x: usize, y: usize) -> f64;
    fn biome_at(&self, x: usize, y: usize) -> TileType;
    fn temperature_at(&self, x: usize, y: usize) -> f64;
    fn humidity_at(&self, x: usize, y: usize) -> f64;
    fn continentalness_at(&self, x: usize, y: usize) -> f64;
    fn erosion_at(&self, x: usize, y: usize) -> f64;
    fn light_level_at(&self, x: usize, y: usize) -> f64;
    fn rock_hardness_at(&self, x: usize, y: usize) -> f64;
    fn river_at(&self, x: usize, y: usize) -> f64;
    fn drainage_at(&self, x: usize, y: usize) -> f64;
    fn tectonic_at(&self, x: usize, y: usize) -> f64;
    fn slope_at(&self, x: usize, y: usize) -> f64;
    
    // Classification queries
    fn is_ocean(&self, x: usize, y: usize) -> bool;
    fn is_river(&self, x: usize, y: usize) -> bool;
    
    // River network access (global, from macro pre-pass)
    fn river_network(&self) -> Option<&RiverNetwork>;
}
```

**Implementation:** `MesoTerrainView` holds references to the 128 `Arc<BiomeMap>` tiles from `TileCache`. Given an (x, y) at 8192×4096 resolution, it computes which macro chunk (x / 512, y / 512) and the local offset within that tile's 512×512 grid, then delegates to the appropriate `BiomeMap` getter.

```rust
pub struct MesoTerrainView {
    tiles: Vec<Vec<Option<Arc<BiomeMap>>>>,  // [chunk_y][chunk_x], 8×16
    tile_size: usize,                         // 512
    chunks_x: usize,                          // 16
    chunks_y: usize,                          // 8
    river_network: Option<Arc<RiverNetwork>>,
}

impl MesoTerrainView {
    /// Build from TileCache after meso pre-generation completes.
    pub fn from_tile_cache(cache: &TileCache) -> Self { ... }
}

impl TerrainQuery for MesoTerrainView {
    fn heightmap_at(&self, x: usize, y: usize) -> f64 {
        let cx = x / self.tile_size;  // chunk x (0..16)
        let cy = y / self.tile_size;  // chunk y (0..8)
        let lx = x % self.tile_size;  // local x within 512×512 tile
        let ly = y % self.tile_size;  // local y within 512×512 tile
        self.tiles[cy][cx]
            .as_ref()
            .and_then(|bm| Some(bm.heightmap[ly * bm.width + lx]))
            .unwrap_or(0.0)
    }
    // ... same pattern for all other queries
}
```

This trait lives in `rb_core` (so both `rb_noise` and `rb_world` can see it). The `MesoTerrainView` implementation lives in `rb_noise` (since it wraps `BiomeMap`).

### LifeGenData (Domain 2 → Domain 3 boundary)

This is what LifeGen produces. SceneGen and the editor read it. All grid layers are at **8192×4096** resolution (matching the stitched meso terrain).

```rust
/// Complete output of civilisation generation.
/// Immutable after world gen (will support mutation via political tick later).
/// All grid layers are 8192×4096 (same as stitched meso terrain).
pub struct LifeGenData {
    pub width: usize,     // 8192
    pub height: usize,    // 4096
    
    // Analysis grids (continuous, derived from terrain, pre-requisite for provinces)
    pub habitability: Vec<f32>,          // composite score: temp comfort + water + elevation + drainage
    pub navigation_cost: Vec<f32>,       // terrain movement cost per cell (feeds A* and faction expansion)
    pub resource_desirability: Vec<f32>, // attractiveness for settlement based on surrounding resources
    
    // Province layer
    pub province_map: ProvinceMap,       // Vec<u16> bitmap at 8192×4096
    pub provinces: Vec<Province>,
    
    // Faction layer  
    pub factions: Vec<Faction>,
    
    // Settlement layer
    pub settlement_seeds: Vec<SettlementSeed>,
    
    // Road network
    pub road_network: Vec<RoadSegment>,
    
    // Trade network
    pub trade_nodes: Vec<TradeNode>,
    pub trade_edges: Vec<TradeEdge>,
    
    // Culture
    pub cultures: Vec<Culture>,
}

/// Province bitmap at 8192×4096 resolution.
pub struct ProvinceMap {
    pub width: usize,     // 8192
    pub height: usize,    // 4096
    pub ids: Vec<u16>,    // province ID per pixel (0 = ocean/uninhabited)
}

/// Per-province attributes.
pub struct Province {
    pub id: u16,
    pub site: (f64, f64),        // Voronoi site position
    pub biome: TileType,         // majority biome
    pub habitability: f32,
    pub area_px: u32,
    pub is_coastal: bool,
    pub is_river_junction: bool,
    pub elevation_mean: f32,
    pub terrain_cost: f32,
    pub political_state: PoliticalState,
    pub trade_good: Option<TradeGood>,
}

/// Settlement seed — committed position + parameters, no tiles yet.
pub struct SettlementSeed {
    pub id: u32,
    pub position: (f64, f64),     // world coordinates
    pub province_id: u16,
    pub size_class: SizeClass,    // from habitability × area
    pub hierarchy_tier: SettlementTier,
    pub culture_id: u32,
    pub is_fortified: bool,
    pub has_port: bool,
    pub has_market: bool,
    pub primary_good: Option<TradeGood>,
}

pub enum SizeClass {
    Metropolis,   // > 8000 score
    City,         // > 3000
    Town,         // > 800
    Village,      // > 150
    Outpost,      // < 150
}

pub enum SettlementTier {
    Capital,
    Major,
    Minor,
    Outpost,
    Camp,
}

pub enum PoliticalState {
    Claimed { faction_id: u32 },
    Unclaimed,
    Uninhabited,
}
```

### LifeGenQuery (query interface for SceneGen and editor)

```rust
/// Read-only query interface for civilisation data.
/// Grid coordinates are at 8192×4096 meso pixel resolution.
pub trait LifeGenQuery {
    fn width(&self) -> usize;   // 8192
    fn height(&self) -> usize;  // 4096
    
    // Grid queries (meso pixel coordinates)
    fn province_at(&self, x: usize, y: usize) -> Option<u16>;
    fn habitability_at(&self, x: usize, y: usize) -> f32;
    fn navigation_cost_at(&self, x: usize, y: usize) -> f32;
    fn is_province_border(&self, x: usize, y: usize) -> bool;
    fn is_faction_border(&self, x: usize, y: usize) -> bool;
    
    // Indexed queries (by ID)
    fn province(&self, id: u16) -> Option<&Province>;
    fn faction(&self, id: u32) -> Option<&Faction>;
    
    // Spatial queries (world coordinates — for SceneGen micro chunks)
    fn settlements_in_rect(&self, min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Vec<&SettlementSeed>;
    fn roads_in_rect(&self, min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Vec<&RoadSegment>;
}
```

### SceneGen Input/Output (Domain 3)

```rust
/// Everything SceneGen needs to produce a micro tile grid.
pub struct SceneGenInput<'a> {
    pub terrain: &'a dyn TerrainQuery,
    pub lifegen: &'a dyn LifeGenQuery,
    pub settlement: Option<&'a SettlementSeed>,
    pub chunk_bounds: (f64, f64, f64, f64),  // min_x, min_y, max_x, max_y in world coords
}

/// Output: a 128×128 grid of tile data.
pub struct TileGrid {
    pub width: usize,   // 128
    pub height: usize,  // 128
    pub ground: Vec<GroundTile>,
    pub structures: Vec<Option<StructureTile>>,
    pub details: Vec<Option<DetailTile>>,
}

/// Pure function — the entire SceneGen interface.
pub fn generate_scene(input: &SceneGenInput) -> TileGrid { ... }
```

---

## Generation Pipeline

```
WorldDefinition (parameters only)
        │
        ▼
┌── TerrainGen Step 1 ───────────┐
│   Macro BiomeMap (1024×512)    │
│   + river pre-pass (512×512)   │
│   + erosion sim                │
└───────────┬────────────────────┘
            │
            ▼
┌── TerrainGen Step 2 ───────────┐
│   Meso pre-generation          │
│   128 macro chunks × 512×512   │
│   = 8192×4096 stitched terrain │
│   Async with progress bar      │
│   Stored in TileCache          │
│   → debug_layers/base/         │
│   → debug_layers/derived/      │
└───────────┬────────────────────┘
            │ 128 × Arc<BiomeMap> in TileCache
            │ wrapped as impl TerrainQuery
            ▼
┌── LifeGen ─────────────────────┐
│   Reads meso terrain tiles     │
│   Operates at 8192×4096        │
│                                │
│   Phase 1: Analysis grids      │
│     habitability               │
│     navigation_cost            │
│     resource_desirability      │
│   Phase 2: Province Voronoi    │
│     Poisson seeding            │
│     tessellation               │
│     river-snapped borders      │
│     province bitmap (8192×4096)│
│   Phase 3: Faction assignment  │
│     capital placement          │
│     terrain-weighted expand    │
│   Phase 4: Settlement seeds    │
│     per-province site scoring  │
│     commit positions + tier    │
│   Phase 5: Road network        │
│     A* over navigation_cost    │
│   Phase 6: Trade DAG           │
│     flow computation           │
│     prosperity scores          │
│                                │
│   → debug_layers/lifegen/      │
└───────────┬────────────────────┘
            │ Arc<LifeGenData>
            ▼
┌── Chunk Generation ────────────┐
│   Macro: terrain only          │
│   Meso:  terrain + lifegen     │
│          (settlement placement │
│           committed here)      │
│   Micro: terrain + lifegen     │
│          + SceneGen if overlap │
│            with settlement     │
└────────────────────────────────┘
```

### Debug Layer Output

LifeGen saves debug PNGs following the same pattern as terrain:

```
debug_layers/
    biome.png                      ← existing
    base/                          ← existing
        continentalness.png
        tectonic.png
        humidity.png
        rock_hardness.png
        light_level.png
    derived/                       ← existing
        temperature.png
        erosion.png
        heightmap.png
        peaks_valleys.png
        rivers.png
        ...
    lifegen/                       ← NEW
        habitability.png           ← continuous gradient (green = habitable, red = hostile)
        navigation_cost.png        ← continuous gradient (green = easy, red = impassable)
        resource_desirability.png  ← continuous gradient (bright = desirable)
        provinces.png              ← random distinct colour per province ID
        factions.png               ← faction colour per pixel
        political_state.png        ← 3-colour: claimed/unclaimed/uninhabited
        culture_zones.png          ← culture type colour
        trade_flow.png             ← intensity gradient along trade routes
        prosperity.png             ← province prosperity rasterised to grid
        settlements.png            ← dots/markers at settlement seed positions on terrain
        roads.png                  ← road network rendered as lines on terrain
```

All lifegen PNGs are rendered at 8192×4096 and downscaled 2× to 4096×2048 (same as terrain debug layers).

`LifeGenData` implements `save_debug_layers(&self, base_path: &Path)` which creates the `lifegen/` subdirectory and writes all layer PNGs. This is called from the same place in `main.rs` that calls `save_stitched_debug_layers` for terrain.

### Orchestration (main.rs)

```rust
// Step 1: Generate macro terrain (existing)
let biome_map = Arc::new(BiomeMap::generate_with_backend(
    world_def.seed, world_def.width, world_def.height, backend
));

// Step 2: Pre-generate meso tiles (existing — async with progress bar)
// Result: 128 × Arc<BiomeMap> stored in TileCache
// Each is 512×512 pixels, stitched = 8192×4096

// Step 3: Generate civilisation (NEW — after meso pregen completes)
let terrain = MesoTerrainView::new(&tile_cache);  // reads the 128 cached BiomeMap tiles
let lifegen_data = Arc::new(LifeGen::generate(
    world_def.civ_seed,
    &terrain,
    &world_def,  // for authored landmarks/regions
));

// Step 4: Save debug layers
lifegen_data.save_debug_layers(Path::new("debug_layers"));

// Step 5: Store as Bevy resource
commands.insert_resource(lifegen_data);

// SceneGen runs on demand during micro chunk generation — not here
```

### Regeneration Rules

| Action | Terrain | LifeGen | Scene Cache |
|---|---|---|---|
| Change terrain seed | Regenerate | Regenerate | Clear |
| Change civ seed | Keep | Regenerate | Clear |
| Change noise params | Regenerate | Regenerate | Clear |
| Zoom to new micro area | Keep | Keep | Generate on demand |

---

## Implementation Phases

### Phase 1: TerrainQuery trait + MesoTerrainView
**Scope:** `rb_core` + `rb_noise`
**Changes:**
- Define `TerrainQuery` trait in `rb_core` (new file: `terrain_query.rs`)
- Implement `MesoTerrainView` in `rb_noise` (new file: `terrain_view.rs`)
- Wraps the 128 `Arc<BiomeMap>` tiles from `TileCache`
- Maps (x, y) at 8192×4096 → chunk index + local offset → BiomeMap field access
- Add `slope_at` computation (finite difference on heightmap)
- No changes to BiomeMap itself, no changes to editor or main.rs
**Test:** Unit tests that create BiomeMaps and query through TerrainQuery

### Phase 2: Split WorldDefinition
**Scope:** `rb_world`
**Changes:**
- Remove cities, factions, roads, trade_routes, cultures, territory_cache from `WorldDefinition`
- Add `civ_seed: u32` to `WorldDefinition`
- Create `LifeGenData` struct (initially empty — just the type + placeholder fields)
- Add `LifeGenData` as a Bevy Resource
- Update serialization (save/load) — `WorldDefinition` gets smaller, `LifeGenData` is not serialized yet
- Update `CivilizationGenerator` to return `LifeGenData` instead of mutating `WorldDefinition`
**Test:** Compile. Existing civ gen still works but outputs to `LifeGenData` instead of `WorldDefinition`.

### Phase 3: Wire into main.rs
**Scope:** `src/main.rs`
**Changes:**
- After BiomeMap generation, wrap in `BiomeMapTerrain` and insert as resource
- After civ gen (if it runs), insert `LifeGenData` as resource
- Update HighlightInfo to query TerrainQuery instead of raw BiomeMap
**Test:** Editor works as before. HighlightInfo shows terrain values via new interface.

### Phase 4: LifeGenQuery trait + province basics
**Scope:** `rb_core` + `rb_world`
**Changes:**
- Define `LifeGenQuery` trait in `rb_core`
- Implement on `LifeGenData`
- Build habitability scoring (reads TerrainQuery)
- Build province Voronoi (first pass — just the seeding and tessellation)
- Build province bitmap rasterization
**Test:** Generate provinces from terrain, visualize in debug PNG

### Phase 5: Editor domain tabs
**Scope:** `rb_core` + `rb_editor` + `src/main.rs`  
**Changes:**
- Expand AppMode with CivGenerator (F2) and SceneInspector (F3)
- Add LifeGenLayer enum
- Build civ generator panel with layer picker + overlay toggles
- Province/faction map mode rendering
- Global coordinate display in HighlightInfo across all modes
**Test:** F2 shows province map, F1 still shows terrain, coordinates always visible

### Phase 6+: Factions, settlements, roads, trade, SceneGen
Each is its own prompt, building on the interfaces established above.

---

## Coordinate Systems

Three coordinate spaces are in play. Getting the mapping right is critical.

| Space | Range | Resolution | Used by |
|---|---|---|---|
| World space | (0..1024, 0..512) | Continuous f64 | WorldDefinition, macro BiomeMap, river network |
| Meso pixel space | (0..8192, 0..4096) | Integer, 1 pixel = 1/8 world unit | TerrainQuery, LifeGenQuery, all grid layers |
| Chunk-local space | (0..512, 0..512) | Integer within a single BiomeMap tile | BiomeMap field indexing |

**World → Meso pixel:** `meso_x = (world_x / total_world_width) * 8192`, same for y.
Since world is 1024 wide and meso is 8192 wide, the ratio is exactly 8:1.
`meso_x = world_x * 8.0`, `meso_y = world_y * 8.0`

**Meso pixel → Chunk + local:** `chunk_x = meso_x / 512`, `local_x = meso_x % 512`
Each of the 16×8 chunks is 512×512 pixels.

**Macro pixel → Meso pixel:** `meso_x = macro_x * 8`, `meso_y = macro_y * 8`
The macro map is 1024×512, each macro pixel spans an 8×8 block of meso pixels.

---

## Key Principles

1. **TerrainQuery is the only way LifeGen reads terrain.** No `use rb_noise::BiomeMap` in `rb_world` — only `use rb_core::TerrainQuery`.

2. **LifeGenQuery is the only way SceneGen reads civilisation.** Same pattern.

3. **WorldDefinition stores parameters, not output.** Generation results go in domain-specific data structs (`BiomeMap`/`TerrainData`, `LifeGenData`, `TileGrid`).

4. **Each domain's output is immutable after generation** (until political tick is implemented).

5. **SceneGen is a pure function.** Inputs in, tile grid out. No global state, no side effects.

6. **Separate seeds for separate iteration.** Terrain seed and civ seed are independent so you can regenerate politics without rebuilding the planet.

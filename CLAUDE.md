# Randlebrot

A Bevy 0.18 game engine for **Margin's Grip** — a 2D open-world survival RPG set on a tidally locked planet orbiting a red supergiant. Fractal noise serves as a compression algorithm for plausibility—not generating a random world, but filling in infinite detail for a handcrafted design.

Randlebrot is the engine; Margin's Grip (mgrip) is the game. The engine handles procedural world generation (TerrainGen, LifeGen, SceneGen), the deterministic simulation (DeterSim), and the editor/launcher tooling. The game itself will live in a separate private repo.

## Design Sources of Truth

**The Obsidian vault** at `/Users/roryhedderman/Documents/mop-jones-brain/Notes/` (files prefixed "Margin's Grip - ") is the source of truth for all game design, worldbuilding, and gameplay decisions.

**The `docs/` folder** in this repo is the source of truth for implementation details and technical architecture.

**When they conflict:** the Obsidian vault wins for design intent; the repo docs win for how things are currently implemented.

### Repo Design Documents

| Document | Covers |
|----------|--------|
| `docs/TERRAIN_DESIGN.md` | Noise layer system, base/derived layer architecture, river system, chunk hierarchy, GPU acceleration, seeding conventions |
| `docs/DOMAIN_ARCHITECTURE.md` | Three-domain split (TerrainGen/LifeGen/SceneGen), interface contracts (`TerrainQuery`, `LifeGenQuery`), `LifeGenData` schema, generation pipeline, coordinate systems |

### Obsidian Vault Design Documents

| Document | Covers |
|----------|--------|
| **Game World Primer** | High-level overview: 2D survival RPG, tidally locked planet, three geographic zones |
| **Geography** | The Wash / Terminus / The Black zone definitions, temperature model, wind system, ocean upwelling |
| **Geology & Resources** | No fossil fuels, graphite/graphene abundance, wood scarcity, nuclear fuel accessibility, metal production |
| **Energy** | Nuclear baseload, graphite moderators, hydropower, wind, graphene supercapacitors, Haber-Bosch via electrolysis |
| **Agriculture** | Finite Terminus farmland, continuous photosynthesis, marine upwelling, fungiculture, nuclear greenhouses |
| **Flora & Fauna** | No green on the planet (red giant spectrum), black/purple/burgundy photosynthesis, wind-sculpted trees, nightside troglomorphism, bioluminescence, underground biosphere |
| **History** | Himaya era, bio-magnetic mutation, The Triad and Centuria, artificial magnetosphere, space elevator destruction (20 years ago), supernova clock (~500 years) |
| **Cultures** | Five base culture types derived from terrain, cultural drift from sim fields, bio-magnetism in daily life, post-Himaya fractures |
| **Religion** | Place-based theology (no day/night cycle), two observable apocalypse clocks, Zenites, Penumbra, Stillborn Dawn, etc. |
| **Factions** | 50+ factions in three tiers (States, Orders, Nomadic), Terminus/Authoritarian/Isolationist/Nightside states, religious orders, nomadic pressure fields |
| **Economy** | Gradient Runner currency, railway-backbone trade, no long-distance road freight, trade DAG, elevator destruction economic shock |
| **Technology** | No plastics, no asphalt, graphene-on-diamond computing, no print age, nuclear foundation, permanent wind harvesting, mycelium as standard material, bio-magnetic interfaces |
| **Transport** | Rail as freight spine, nuclear ships, no highway network, dense walkable cities, buggies rare, airships viable |
| **Weapons & Warfare** | Gunpowder normal (non-fuel-dependent), nuclear threshold reachable, graphene composite armour, directed energy primary, no polymer components |
| **Randlebrot Engine** | Engine architecture, chunk hierarchy, streaming, rendering resolution, editor modes, two-seed system |
| **World Generation** | TerrainGen/LifeGen/SceneGen pipeline, biome zones by light level, river locked constraints, province/faction generation, settlement sizing |
| **DeterSim** | Deterministic simulation: `f(world_seed, game_time)`, time axis frequencies, province/faction fields, NPC three-tier architecture, event log, archaeology |
| **Save System** | Minimal save: seed + time + events (kilobytes), no world state stored |
| **Combat & Resolution** | Three input modes into one resolver, limb-based injury (Healthy→Lost), morale cohesion, positioning emphasis |
| **Items & Crafting** | Zone-dependent gear, quality spectrum 0.0–1.0, recipe discovery, tool quality caps output |
| **Survival & Movement** | Geographic survival pressure (heat/cold from terrain), line-of-sight required for top-down camera |
| **Companions** | Emergent acquisition, autonomous with overridable orders, permanent death viable |
| **Base Building** | Late-game win condition, settlement in Unclaimed fringe province, visibility penalty in sim |
| **Player & Progression** | Four skill tracks (use-based, 1-100), Notoriety tiers, bio-magnetism hidden progression (Tier 0-5), body as progression record, no quest log |
| **Main Quest** | Space elevator mystery, supernova clock, ~20-30 AuthoredTension structs resolved via sim |
| **Side Quests** | Emergent from sim field imbalances, five tension types, no authored quests, no quest log |
| **Emergent Systems** | Append-only WorldEvent log, field-based NPC instantiation, ~1M inhabitants from seed+time+events |
| **Game Loop** | Early (nobody, survival) → Mid (local factor, companions) → Late (meaningful choice, base, faction influence) |
| **Art & Audio** | 2D first, then voxel space raycasting (Comanche-style) as isolated crate consuming TerrainQuery |
| **Interface** | Top-down 2D camera mandate |
| **Geopolitics** | (Empty — covered by Factions) |

## World Rules (Never Violate)

These rules derive from the game design and are non-negotiable constraints on the engine. If code violates these, the simulation is wrong.

### Planet Physics

- **Tidally locked** — no day/night cycle, no seasons, no latitude-based temperature. ALL climate logic radiates from the sub-stellar point.
- **Three zones**: The Wash (dayside, scorching desert), Terminus (habitable crescent), The Black (frozen nightside). Determined by `light_level` thresholds.
- **Temperature is derived** from `light_level × 80 − 40 − lapse_rate + humidity_buffer`. Never an independent noise layer.
- **Sub-stellar point** default: `(0.5, 1.0)` — bottom center of map. South = day, North = night, Middle = habitable.
- **Permanent unidirectional wind** — dayside-to-nightside pressure differential. Never calm.
- **No liquid surface water on dayside** — all evaporated. Nightside ice is the planet's freshwater reservoir.

### Biology & Aesthetics

- **No green anywhere on the planet.** The star is a red giant producing far less blue-green light. All photosynthetic organisms appear **black, dark purple, burgundy, maroon**. If you see green in biome output, the palette is wrong.
- **All trees lean toward the sub-stellar point** — phototropism + persistent wind sculpting. Flag-form growth, bamboo-strategy flexible stems.
- **Bioluminescence dominates nightside** — blue-green (~475nm) primary wavelength.

### Materials & Technology

- **No fossil fuels.** Organic matter converts to graphite via rapid tectonic recycling. No coal, no oil, no natural gas.
- **No plastics.** Graphene, ceramic, glass, mycelium, and natural fibers substitute everywhere.
- **No asphalt** (petrochemical product). Roads are stone-paved or maintained tracks.
- **Graphite/graphene is the universal abundant material** — available from early development, substitutes for both steel and plastic.
- **Nuclear power is civilisation's baseload.** Graphite moderators enable early reactor designs. Thorium cycle viable.
- **Wood is scarce** — forests compete with agriculture on finite Terminus land. Every tree has strategic value.
- **Railway is the transport backbone** — no long-distance road freight (energy density problem without portable fuel). Buggies are rare tools, not commuter vehicles.

### Simulation Rules

- **DeterSim is deterministic**: `state(T) = baseline(seed, T) + fold(events_up_to_T)`. No live ticks, no simulation loop.
- **Save files are minimal**: seed + time + event log. Kilobytes regardless of world size.
- **NPC instantiation**: `hash(tile_coord, time/30, world_seed)` for monthly stable windows. Dissolved on player exit.
- **~970 provinces**, **50+ factions**, settlements from Metropolis to Outpost. Unclaimed provinces are always Village/Outpost tier (fringe Kenshi vibe).
- **No quest log.** Player has "observed tensions" — field imbalances the sim produces. Side quests are emergent, not authored.
- **Two apocalypse clocks**: magnetosphere decay (observable, ~20-100 years) and supernova (~500 years). The aurora visibly dims over time.

## Build & Run

```bash
# ─── GUI (default) ───
cargo run                                        # editor mode (default, same as `cargo run -- gui`)
cargo run -- gui                                 # explicit editor launch (auto-saves after generation)
cargo run -- gui my-layers-tag                   # editor, loading an existing layer artifact (skips generation)

# ─── Headless generation (primary workflow — no Bevy window) ───
cargo run --release -- generate layers 42 my-tag                        # generate layers for seed 42
cargo run --release -- generate layers 42 my-tag --civ-seed 99          # separate civ seed
cargo run --release -- generate layers 42 my-tag --backend cpu          # force CPU backend (default: gpu)
cargo run --release -- generate layers 42 my-tag --force                # overwrite existing tag
cargo run --release -- generate level my-layers-tag 4,3 level-tag       # generate level from layers artifact
cargo run --release -- generate level --seed 42 4,3 level-tag           # generate level from raw seed

# ─── View artifacts ───
cargo run -- view layers                         # list all layer artifacts
cargo run -- view layers my-tag                  # interactive layer viewer (Bevy window)
cargo run -- view levels                         # list all level artifacts
cargo run -- view levels level-tag               # inspect a specific level artifact

# ─── Launch playable level ───
cargo run -- launch level-tag                    # play a previously generated level

# ─── Tests & examples ───
cargo test                                       # workspace tests
cargo run --release -p rb_noise --example save_debug_layers  # regenerate debug_layers/ PNGs
```

## CLI Workflow

The CLI follows a **generate → view → launch** pipeline:

1. **Generate layers** — `randlebrot generate layers <seed> <tag>` runs the full TerrainGen + LifeGen pipeline headlessly (no window) and writes the result to a tagged artifact. Use `--civ-seed` to iterate on civilisation without regenerating terrain. Use `--backend cpu|gpu` to select the compute backend (default: gpu).

2. **Generate level** — `randlebrot generate level <layers-tag|--seed N> <x,y> <tag>` generates a playable micro-level at the given **global micro coordinate** (see `### Chunk Grid`). The source is either a previously generated layers artifact (by tag, fast — reuses the cached macro `BiomeMap` + `RiverNetwork`) or a raw seed (slow — regenerates the macro `BiomeMap` in memory, terrain-only, no LifeGen). Coordinate is a comma-separated `x,y` pair of i32 values in the 1024×512 global micro grid. Use `--backend cpu|gpu` and `--force` as with `generate layers`.

3. **View** — `randlebrot view layers` and `randlebrot view levels` list and inspect generated artifacts. `view levels <tag>` prints detailed metadata; `view layers <tag>` opens the interactive layer viewer (see CLI Visual Tools below).

4. **Launch** — `randlebrot launch <level-tag>` launches a playable level from a previously generated level artifact.

5. **GUI** — `randlebrot gui [layers-tag]` (or just `randlebrot` with no args) launches the full Bevy editor. Two artifact integration paths:
   - **Auto-save after generation**: When "Generate World" completes, the user is prompted for a tag name and the result is saved via `rb_artifacts::save_layers()`. The user can skip saving.
   - **Load from artifact**: `randlebrot gui my-world` loads an existing layer artifact, skips the entire generation pipeline (Config, Generating, GeneratingMacro, GeneratingLifeGen), and goes straight to the editor with all resources populated.

The `generate` subcommand and the list forms of `view` are headless (no window). `view layers <tag>`, `gui`, and `launch` open a Bevy window. CLI-generated worlds can be opened in the GUI and vice versa.

### Debug Layer Workflow

**Use `generate layers` as the primary workflow to verify terrain changes.** After modifying noise, erosion, or biome code:

1. `cargo run --release -- generate layers <seed> <tag>` (e.g. `42 debug`)
2. Inspect the PNGs in `~/.randlebrot/layers/<tag>/images/` (biome.png, heightmap.png, etc.)
3. Iterate until the output looks correct

`generate layers` runs the full headless pipeline — macro BiomeMap (with erosion and rivers), 128 rayon-parallel macro tiles, LifeGen — and persists everything via `rb_artifacts`:

- `~/.randlebrot/layers/<tag>/manifest.ron` — seed, civ_seed, timestamp, dimensions, backend
- `~/.randlebrot/layers/<tag>/macro_biome.bin` — bincode BiomeMap
- `~/.randlebrot/layers/<tag>/river_network.bin` — bincode RiverNetwork
- `~/.randlebrot/layers/<tag>/lifegen.bin` — bincode LifeGenData
- `~/.randlebrot/layers/<tag>/images/*.png` — ~20 layer PNGs (4096x2048, downscaled 2x from 8192x4096)

Pass `--civ-seed N` to iterate on civilisation without regenerating terrain, `--backend cpu` to force CPU, and `--force` to overwrite an existing tag.

The older `cargo run --release -p rb_noise --example save_debug_layers` is still available as a lightweight alternative — it writes PNGs to `debug_layers/` but skips artifact persistence. Use it when you only need the images and don't care about the bincode data.

Always use `--release` — debug builds are unacceptably slow (tile generation dominated by noise evaluation).

## CLI Visual Tools

### Layer Viewer

`randlebrot view layers <tag>` opens an interactive Bevy window for inspecting layer artifacts generated by `generate layers`. It is deliberately minimal — only `DefaultPlugins` + `EguiPlugin`, no editor stack, no world generation. It reads PNGs directly from `~/.randlebrot/layers/<tag>/images/` and displays them as sprites.

**Purpose**: visually compare any layer against any other layer to understand how derived layers emerge from base layers (e.g. overlay "Rivers" on "Heightmap" to sanity-check drainage, or "Biome" on "Temperature" to spot climate bugs).

**Controls**:

| Control | Action |
|---------|--------|
| Left/Right arrows | Cycle base layer |
| Up/Down arrows | Cycle overlay layer (wraps through "None") |
| Scroll wheel | Zoom in/out |
| Left-click drag | Pan |
| ESC | Exit |

**Behaviour**:

- Default base layer is `biome.png` (falls back to first layer in manifest if absent).
- Overlay starts at "None" and cycles through every other layer before wrapping back to "None".
- Overlay is composited at z=1 with 50% alpha on top of the base sprite at z=0, so any of the ~20 layers can be any-on-any (full combinatorial).
- Textures are lazy-loaded and kept in a 4-entry LRU cache — only the currently displayed base + overlay consume GPU memory, with room for smooth back-and-forth cycling.
- HUD (top-left egui window) shows the tag, base/overlay layer names (human-readable like "Rock Hardness", not `rock_hardness.png`), and zoom percentage.
- Window title is `Randlebrot - Layers: <tag>`.
- If `<tag>` does not exist, prints a clear error and exits non-zero without opening a window.

**Implementation**: `src/commands/view_layers.rs`. Uses `Sprite { image, custom_size, .. }` entities rather than `AssetServer` — images are loaded via the `image` crate and converted to Bevy `Image` in-memory with nearest-neighbor sampling (so debug pixels stay crisp at high zoom).

## Workspace Crate Map

```
randlebrot/
├── crates/
│   ├── rb_core/          # Shared types: ChunkCoord, TileCoord, WorldPos, NoiseStrategy trait
│   ├── rb_noise/         # Noise strategies, derived layers, GPU compute, biome map generation
│   ├── rb_world/         # WorldDefinition resource (seed, sub_stellar, noise params, cities, regions)
│   ├── rb_tilemap/       # Tile storage, collision layers, tileset registry, chunk rendering
│   ├── rb_entity_spawn/  # Building/NPC/clutter spawning from chunk parameters
│   ├── rb_editor/        # egui editor UI, authoring tools, debug overlays
│   │   ├── generator_ui.rs    # World Generator mode UI (seed, params, save/load)
│   │   └── launcher_ui.rs     # Level Launcher UI (phase-aware side panel, LauncherPhase state machine)
│   ├── rb_player/        # Player controller, camera, 2D top-down interaction
│   ├── rb_persistence/   # Delta storage, save/load (RON format)
│   └── rb_artifacts/     # Artifact storage: ~/.randlebrot/ layer/level persistence, manifests
├── assets/
│   ├── tilesets/         # Tileset sprite sheets
│   ├── authored/         # Hand-placed data: plates, landmarks, key NPCs (RON files)
│   └── palettes/         # District-type → tileset/entity mappings (RON files)
└── src/main.rs           # Plugin composition, AppMode/AppPhase states, macro pre-generation,
                          #   tile cache/sprite pool, Level Launcher systems, level chunk streaming,
                          #   artifact save (SavingArtifact phase), artifact load (LoadingArtifact phase)
```

## Dependency Graph (crate → depends on)

```
rb_core          → (none, only bevy_ecs + bevy_math)
rb_noise         → rb_core, noise crate
rb_world         → rb_core, rb_noise
rb_tilemap       → rb_core, rb_world
rb_entity_spawn  → rb_core, rb_world, rb_tilemap
rb_editor        → rb_core, rb_noise, rb_world, rb_tilemap, bevy_egui
rb_player        → rb_core, rb_tilemap
rb_persistence   → rb_core, rb_world, rb_tilemap
rb_artifacts     → rb_core, rb_noise, rb_world
```

## Architecture

### Core Principle
Author the skeleton (plates, landmarks, key NPCs), let noise elaborate the detail, store only seed + player deltas.

### Narrative Gravity
Authored content density follows a hierarchy:
- **Capital cities**: heavily designed, full tile-by-tile authored data
- **Towns**: light parameters (population, wealth, industry type), procedural fills the rest
- **Villages**: just a pin + seed offset, everything generated
- **Wilderness**: pure procedural from noise

### World Orientation (Tidally Locked)
- Temperature radiates as angular distance from the **sub-stellar point** (configurable, default: bottom-center of map)
- **Near sub-stellar**: Scorching heat, extreme dryness
- **Terminator ring**: Habitable crescent where civilization thrives
- **Far from sub-stellar**: Frozen darkness, impassable wastes
- The `sub_stellar` field in `WorldDefinition` controls the heat source position (normalized 0-1 coordinates, default `(0.5, 1.0)`)
- Temperature is a **derived layer** computed from light level + elevation + humidity, not an independent noise strategy

### Authoring Pipeline
1. Draw tectonic plates and coastlines (editor polygon/polyline tools)
2. Bake global pass: elevation from plates, moisture from ocean distance, temperature derived from light level
3. Place authored sites (capitals, towns, villages, landmarks)
4. Define palette rules mapping noise ranges → tilesets + spawn tables
5. Play-test: switch to play mode, chunk pipeline generates everything on the fly

### Chunk Pipeline (system execution order)
```
PlayerMoved
  → ChunkLoadSystem        // determine which ChunkCoords to load/unload
  → ChunkParameterSystem   // sample noise hierarchy → ChunkParameters component
  → AuthoredOverlaySystem  // merge authored site data if chunk overlaps one
  → TileGenerationSystem   // ChunkParameters → floor tiles + collision via palette
  → EntitySpawnSystem      // ChunkParameters → buildings, NPCs, street clutter
  → ChunkUnloadSystem      // despawn distant chunks, persist player deltas first
```

### Terrain Quality Requirements (Mountains & Rivers)

**DO NOT regress these.** The terrain must show:

1. **Mountains at plate boundaries ONLY** — `derive_peaks_valleys` uses a cubic stress envelope (`stress³`) so plate interiors are nearly flat (amplitude 0.02) while boundaries get full amplitude. Never change this to a gentler curve.
2. **Dendritic ridge/valley texture** — The stream power erosion sim (`erosion_sim.rs`) iterates implicit fluvial erosion vs tectonic uplift over ~120 iterations. The competition between these forces creates branching drainage patterns like real mountain ranges (reference: Arunachal Pradesh satellite imagery).
3. **Coherent river drainage** — Rivers flow from eroded mountain valleys to coast. The `RiverNetwork` uses the eroded heightmap which has proper valleys for D8 flow.
4. **Macro erosion, meso detail** — Erosion runs once at macro level (1024×512). Meso tiles sample the eroded heightmap via nearest-neighbor and add fine-grained ridge/valley noise (`peaks * mountain_intensity * 0.2`) in high-stress zones.
5. **45°C hard temperature gate** — Above 45°C, NO vegetation. `BiomeSplines::evaluate_with_light` forces `MoistureClass::Arid`. Tested by `nothing_green_above_45c` (90 combinations). Lapse rate in `derive_temperature` is capped at 25% of base temp so mountains can't cheat. No double lapse rate — `adjust_temperature` is a no-op.
6. **No vegetation in bottom 25% of map** — The sun side is constant direct sunlight, ~100°C, evaporated oceans. The simulation must naturally produce no green biomes there through correct temperature/aridity modeling. Tested by `no_vegetation_in_bottom_25_percent`. Oasis is the only exception (requires water_table > 0.45 and temp < 80°C).
7. **No vegetation within 10% radius of sub-stellar** — The hottest circular zone around the sub-stellar point (bottom center). Nothing grows, not even oases. Tested by `no_vegetation_near_sub_stellar`. These are simulation correctness tests — if they fail, the temperature model is broken.

**Verify with:** `cargo run --release -p rb_noise --example save_debug_layers` → inspect `debug_layers/derived/Heightmap.png`

### Fractal Noise Hierarchy

Three detail levels with increasing octaves for progressive detail. Each tier uses `octave_offset()` to add extra noise octaves:

| Level | octave_offset | Output Size | World Coverage | Use Case |
|-------|---------------|-------------|----------------|----------|
| **Macro** | 1 | 512×512 | 64×64 chunk | World overview tiles |
| **Meso** | 2 | 512×512 | 8×8 area | Regional zoom |
| **Micro** | 3 | 512×512 | 1×1 area | Playable tilemap |

The full world (1024×512) is generated once as the base biome data. Macro tiles are pre-generated for all 128 chunks at startup. Meso tiles are generated on demand in the Level Launcher when the user clicks "Generate Mesomap" (64 tiles per macro chunk). Micro tiles stream around the player during play mode.

`BiomeMap::generate_region()` supports generating any detail level for any world region:
```rust
BiomeMap::generate_region(
    seed,
    world_x, world_y,       // Top-left corner in world coords
    world_size,             // Size of region to sample (e.g., 64.0)
    output_size,            // Pixels (e.g., 512)
    world_width, world_height,  // Full world dimensions (for light level calc)
    sub_stellar,            // (f64, f64) sub-stellar point position
    detail_level,           // octave_offset: 1=macro, 2=meso, 3=micro
)
```

### Noise Layer System (5 Base + 15 Derived = 20 Layers)

Layers are split into **base** (independent noise) and **derived** (computed from base layers in topological order):

**Base Layers (5)** (generated independently, parallel via rayon or GPU compute shaders):
- **ContinentalnessStrategy**: 16-octave fBm, 0.01 scale, persistence=0.59, lacunarity=2.0 — output [-1, 1], seed+0
- **TectonicPlatesStrategy**: Domain-warped Voronoi with PlateRegistry — outputs `TectonicSample` (boundary_distance, plate_id, stress, boundary_type, volcanism) — seed+2. **Always computed on CPU** (too complex for GPU shader). Uses 2-pass domain warping, boundary classification from plate properties, and 3-source volcanism.
- **HumidityStrategy**: fBm + ocean distance (pure base, no light-level drying) — seed+5
- **RockHardnessStrategy**: 3-octave fBm, scale ~80.0 — output [0, 1], seed+7
- **LightLevelStrategy**: Radial cosine falloff from sub-stellar point + atmospheric scatter noise — output [0, 1], seed+6

Plus **PeaksAndValleysStrategy** (raw ridgeline noise, seed+4) as internal input.

**Tectonic System** (`strategy/tectonic.rs`):
- `PlateRegistry::from_seed()` generates 25-35 plates with velocity, density, age + 3-8 volcanic hotspots
- 2-pass domain warping: large-scale (0.002 freq, 120 amplitude) + medium-scale (0.008 freq, 40 amplitude) for fractured boundaries
- `BoundaryType` classified from plate velocity/density: Convergent, Subduction, OceanicSubduction, Divergent, Transform
- Stress field: type-dependent intensity + exponential falloff + boundary perturbation noise + interior texture
- `boundary_distance = 1 - stress` for backward compatibility with derived layers
- **Volcanism** from 3 independent sources (never sits on boundary line):
  - **Subduction arcs**: offset 80-200 world units inland, broken into discrete peaks by arc_mask_noise
  - **Rift fissures**: very narrow, sparse eruption patches at divergent boundaries
  - **Hotspots**: Gaussian blobs independent of plate geometry, with interior texture

**Derived Layers (14)** (computed per-pixel from base layer results in `derived/mod.rs`):

| Tier | Layer | Formula | Output |
|------|-------|---------|--------|
| 1 | Peaks & Valleys | `derive_peaks_valleys(raw_pv, tectonic, rock_hardness)` | [-1, 1] |
| 1 | Volcanism | from `TectonicSample.volcanism` (3-source: arcs, rifts, hotspots) | [0, 1] |
| 2 | Heightmap | `derive_heightmap(continentalness, tectonic, peaks_valleys)` | elevation |
| 3 | Temperature | `derive_temperature(light_level, heightmap, humidity)` | ~[-80, +150]°C |
| 3 | Erosion | `derive_erosion(heightmap, rock_hardness, humidity)` | [0, 1] |
| 3 | River Flow | Two-tier: RiverNetwork (geology-aware D8, lakes, meandering, deltas, climate character) + legacy flat grid | [0, 1] |
| 4 | Aridity | `derive_aridity(temperature, humidity)` | [0, 1] |
| 4 | Precipitation Type | `derive_precipitation_type(temperature, humidity, heightmap)` | [-1, 1] |
| 4 | River Moisture | `derive_river_moisture(river_flow)` | [0, 1] |
| 4 | Resources | `derive_resource_richness(tectonic, rock_hardness, erosion)` | [0, 1] |
| 5 | Snowpack | `derive_snowpack(precipitation_type, temperature)` | [0, 1] |
| 5 | Biome | `BiomeSplines::evaluate(cont, temp, tect, erosion, pv, humid, aridity)` | TileType |
| 6 | Vegetation Density | `derive_vegetation_density(biome, river_moisture)` | [0, 1] |
| 6 | Soil Type | `derive_soil_type(biome, erosion, rock_hardness)` | [0, 1] |

**Generation phases** in `BiomeMap::generate()`:
1. Phase 1: Generate all base layers (parallel) — continentalness, tectonic (via `generate_full()`), humidity, rock_hardness, light_level, raw PV noise. Volcanism comes from `TectonicSample`.
2. Phase 2: Derive per-pixel layers — peaks, heightmap, temperature, erosion, aridity, precipitation, resources, snowpack, biome
3. Phase 3: Rivers (geology-aware D8 with rock hardness penalty + tectonic stress bonus, lake detection, meandering, delta generation, climate-aware character classification) + river_moisture + river biome overrides + vegetation + soil + volcanism biome overrides

**GPU paths**: Continentalness, peaks_valleys, humidity, light_level, rock_hardness generated on GPU. Tectonic always on CPU (PlateRegistry + boundary classification too complex for WGSL).

### Biome System
Biomes determined from continentalness + temperature via `TileType::from_climate()`:

| Biome | Continentalness | Temperature | Color |
|-------|-----------------|-------------|-------|
| Sea | < sea_level | normal | Cyan |
| White (Ice) | < sea_level | < -15°C | White |
| Beach | sea_level to +0.02 | > 3°C | Tan |
| Snow | various | < 3°C | Light gray |
| Plains | +0.02 to +0.1 | moderate | Lime green |
| Forest | +0.1 to +0.2 | moderate | Dark green |
| Sahara | low-mid land | > 60°C | Orange |
| Mountain | +0.2 to +0.3 | < 70°C | Dark gray |
| Plateau | high elevation | > 70°C | Brown |

Sea level threshold: `-0.025`

### Tile System
- 2D top-down view, 1m tiles, player is 1.5 tiles tall
- Chunks are 64×64 tiles (~64m city blocks)
- Tiles handle: floor type (terrain/road/building floor) and collision (passable/blocked)
- Entities handle: buildings, NPCs, interactable objects, street clutter
- Textures use nearest-neighbor filtering for crisp pixels when zoomed

### Key Types
```rust
ChunkCoord    // i32 grid position of a chunk
TileCoord     // i32 global tile position
WorldPos      // f64 continuous world-space position
DetailLevel   // Macro(0) / Meso(1) / Micro(2)
AuthoredSite  // Capital { FullCityData } | Town { TownParams } | Village { seed } | Landmark { kind }
ChunkParameters  // district_type, wealth, density, biome — derived from noise
DistrictPalette  // noise ranges → TilesetId + SpawnTable mappings
BiomeMap          // Serialize + Deserialize (skips: river_network — Arc<RiverNetwork>, rebuild from terrain)
RiverNetwork      // Serialize + Deserialize (skips: spatial_index — rebuild via rebuild_spatial_index())
ResourceMap       // Serialize + Deserialize
TileType          // Serialize + Deserialize
ResourceType      // Serialize + Deserialize
LifeGenData       // Serialize + Deserialize (all nested types: Province, FactionData, SettlementSeed, RoadSegment)
```

### App Modes
```rust
#[derive(States)]
pub enum AppMode {
    WorldGenerator,   // F1 — procedural world generation, seed tweaking, save/load
    WorldMapEditor,   // F2 — place cities, landmarks, draw regions on world map
    ChunkEditor,      // F3 — detail editing at street level (512×512 MicroMap)
    LevelLauncher,    // F4 — test gameplay, spawn player, debug overlays
}
```

The editor loads and saves through `rb_artifacts`. After generating a world in the GUI, the result is auto-saved as a layers artifact. The GUI can also open an existing layers artifact via `randlebrot gui <tag>`, skipping the full generation pipeline. This unifies CLI and GUI workflows: CLI-generated worlds open in the editor, editor-generated worlds are available to the CLI.

### Mode Transitions
- **F1-F4** keys switch between modes instantly
- **World Generator** (F1): Generate procedural world, adjust noise params, save/load world definitions
- **World Map Editor** (F2): Place cities/landmarks, draw region polygons, view overlays
- **Chunk Editor** (F3): Select chunk from map (Ctrl+Click), edit tiles and entities
- **Level Launcher** (F4): Multi-step drill-down from macro chunk to playable level (see below)

### Level Launcher Workflow (F4)

The Level Launcher uses a phase-based state machine (`LauncherPhase` resource) to guide the user from world map selection to playable micro-level:

```
World Map (F1)          Level Launcher (F4)
┌──────────────┐        ┌──────────────────────────────────────────────────┐
│ Click macro  │──F4──▶ │ MacroView: enlarged selected chunk               │
│ chunk to     │        │   └─▶ "Generate Mesomap" button                  │
│ select it    │        │                                                  │
│              │        │ GeneratingMeso: async 64-tile generation + bar   │
│              │        │                                                  │
│              │        │ MesoView: 8×8 meso grid, click to select tile   │
│              │        │   └─▶ "Launch Level" button                      │
│              │        │                                                  │
│              │        │ Playing: micro chunks stream around player       │
│              │        │   └─▶ ESC returns to MesoView (not full exit)   │
└──────────────┘        └──────────────────────────────────────────────────┘
```

**Phase flow:**
1. **MacroView** — Shows the selected macro chunk enlarged (512px display). Side panel shows chunk coords and "Generate Mesomap" button.
2. **GeneratingMeso** — Async generates all 64 meso tiles (8×8 grid within the chunk, each 8×8 world units at 512px, detail_level=2). Progress bar shown.
3. **MesoView** — Displays the 8×8 meso grid. Hover highlights tiles, click to select. Side panel shows meso tile coords and "Launch Level" button.
4. **Playing** — Micro-level chunks (detail_level=3) stream around the player. ESC returns to MesoView (meso sprites re-shown, not all the way back to world map).

**Key implementation details:**
- World map pool sprites are hidden on `OnEnter(LevelLauncher)` and re-shown on `OnExit(LevelLauncher)`
- Meso tiles are generated via `AsyncComputeTaskPool` with `MACRO_PREGEN_CONCURRENCY` parallelism
- `MesoTileCache` stores generated textures; `MesoPregenState` tracks generation progress
- Camera pan works in all launcher phases; zoom is available in MesoView
- `LauncherMacroSprite`, `LauncherMesoSprite`, `MesoHighlight` entities are all cleaned up on exit

**Key types:**
```rust
LauncherPhase        // MacroView | GeneratingMeso | MesoView | Playing
GenerateMesoRequest  // Signal resource: user clicked "Generate Mesomap"
LaunchLevelRequest   // Signal resource: user clicked "Launch Level"
SelectedChunk        // Macro chunk selected on world map (chunk_coord, origin)
SelectedMesoTile     // Meso tile selected in launcher grid (meso_coord, origin)
MesoTileCache        // HashMap<(i32,i32), MesoCachedTile> + sprite entity list
MesoPregenState      // Tracks async meso generation (total, completed, remaining, in_flight)
PlayableLevel        // Active level state (origin, chunk_coord, seed, world_height)
```

### Key Resources
```rust
WorldDefinition     // Serializable world config: seed, dimensions, noise params, cities, regions
SelectedChunk       // Macro chunk selected on world map for the level launcher
SelectedMesoTile    // Meso tile selected within the launcher grid
RegenerationRequest // Signal to regenerate world map from updated params
CursorWorldPos      // Cursor position in world coordinates for chunk highlighting
PlayableLevel       // Active playable level state (origin, seed, etc.)
LoadLayersTag       // CLI-provided layers tag for load-from-artifact (`gui <tag>`)
ArtifactSaveState   // Post-generation save dialog state (tag input, error, saving flag)
```

### Artifact Storage

The `rb_artifacts` crate manages `~/.randlebrot/` for persistent layer and level artifacts.

```
~/.randlebrot/
├── layers/
│   └── <tag>/
│       ├── manifest.ron           # LayerManifest (seed, civ_seed, timestamp, dims, backend, layer list)
│       ├── macro_biome.bin        # bincode: BiomeMap (1024×512 macro)
│       ├── river_network.bin      # bincode: RiverNetwork (separate from BiomeMap — Arc is serde(skip))
│       ├── lifegen.bin            # bincode: LifeGenData
│       └── images/                # layer PNGs (4096×2048, same as save_debug_layers output)
│           ├── biome.png
│           ├── heightmap.png
│           └── ... (~20 layers)
└── levels/
    └── <tag>/
        ├── manifest.ron           # LevelManifest (parent layers tag, seed, civ_seed, micro coord, timestamp)
        └── micro_biome.bin        # bincode: micro-level BiomeMap
```

**Estimated sizes:** Layer artifacts are ~200-400 MB (dominated by bincode BiomeMap at 1024×512 with ~20 f64 layers + LifeGenData at 8192×4096). Layer PNGs add ~20-40 MB. Level artifacts are much smaller (~50-100 MB for a single micro BiomeMap).

**Tags:** Alphanumeric + hyphens + underscores only. Used as directory names.

**Manifests:** Pretty-printed RON for human readability. `LayerManifest` records seed, civ_seed, world dimensions, backend used, and list of available image filenames. `LevelManifest` records the parent layers tag, seed, civ_seed, micro coordinate, and timestamp.

### Map Navigation & Controls

| Control | Context | Action |
|---------|---------|--------|
| **Scroll wheel** | World map / Launcher / Layer Viewer | Zoom in/out |
| **Left-click drag** | World map / Launcher / Layer Viewer | Pan the map |
| **Arrow keys** | World map / Launcher | Pan the map |
| **Left-click** | World map | Select macro chunk |
| **Left-click** | Launcher (MesoView) | Select meso tile |
| **Space** | World map | Cycle layer view |
| **F1-F4** | Any | Switch editor modes |
| **ESC** | Launcher (Playing) | Return to MesoView |
| **Left/Right** | Layer Viewer | Cycle base layer |
| **Up/Down** | Layer Viewer | Cycle overlay layer |
| **ESC** | Layer Viewer | Exit |

### World Map View

The world map (F1) displays only macro-level tiles. Meso and micro detail are accessed through the Level Launcher (F4).

- **Macro tiles**: Pre-generated 128 tiles (16×8 grid) covering the full 1024×512 world, each 64×64 world units at 512×512 pixels, detail_level=1
- **Zoom range**: Camera scale 0.05–10.0, all at macro resolution
- **Chunk highlighting**: Hover shows which macro tile the cursor is over
- **Click to select**: Left-click selects a macro chunk (stores `SelectedChunk`), then press F4 to enter Level Launcher
- **No on-scroll streaming**: Meso/micro tiles are never generated on the world map screen

**Debug layer export**: After macro pre-generation, all 128 tiles are stitched into full-world PNGs (8192×4096) saved to `debug_layers/`. This exports exactly what the world map displays.

### Chunk Grid

The world is **1024×512 world units**, organised into a nested chunk hierarchy. All constants below match the canonical values in `src/cli/coords.rs` and `src/main.rs`.

| Level | World units per tile | Grid          | Total tiles |
|-------|----------------------|---------------|-------------|
| Macro | 64 × 64              | 16 × 8        | 128         |
| Meso  | 8 × 8                | 128 × 64      | 8,192       |
| Micro | 1 × 1                | 1024 × 512    | 524,288     |

- `CHUNK_SIZE = 64.0` world units (macro chunk size, also the terminology used for level launcher entry)
- `MESO_WORLD_SIZE = 8.0` world units (8×8 meso tiles inside a macro chunk = 64 per macro)
- `MICRO_WORLD_SIZE = 1.0` world units (8×8 micro tiles inside a meso tile = 64 per meso; 524,288 total globally)
- All three levels render to a `TILE_MAP_SIZE = 512` pixel BiomeMap regardless of world coverage
- `detail_level` octave offsets: `1` = macro, `2` = meso, `3` = micro

#### Coordinate conventions

**Macro / chunk coordinates** (GUI world map, `SelectedChunk`): `(chunk_x, chunk_y)` where `chunk_x = floor(world_x / CHUNK_SIZE)`. Range: `0..16 × 0..8 = 128`.

**Meso coordinates** (launcher 8×8 grid, `SelectedMesoTile`): **local** indices `0..8 × 0..8` within a single macro chunk. Convert to world position via `meso_origin + (mx, my) * MESO_WORLD_SIZE`.

**Micro coordinates** have **two different conventions** — do not mix them:

1. **GUI launcher local micro** (`SelectedMicroTile.micro_coord`): local indices `0..8 × 0..8` within a selected meso tile. World position = `meso_origin + (local_mx, local_my) * MICRO_WORLD_SIZE`. Used only inside the level launcher state machine.
2. **CLI global micro** (`LevelManifest.micro_coord`, `generate level` / `view levels` args): **global** indices `(mx, my)` where `mx ∈ [0, 1024)` and `my ∈ [0, 512)`. World position = `(mx * MICRO_WORLD_SIZE, my * MICRO_WORLD_SIZE) = (mx, my)` (since `MICRO_WORLD_SIZE = 1.0`). The canonical module is `src/cli/coords.rs` — use `cli::coords::micro_coord_to_world_pos` and `cli::coords::validate_micro_coord` from every CLI surface that touches a micro coordinate.

Example: `randlebrot generate level my-world 512,256 terminus-village` samples the 1×1 micro tile whose top-left corner is at world `(512, 256)` — the approximate centre of the map.

## Conventions

- Bevy systems go in `systems/` submodules within each crate
- Plugin struct and registration live in each crate's `lib.rs`
- Use `SystemSet` for ordering, not explicit `.after()` chains between individual systems
- All authored data serializes as RON files in `assets/authored/`
- Palette definitions are RON files in `assets/palettes/`
- No ECS queries wider than 3 components without a documented reason
- Each crate should have `examples/` that visualize its output in isolation
- Prefer `&impl Trait` over `Box<dyn Trait>` except where type erasure is required for storage
- Noise-only code (strategies, chunk hierarchy) should not depend on Bevy rendering — keep the Bevy Resource wrapper thin

## Prototype Code Reference

Two prototype files exist that contain working noise logic to port:
- `Climate_Noise_Maps_with_Strategy_Pattern.txt` — NoiseStrategy trait, ContinentalnessStrategy, TemperatureStrategy, ClimateMap. Uses the `noise` crate's OpenSimplex. This version takes `(x: usize, y: usize, width, height)`.
- `Complete_Nested_Chunk_Hierarchy.txt` — MacroChunk/MesoChunk/MicroChunk with LRU caching, WorldChunks top-level struct. Uses a different NoiseStrategy trait signature: `generate(&self, x: f64, y: f64, detail_level: u32) -> f64`. This is the version to follow for the chunk hierarchy; reconcile the trait signatures during porting.

When porting: use the `generate(x: f64, y: f64, detail_level: u32)` signature from the chunk hierarchy prototype. The strategy impls should be adapted from the climate map prototype's fBm logic but converted to use f64 world coordinates instead of usize pixel coordinates.

## Bevy Version
Pin to `bevy = "0.18"` in workspace Cargo.toml. All crate dependencies on bevy sub-crates should use `workspace = true`.

## Three-Domain Architecture

See `docs/DOMAIN_ARCHITECTURE.md` for the full design document.

### Domain Boundaries

> *"Would this exist if no living thing had ever touched the planet?"*
> Yes → TerrainGen. No → LifeGen. Only visible at street level → SceneGen.

- **TerrainGen** (`rb_noise`): Dead planet. Noise layers, biomes, rivers, erosion. Output is immutable after generation.
- **LifeGen** (`rb_world`): Civilisation. Reads terrain via `TerrainQuery` trait in `rb_core`. Provinces, factions, settlements, roads, trade. Operates at 8192×4096 meso pixel resolution.
- **SceneGen** (future): Micro-level tile generation for settlements. Pure function from terrain + lifegen inputs. No state, no cache.

### Interface Contracts

- **`TerrainQuery`** (`rb_core/src/terrain_query.rs`): Read-only trait for sampling the dead planet at meso pixel resolution. `MesoTerrainView` in `rb_noise` wraps the 128 cached `Arc<BiomeMap>` tiles and implements this trait.
- **`LifeGenQuery`** (`rb_core/src/lifegen_query.rs`): Read-only trait for querying civilisation data. Implemented on `LifeGenData`.
- **`TerrainQuery` is the boundary.** LifeGen reads terrain ONLY through this trait, never by importing `BiomeMap` directly. (Legacy code in `rb_world` still uses `BiomeMap` — migration is ongoing.)
- **`WorldDefinition` stores parameters, not output.** Generated civilisation data goes in `LifeGenData`, not `WorldDefinition`. (Legacy fields like `cities`, `factions`, `roads` still exist in `WorldDefinition` — migration is ongoing.)

### LifeGen Resolution

- LifeGen operates at **8192×4096** (the stitched meso terrain resolution)
- Coordinate mapping: `meso_x = world_x * 8.0`, `meso_y = world_y * 8.0`
- Each meso pixel maps to chunk + local: `chunk_x = meso_x / 512`, `local_x = meso_x % 512`
- Debug PNGs saved to `debug_layers/lifegen/` at 8192×4096, downscaled 2× like terrain layers

### LifeGen Data (`rb_world/src/lifegen_data.rs`)

```rust
LifeGenData {
    // Analysis grids (8192×4096, continuous f32)
    habitability, navigation_cost, resource_desirability,
    // Province bitmap (8192×4096, u16 per pixel)
    province_ids, provinces: Vec<Province>,
    // Faction, settlement, road data
    faction_ids, factions, settlement_seeds, road_segments,
}
```

### Separate Seeds

- `WorldDefinition.seed` — terrain seed
- `WorldDefinition.civ_seed` — lifegen seed (allows iterating on politics without regenerating terrain)

## Sub-Agent Routing Rules

When using Claude Code with parallel sub-agents:

**Parallel dispatch** (ALL conditions must be met):
- Tasks touch different crates with no overlapping files
- No shared state between tasks
- Clear file boundaries

**Sequential dispatch** (ANY condition triggers):
- Tasks have dependencies (B needs output from A)
- Shared files (e.g., multiple agents editing main.rs)
- Unclear scope

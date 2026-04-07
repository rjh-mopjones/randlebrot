# Randlebrot

A Bevy 0.18 game engine for **Margin's Grip** — a 2D open-world survival RPG set on a tidally locked planet orbiting a red supergiant. Fractal noise serves as a compression algorithm for plausibility—not generating a random world, but filling in infinite detail for a handcrafted design.

Randlebrot is the engine; Margin's Grip (mgrip) is the game. The engine handles procedural world generation (TerrainGen, LifeGen, SceneGen), the deterministic simulation (DeterSim), and the editor/launcher tooling. The game itself will live in a separate private repo.

## Design Sources of Truth

**The Obsidian vault** at `/Users/roryhedderman/Documents/mop-jones-brain/Notes/` (files prefixed "Margin's Grip - ") is the source of truth for all game design, worldbuilding, and gameplay decisions.

**The `docs/` folder** in this repo is the source of truth for implementation details and technical architecture.

| Document | Covers |
|----------|--------|
| `docs/TERRAIN_DESIGN.md` | Noise layer system, base/derived layer architecture, river system, chunk hierarchy, GPU acceleration, seeding conventions |
| `docs/DOMAIN_ARCHITECTURE.md` | Three-domain split (TerrainGen/LifeGen/SceneGen), interface contracts, `LifeGenData` schema, generation pipeline, coordinate systems |

**The `specs/` folder** contains structured specification files for open work items. GitHub issues track STATUS; specs contain CONTENT (exact code, file paths, verification commands). When implementing an issue, check `specs/<issue-number>-*.md` first — if a spec exists, it takes priority over the issue body.

**When they conflict:** the Obsidian vault wins for design intent; the repo docs win for how things are currently implemented.

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

- **No fossil fuels.** No coal, no oil, no natural gas.
- **No plastics.** Graphene, ceramic, glass, mycelium, and natural fibers substitute everywhere.
- **No asphalt** (petrochemical product). Roads are stone-paved or maintained tracks.
- **Graphite/graphene is the universal abundant material.**
- **Nuclear power is civilisation's baseload.** Graphite moderators, thorium cycle viable.
- **Wood is scarce** — every tree has strategic value.
- **Railway is the transport backbone** — no long-distance road freight.

### Simulation Rules

- **DeterSim is deterministic**: `state(T) = baseline(seed, T) + fold(events_up_to_T)`. No live ticks, no simulation loop.
- **Save files are minimal**: seed + time + event log. Kilobytes regardless of world size.
- **NPC instantiation**: `hash(tile_coord, time/30, world_seed)` for monthly stable windows. Dissolved on player exit.
- **~970 provinces**, **50+ factions**, settlements from Metropolis to Outpost. Unclaimed provinces are always Village/Outpost tier.
- **No quest log.** Side quests are emergent from sim field imbalances.
- **Two apocalypse clocks**: magnetosphere decay (~20-100 years) and supernova (~500 years).

## Build & Run

```bash
# ─── GUI (default) ───
cargo run                                        # editor mode
cargo run -- gui my-layers-tag                   # load existing artifact (skips generation)

# ─── Headless generation ───
cargo run --release -- generate layers 42 my-tag                  # full TerrainGen + LifeGen pipeline
cargo run --release -- generate layers 42 my-tag --civ-seed 99    # separate civ seed
cargo run --release -- generate layers 42 my-tag --force          # overwrite existing tag
cargo run --release -- generate level my-layers-tag 4,3 level-tag # level from layers artifact
cargo run --release -- generate level --seed 42 4,3 level-tag     # level from raw seed

# ─── View & Launch ───
cargo run -- view layers [my-tag]                # list artifacts or open interactive layer viewer
cargo run -- view levels [level-tag]             # list artifacts or inspect a level
cargo run --release -- launch level-tag          # 3D terrain flyover
cargo run --release -- launch level-tag --flythrough  # automated screenshots → /tmp/randlebrot_flythrough/

# ─── Tests & examples ───
cargo test
cargo run --release -p rb_noise --example save_debug_layers  # regenerate debug_layers/ PNGs
```

**Always use `--release`** — debug builds are unacceptably slow.

## CLI Workflow

The CLI follows a **generate → view → launch** pipeline. All generate/list commands are headless; `view layers <tag>`, `gui`, and `launch` open a Bevy window.

**Debug layer workflow** — primary way to verify terrain changes:
1. `cargo run --release -- generate layers <seed> <tag>`
2. Inspect PNGs in `~/.randlebrot/layers/<tag>/images/`
3. Iterate until correct

Artifacts are stored in `~/.randlebrot/`:
- `layers/<tag>/` — `manifest.ron`, `macro_biome.bin`, `river_network.bin`, `lifegen.bin`, `images/*.png` (~20 layers, 4096×2048)
- `levels/<tag>/` — `manifest.ron`, `micro_biome.bin`

Use `--civ-seed N` to iterate on civilisation without regenerating terrain. `--force` overwrites an existing tag.

## CLI Visual Tools

**Layer Viewer** (`src/commands/view_layers.rs`): `randlebrot view layers <tag>` — Bevy window displaying any layer PNG from the artifact. Select base + overlay layers via egui combo boxes, adjust overlay opacity. Scroll to zoom, drag to pan. Nearest-neighbor sampling keeps debug pixels crisp. 4-entry LRU texture cache.

**Level Launcher** (`src/commands/launch.rs`): `randlebrot launch <level-tag>` — Minecraft-style 3D block terrain. `Camera3d` (terrain) + `Camera2d` (egui HUD). Chunks stream asynchronously. `HEIGHT_SCALE = 128` (absolute, cross-chunk consistent). Greedy meshing for top faces, depth-based side shading. Press M for world map overlay. macOS Sequoia: `.app` trampoline at `/tmp/Randlebrot.app` for keyboard focus (skipped in `--flythrough` mode).

**Visual changes to the launcher MUST be verified via flythrough** — `cargo run --release -- launch <tag> --flythrough` saves 10 waypoint frames to `/tmp/randlebrot_flythrough/frame_NNN.png`.

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
│   ├── rb_artifacts/     # Artifact storage: ~/.randlebrot/ layer/level persistence, manifests
│   └── rb_voxel/         # Voxel terrain utilities (raycaster + player marker; launch.rs uses Bevy 3D mesh rendering)
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
rb_voxel         → rayon (no Bevy, no rb_core)
```

## Architecture

### Core Principle
Author the skeleton (plates, landmarks, key NPCs), let noise elaborate the detail, store only seed + player deltas.

### Narrative Gravity
- **Capital cities**: heavily designed, full tile-by-tile authored data
- **Towns**: light parameters (population, wealth, industry type), procedural fills the rest
- **Villages**: just a pin + seed offset, everything generated
- **Wilderness**: pure procedural from noise

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
2. **Dendritic ridge/valley texture** — The stream power erosion sim (`erosion_sim.rs`) iterates implicit fluvial erosion vs tectonic uplift over ~120 iterations. The competition creates branching drainage patterns like real mountain ranges.
3. **Coherent river drainage** — Rivers flow from eroded mountain valleys to coast. The `RiverNetwork` uses the eroded heightmap which has proper valleys for D8 flow.
4. **Macro erosion, meso detail** — Erosion runs once at macro level (1024×512). Meso tiles sample the eroded heightmap via nearest-neighbor and add fine-grained ridge/valley noise in high-stress zones.
5. **45°C hard temperature gate** — Above 45°C, NO vegetation. `BiomeSplines::evaluate_with_light` forces `MoistureClass::Arid`. Tested by `nothing_green_above_45c` (90 combinations).
6. **No vegetation in bottom 25% of map** — ~100°C, evaporated oceans. Tested by `no_vegetation_in_bottom_25_percent`. Oasis is the only exception (water_table > 0.45 and temp < 80°C).
7. **No vegetation within 10% radius of sub-stellar** — tested by `no_vegetation_near_sub_stellar`. If these fail, the temperature model is broken.

**Verify with:** `cargo run --release -p rb_noise --example save_debug_layers` → inspect `debug_layers/derived/Heightmap.png`

### Fractal Noise Hierarchy

See `docs/TERRAIN_DESIGN.md` for the full noise layer system (5 base + 14 derived layers, seeding conventions, GPU paths).

Three detail levels:

| Level | octave_offset | World Coverage | Use Case |
|-------|---------------|----------------|----------|
| **Macro** | 1 | 64×64 chunk | World overview tiles (128 pre-generated) |
| **Meso** | 2 | 8×8 area | Regional zoom (on-demand in launcher) |
| **Chunk** | 3 | 1×1 area | Playable tilemap |

**Micro-scale octave splitting** (detail_level=3): `derive_micro_heightmap` independently normalizes fBm octaves 12-17 and adds them with terrain-type amplitude budgets (0.05–0.25). This produces visible block-level height variation without affecting macro/meso views. Detail noise uses `OpenSimplex::new(seed.wrapping_add(50))`, created once outside the pixel loop.

### Three-Domain Architecture

See `docs/DOMAIN_ARCHITECTURE.md` for the full design document.

> *"Would this exist if no living thing had ever touched the planet?"*
> Yes → TerrainGen. No → LifeGen. Only visible at street level → SceneGen.

**Key boundary**: `TerrainQuery` (`rb_core/src/terrain_query.rs`) is the read-only interface between LifeGen and terrain. LifeGen reads terrain ONLY through this trait, never by importing `BiomeMap` directly. `WorldDefinition` stores parameters, not output — generated civilisation data goes in `LifeGenData`.

Two seeds: `WorldDefinition.seed` (terrain), `WorldDefinition.civ_seed` (lifegen — iterate politics without regenerating terrain).

### Tile System
- 2D top-down view, 1m tiles, player is 1.5 tiles tall
- Chunks are 64×64 tiles (~64m city blocks)
- Tiles handle: floor type (terrain/road/building floor) and collision
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
BiomeMap         // Serialize + Deserialize (skips: river_network — Arc<RiverNetwork>, rebuild from terrain)
RiverNetwork     // Serialize + Deserialize (skips: spatial_index — rebuild via rebuild_spatial_index())
LifeGenData      // Serialize + Deserialize (all nested: Province, FactionData, SettlementSeed, RoadSegment)
// Key resources in src/main.rs:
WorldDefinition     // Serializable world config: seed, dimensions, noise params, cities, regions
SelectedChunk       // Macro chunk selected on world map for the level launcher
SelectedMesoTile    // Meso tile selected within the launcher grid
RegenerationRequest // Signal to regenerate world map from updated params
PlayableLevel       // Active playable level state (origin, seed, etc.)
```

### App Modes
```rust
#[derive(States)]
pub enum AppMode {
    WorldGenerator,   // F1 — procedural world generation, seed tweaking, save/load
    WorldMapEditor,   // F2 — place cities, landmarks, draw regions on world map
    ChunkEditor,      // F3 — detail editing at street level (512×512 chunk)
    LevelLauncher,    // F4 — test gameplay, spawn player, debug overlays
}
```

The editor loads/saves through `rb_artifacts`. `randlebrot gui <tag>` loads an existing layers artifact, skipping generation. CLI-generated worlds open in the editor and vice versa.

### Level Launcher Workflow (F4 / CLI)

**CLI path**: `randlebrot launch <level-tag>` opens a level artifact directly.

**GUI path (F4)** — phase-based state machine:

```
World Map (F1)          Level Launcher (F4)
┌──────────────┐        ┌─────────────────────────────────────────────────┐
│ Click macro  │──F4──▶ │ MacroView: enlarged selected chunk              │
│ chunk to     │        │   └─▶ "Generate Mesomap" button                 │
│ select it    │        │ GeneratingMeso: async 64-tile generation + bar  │
│              │        │ MesoView: 8×8 meso grid, click to select tile   │
│              │        │   └─▶ "Generate Chunks" button                  │
│              │        │ GeneratingMicro: async 64-tile generation + bar │
│              │        │ MicroView: 8×8 chunk grid, click to select tile │
│              │        │   └─▶ "Play" + "Save Level" buttons             │
│              │        │ Playing: chunks stream around player            │
│              │        │   └─▶ ESC returns to MicroView                  │
└──────────────┘        └─────────────────────────────────────────────────┘
```

**Save Level** coordinate conversion: `global_x = chunk_x * 64 + meso_x * 8 + micro_x`. Saved levels appear in `randlebrot view levels`.

**Key launcher types** (`LauncherPhase`, signal resources): `LauncherPhase` (MacroView|GeneratingMeso|MesoView|GeneratingMicro|MicroView|Playing), `GenerateMesoRequest`, `LaunchLevelRequest`, `StartPlayRequest`, `SaveLevelRequest`, `MesoTileCache`, `MicroTileCache`, `MesoPregenState`, `MicroPregenState`.

### Chunk Grid

The world is **1024×512 world units** in a nested hierarchy:

| Level | World units per tile | Grid       | Total tiles |
|-------|----------------------|------------|-------------|
| Macro | 64 × 64              | 16 × 8     | 128         |
| Meso  | 8 × 8                | 128 × 64   | 8,192       |
| Chunk | 1 × 1                | 1024 × 512 | 524,288     |

All three levels render to 512px BiomeMap output. `TILE_MAP_SIZE = 512`, `CHUNK_WORLD_SIZE = 1.0`.

**Coordinate conventions** — do not mix:
- **GUI launcher local** (`SelectedMicroTile.micro_coord`): local `0..8` indices within a meso tile. Used only in the launcher state machine.
- **CLI global chunk** (`LevelManifest.chunk_coord`, `generate level` args): global `cx ∈ [0, 1024)`, `cy ∈ [0, 512)`. Canonical module: `src/cli/coords.rs` — use `chunk_coord_to_world_pos` and `validate_chunk_coord` from every CLI surface.

### Map Navigation & Controls

| Control | Context | Action |
|---------|---------|--------|
| **Scroll wheel** | World map / Launcher / Layer Viewer | Zoom in/out |
| **Left-click drag** | World map / Launcher / Layer Viewer | Pan |
| **Arrow keys** | World map / Launcher | Pan |
| **Left-click** | World map | Select macro chunk |
| **Left-click** | Launcher MesoView/MicroView | Select tile |
| **Space** | World map | Cycle layer view |
| **F1-F4** | Any | Switch editor modes |
| **ESC** | Launcher Playing | Return to MicroView |
| **WASD** | Launch 3D terrain | Move (relative to camera yaw) |
| **Mouse** | Launch 3D terrain | Look (yaw + pitch, ±60°) |
| **V** | Launch 3D terrain | Toggle first/third-person camera |
| **Scroll wheel** | Launch 3D terrain | Adjust draw distance (100-800) |
| **M** | Launch 3D terrain | Toggle world map overlay |
| **ESC** | Launch 3D terrain | Exit |

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

## Bevy Version
Pin to `bevy = "0.18"` in workspace Cargo.toml. All crate dependencies on bevy sub-crates should use `workspace = true`.

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

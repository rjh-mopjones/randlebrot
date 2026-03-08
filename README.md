# Randlebrot

A Bevy 0.15 game engine for a tidally locked procedural world. Fractal noise serves as a compression algorithm for plausibility—not generating a random world, but filling in infinite detail for a handcrafted design.

## Build & Run

```bash
cargo run                                        # editor mode (default)
cargo run -- --play                              # play mode
cargo test                                       # workspace tests
cargo run -p rb_noise --example noise_preview    # noise debug visualization
cargo run -p rb_tilemap --example tile_render    # tile rendering test
cargo run -p rb_editor --example editor_shell    # editor UI test
```

## Project Structure

```
randlebrot/
├── Cargo.toml                          # Workspace root, feature flags (gpu enabled by default)
├── src/
│   └── main.rs                         # App entry point, Bevy plugin composition, camera,
│                                       #   generation task spawning, meso tile cache, zoom system
├── crates/
│   ├── rb_core/                        # Shared types, no Bevy rendering dependency
│   │   └── src/
│   │       ├── lib.rs                  # Plugin registration, re-exports
│   │       ├── biome.rs                # TileType enum, from_climate(), biome colors
│   │       ├── coords.rs              # ChunkCoord, TileCoord, WorldPos
│   │       ├── mode.rs                # AppMode state (F1-F4), mode transitions
│   │       ├── noise.rs               # NoiseStrategy trait definition
│   │       └── resource_type.rs       # ResourceType enum (Iron, Gold, Timber, etc.)
│   │
│   ├── rb_noise/                       # All noise generation and terrain logic
│   │   ├── Cargo.toml                 # Optional "gpu" feature for wgpu compute shaders
│   │   ├── examples/
│   │   │   ├── noise_preview.rs       # Interactive noise visualization
│   │   │   └── save_debug_layers.rs   # Dump all layers as PNGs to debug_layers/
│   │   └── src/
│   │       ├── lib.rs                 # Plugin, re-exports
│   │       ├── biome_map.rs           # BiomeMap: generates all terrain layers (macro + meso),
│   │       │                          #   CPU and GPU paths, layer-to-RGBA conversion
│   │       ├── biome_splines.rs       # Spline-based biome classification from 6 noise inputs
│   │       ├── chunk_hierarchy.rs     # MacroChunk/MesoChunk/MicroChunk with LRU caching
│   │       ├── rivers.rs             # D8 flow accumulation, depression filling,
│   │       │                          #   macro-seeded meso river generation
│   │       ├── resource_map.rs        # Sparse resource storage per tile
│   │       ├── resource.rs            # Resource distribution logic
│   │       ├── progress.rs            # Per-layer progress tracking for UI bars
│   │       ├── tidally_locked.rs      # LatitudeTemperatureStrategy (legacy, deprecated)
│   │       ├── visualization.rs       # NoiseLayer enum, color mapping functions
│   │       ├── derived/
│   │       │   └── mod.rs             # Derived layer functions: derive_temperature,
│   │       │                          #   derive_erosion, derive_peaks_valleys, derive_heightmap
│   │       │                          #   (volcanism removed — now computed in tectonic strategy)
│   │       ├── strategy/              # NoiseStrategy implementations (fBm-based)
│   │       │   ├── mod.rs             # Re-exports all strategies
│   │       │   ├── continentalness.rs # 16-octave fBm, land vs ocean
│   │       │   ├── light_level.rs     # Radial light from sub-stellar point + scatter noise
│   │       │   ├── rock_hardness.rs   # 3-octave fBm for geological hardness
│   │       │   ├── temperature.rs     # Base temperature strategy (legacy)
│   │       │   ├── tectonic.rs        # PlateRegistry, domain-warped Voronoi, boundary
│   │       │   │                     #   classification, 3-source volcanism (arcs/rifts/hotspots)
│   │       │   ├── erosion.rs         # Erosion dependent on continentalness
│   │       │   ├── peaks_valleys.rs   # Ridgeline noise for mountains
│   │       │   ├── humidity.rs        # Moisture from ocean distance + light-level drying
│   │       │   └── resource.rs        # Per-resource-type noise with geological bias
│   │       └── gpu/                   # GPU compute shader acceleration
│   │           ├── mod.rs             # GpuNoiseResult struct (no tectonic — CPU only)
│   │           ├── context.rs         # GpuNoiseContext: wgpu device/queue, layer dispatch
│   │           ├── pipelines.rs       # WGSL shaders: continentalness, peaks, humidity,
│   │           │                      #   light_level, rock_hardness (tectonic removed)
│   │           └── perm_table.rs      # Permutation table for GPU noise
│   │
│   ├── rb_world/                       # World definition and high-level world systems
│   │   └── src/
│   │       ├── lib.rs                 # Plugin, re-exports
│   │       ├── definition.rs          # WorldDefinition resource (seed, dimensions, sub_stellar,
│   │       │                          #   noise params)
│   │       ├── settlement_placement.rs # Settlement placement logic
│   │       ├── roads.rs              # Road network generation
│   │       ├── territory.rs          # Territory/region assignment
│   │       ├── civilization.rs       # Civilization generation
│   │       ├── culture.rs            # Culture system
│   │       └── faction.rs            # Faction system
│   │
│   ├── rb_editor/                      # egui editor UI for all four modes
│   │   └── src/
│   │       ├── lib.rs                 # Plugin, re-exports
│   │       ├── generator_ui.rs        # F1: World Generator (seed, params, GPU toggle, save/load,
│   │       │                          #   layer picker: Base / Derived)
│   │       ├── map_editor_ui.rs       # F2: World Map Editor (city/landmark placement)
│   │       ├── chunk_editor_ui.rs     # F3: Chunk Editor (tile/entity tools)
│   │       ├── launcher_ui.rs         # F4: Level Launcher (play/stop, debug)
│   │       └── world_overlay.rs       # City/landmark/region rendering on map
│   │
│   ├── rb_tilemap/                     # Tile storage and rendering
│   │   └── src/
│   │       └── lib.rs                 # TileMap, collision layers, tileset registry
│   │
│   ├── rb_entity_spawn/                # Entity spawning from chunk parameters
│   │   └── src/
│   │       └── lib.rs                 # Building/NPC/clutter spawn systems
│   │
│   ├── rb_player/                      # Player controller
│   │   └── src/
│   │       └── lib.rs                 # WASD movement, camera follow, interaction
│   │
│   └── rb_persistence/                 # Save/load system
│       └── src/
│           ├── lib.rs                 # Plugin, re-exports
│           └── world_io.rs            # RON serialization, world file management
│
├── debug_layers/                       # Auto-generated debug PNGs
│   ├── aggregate.png                  # Biome aggregate view
│   ├── base/                          # Independent noise layers
│   │   ├── continentalness.png
│   │   ├── tectonic.png
│   │   ├── light_level.png
│   │   └── rock_hardness.png
│   └── derived/                       # Layers computed from base layers
│       ├── temperature.png
│       ├── erosion.png
│       ├── peaks_valleys.png
│       ├── humidity.png
│       └── rivers.png
│
└── assets/                             # Runtime assets
    ├── tilesets/                       # Tileset sprite sheets
    ├── authored/                      # Hand-placed data (RON files)
    └── palettes/                      # District-type -> tileset/entity mappings (RON files)
```

### Crate Dependency Graph

```
rb_core          → (none, only bevy_ecs + bevy_math)
rb_noise         → rb_core
rb_world         → rb_core, rb_noise
rb_tilemap       → rb_core, rb_world
rb_entity_spawn  → rb_core, rb_world, rb_tilemap
rb_editor        → rb_core, rb_noise, rb_world, rb_tilemap, bevy_egui
rb_player        → rb_core, rb_tilemap
rb_persistence   → rb_core, rb_world, rb_tilemap
```

## World Design

### Tidally Locked Planet

The world is a tidally locked planet where one hemisphere permanently faces its star. Temperature radiates as angular distance from the **sub-stellar point** (configurable, default: bottom-center of map).

- **Near sub-stellar**: Scorching heat, extreme dryness
- **Terminator ring**: Habitable crescent where civilization thrives
- **Far from sub-stellar**: Frozen darkness, impassable wastes

The `sub_stellar` field in `WorldDefinition` controls the heat source position (normalized 0-1 coordinates).

### Narrative Gravity

Authored content density follows a hierarchy:

| Location | Design Level |
|----------|--------------|
| Capital cities | Full tile-by-tile authored data |
| Towns | Light parameters, procedural fill |
| Villages | Pin + seed offset, fully generated |
| Wilderness | Pure procedural from noise |

## Editor Modes

| Key | Mode | Purpose |
|-----|------|---------|
| F1 | World Generator | Procedural world generation, seed tweaking |
| F2 | World Map Editor | Place cities, landmarks, draw regions |
| F3 | Chunk Editor | Detail editing at street level |
| F4 | Level Launcher | Test gameplay with player spawn |

## Controls

| Control | Action |
|---------|--------|
| Scroll wheel | Zoom in/out |
| Left-click drag | Pan the map |
| Arrow keys | Pan the map |
| Space | Cycle layer view |
| F1-F4 | Switch editor modes |

## License

MIT

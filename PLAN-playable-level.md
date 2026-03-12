# Playable Level: From World Map to Street-Level Gameplay

## Goal

When the player selects a location on the world map and enters play mode, generate a micro-detail (0.25 world-unit) tilemap around that location, spawn a player entity, and run a chunk-streaming pipeline that loads/unloads tiles as the player moves. The micro zoom tier is *only* accessible through this playable level system — never from the world map scroll.

## What Already Exists

| Component | Status | Location |
|-----------|--------|----------|
| `AppMode::LevelLauncher` (F4) | ✅ UI shell, play/stop toggle | `crates/rb_editor/src/launcher_ui.rs` |
| `LauncherState` resource | ✅ is_playing, debug toggles | `crates/rb_editor/src/launcher_ui.rs` |
| `TestPlayer` + WASD movement | ✅ Basic sprite, no collision | `crates/rb_editor/src/launcher_ui.rs` |
| `SelectedChunk` resource | ✅ Stores selected chunk coord | `crates/rb_world/src/definition.rs` |
| `Player` component | ⬜ Stub only | `crates/rb_player/src/lib.rs` |
| `RbPlayerPlugin` | ⬜ Empty plugin | `crates/rb_player/src/lib.rs` |
| `CollisionFlags` | ✅ Bitflags defined | `crates/rb_tilemap/src/lib.rs` |
| `RbTilemapPlugin` | ⬜ Empty plugin | `crates/rb_tilemap/src/lib.rs` |
| `RbEntitySpawnPlugin` | ⬜ Empty plugin | `crates/rb_entity_spawn/src/lib.rs` |
| `BiomeMap::generate_meso_full` | ✅ Full-layer meso/micro gen | `crates/rb_noise/src/biome_map.rs` |
| `ChunkCoord`, `TileCoord`, `WorldPos` | ✅ Core types | `crates/rb_core/src/coords.rs` |
| Chunk pipeline (CLAUDE.md design) | ⬜ Designed, not implemented | — |

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│  World Map (F1/F2)                                              │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │ Macro tier (pre-generated) ─► Meso tier (streamed)      │    │
│  │ STOPS HERE. No micro zoom on world map.                  │    │
│  └─────────────────────────────────────────────────────────┘    │
│                           │                                      │
│                    Select chunk + F4                              │
│                           ▼                                      │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │ Playable Level (F4 LevelLauncher)                        │    │
│  │                                                          │    │
│  │  Camera follows player (not free-scroll)                 │    │
│  │  Micro-detail tiles streamed around player position      │    │
│  │  Collision, entities, NPCs loaded per chunk              │    │
│  │  World map hidden — separate render layer                │    │
│  └─────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────┘
```

## Implementation Plan

### Phase 1: Entry/Exit — Transitioning to Playable Mode

**Goal:** When F4 + Play is pressed, hide the world map, spawn the player at the selected location, switch to a player-follow camera.

#### Step 1.1: Define PlayableLevel resource

**File:** `crates/rb_core/src/lib.rs` (or new `crates/rb_core/src/level.rs`)

```rust
/// Active playable level state. Inserted when entering play mode, removed on exit.
#[derive(Resource)]
pub struct PlayableLevel {
    /// World-space origin of the level (center of the selected chunk).
    pub origin: WorldPos,
    /// The chunk coordinate the level is centered on.
    pub chunk: ChunkCoord,
    /// Seed for this level (inherited from WorldDefinition).
    pub seed: u32,
}
```

This resource acts as a flag — its presence means we're in playable mode.

#### Step 1.2: Enter/exit play mode system

**File:** `crates/rb_editor/src/launcher_ui.rs` (extend existing)

When `LauncherState.is_playing` flips to `true`:
1. Insert `PlayableLevel` resource with origin from `SelectedChunk`
2. Hide all world map sprites (set `Visibility::Hidden` on macro/meso sprite pools)
3. Snap camera to player spawn position
4. Set camera scale to playable range (~0.02–0.05 for street-level view)
5. Lock camera to follow-player mode (disable free pan/zoom)

When `is_playing` flips to `false` or ESC:
1. Remove `PlayableLevel` resource
2. Despawn all playable-level entities (chunks, player, NPCs)
3. Restore world map sprites visibility
4. Snap camera back to the chunk location on the world map
5. Re-enable free pan/zoom

#### Step 1.3: Separate world map and level rendering

The world map and playable level should NOT render simultaneously. Use a simple approach:

- World map sprites: only visible when `PlayableLevel` resource is absent
- Level sprites: only visible when `PlayableLevel` resource is present

Can use Bevy's `run_if(resource_exists::<PlayableLevel>)` for level systems and `run_if(not(resource_exists::<PlayableLevel>))` for map systems.

---

### Phase 2: Chunk Pipeline — Stream Micro Tiles Around the Player

This is the core system from CLAUDE.md's chunk pipeline design, adapted for the actual codebase.

#### Step 2.1: Define chunk components and resources

**File:** `crates/rb_tilemap/src/lib.rs` (extend)

```rust
/// A loaded playable-level chunk.
#[derive(Component)]
pub struct LevelChunk {
    pub coord: ChunkCoord,
}

/// Tile data for a loaded chunk: floor types + collision.
/// 64×64 tiles per chunk (CHUNK_SIZE = 64, but at micro scale
/// one "chunk" might be smaller — see step 2.2 for sizing).
#[derive(Component)]
pub struct ChunkTiles {
    pub tiles: Vec<TileType>,
    pub collision: Vec<CollisionFlags>,
    pub width: usize,
    pub height: usize,
}

/// Tracks which chunks are currently loaded around the player.
#[derive(Resource, Default)]
pub struct LoadedChunks {
    pub chunks: HashMap<ChunkCoord, Entity>,
}

/// Queue for chunks being generated asynchronously.
#[derive(Resource, Default)]
pub struct ChunkGenQueue {
    pub in_flight: Vec<(ChunkCoord, Task<GeneratedChunk>)>,
}
```

#### Step 2.2: Define the playable chunk grid

The micro-level BiomeMap generates a 512×512 pixel map covering 0.25×0.25 world units. That's the highest detail, but for *playable tiles* at 1m per tile:

- A playable chunk = 64×64 tiles = 64×64 meters = covers some fraction of a world unit
- Need to decide the mapping: **1 tile = 1 world unit / 256** (so 64 tiles = 0.25 world units, matching MICRO_WORLD_SIZE)
- Or simpler: **1 playable chunk = 1 micro BiomeMap region** (0.25×0.25 world units, 512 pixels downsampled to 64×64 tiles = 8:1 ratio)

**Recommended:** One playable chunk = one micro BiomeMap call at `detail_level=3`, downsampled 8:1 from 512→64 tile resolution. This means each tile gets an 8×8 pixel area from the biome map to determine its floor type (use majority-vote or center sample).

The **load radius** should be ~5 chunks in each direction (a 10×10 grid = 100 chunks), giving ~2.5 world units of visible area. This is generous for a top-down view at street scale.

#### Step 2.3: ChunkLoadSystem — determine which chunks to load/unload

**File:** `crates/rb_tilemap/src/systems/chunk_load.rs` (new)

```rust
/// Runs every frame. Compares player position to loaded chunks.
/// Queues loads for chunks within LOAD_RADIUS, marks distant chunks for unload.
fn chunk_load_system(
    player: Query<&Transform, With<Player>>,
    level: Res<PlayableLevel>,
    loaded: Res<LoadedChunks>,
    mut gen_queue: ResMut<ChunkGenQueue>,
) {
    let player_pos = player.single().translation;
    let player_chunk = world_pos_to_chunk(player_pos);

    const LOAD_RADIUS: i32 = 5;
    const UNLOAD_RADIUS: i32 = 7; // Hysteresis: unload further than load

    // Queue loads for missing chunks within radius
    for dy in -LOAD_RADIUS..=LOAD_RADIUS {
        for dx in -LOAD_RADIUS..=LOAD_RADIUS {
            let coord = ChunkCoord::new(player_chunk.x + dx, player_chunk.y + dy);
            if !loaded.chunks.contains_key(&coord) && !gen_queue.contains(&coord) {
                // Dispatch async BiomeMap::generate_meso_full_with_backend(... detail_level=3 ...)
                gen_queue.enqueue(coord, level.seed, level.origin);
            }
        }
    }
}
```

#### Step 2.4: ChunkParameterSystem — generate chunk data (async)

**File:** `crates/rb_tilemap/src/systems/chunk_gen.rs` (new)

Each chunk generation task:
1. Calls `BiomeMap::generate_meso_full_with_backend()` with `detail_level=3`, `world_size=MICRO_WORLD_SIZE` for the chunk's world region
2. Passes the macro `BiomeMap` for river seeding (from `WorldTextures` or cached)
3. Downsamples the 512×512 biome result to 64×64 tiles
4. Maps `TileType` → floor sprite + `CollisionFlags`
5. Returns `GeneratedChunk { tiles, collision, biome_map }`

This runs on `AsyncComputeTaskPool` — same pattern as the existing macro pregen.

#### Step 2.5: TileGenerationSystem — spawn tile sprites

**File:** `crates/rb_tilemap/src/systems/tile_render.rs` (new)

When a chunk finishes generating:
1. Spawn a parent entity with `LevelChunk` component
2. For now: render as a single sprite (colored by dominant biome), same as the world map tiles
3. Later: spawn individual tile sprites or use a tilemap crate (bevy_ecs_tilemap) for efficient rendering
4. Store the `ChunkTiles` component for collision queries

**Initial approach (colored rectangles):** Don't worry about actual tilesets yet. Use the same biome→color mapping from the world map. One sprite per chunk, textured from the BiomeMap image data. This gets the pipeline working before adding tile-by-tile rendering.

#### Step 2.6: ChunkUnloadSystem — despawn distant chunks

**File:** `crates/rb_tilemap/src/systems/chunk_unload.rs` (new)

When chunks are beyond `UNLOAD_RADIUS`:
1. Despawn the chunk entity and all children
2. Remove from `LoadedChunks`
3. (Future: persist player deltas before despawning)

---

### Phase 3: Player Controller

#### Step 3.1: Proper Player component and spawn

**File:** `crates/rb_player/src/lib.rs` (replace stub)

```rust
#[derive(Component)]
pub struct Player {
    pub speed: f32,          // Movement speed in tiles/second
}

#[derive(Component)]
pub struct PlayerCamera;     // Marker for the camera that follows the player
```

Move the spawn/despawn/movement logic from `launcher_ui.rs` into `rb_player`. The launcher UI should just toggle `PlayableLevel` on/off; the player plugin handles the rest.

#### Step 3.2: Camera follow system

**File:** `crates/rb_player/src/systems/camera.rs` (new)

```rust
fn camera_follow_player(
    player: Query<&Transform, With<Player>>,
    mut camera: Query<&mut Transform, (With<PlayerCamera>, Without<Player>)>,
) {
    let Ok(player_tf) = player.get_single() else { return };
    let Ok(mut cam_tf) = camera.get_single_mut() else { return };
    // Smooth follow with lerp
    let target = player_tf.translation.truncate();
    let current = cam_tf.translation.truncate();
    let smoothed = current.lerp(target, 0.1);
    cam_tf.translation.x = smoothed.x;
    cam_tf.translation.y = smoothed.y;
}
```

#### Step 3.3: Tile-based collision

**File:** `crates/rb_player/src/systems/collision.rs` (new)

Before applying movement:
1. Compute candidate position from WASD input
2. Query `ChunkTiles` for the chunk(s) the candidate position falls in
3. Check `CollisionFlags` at the target tile
4. If `BLOCKED` or `WATER`, cancel that movement axis (slide along walls)

Simple AABB collision — the player is 1×1.5 tiles, check all tiles the player rect would overlap.

---

### Phase 4: Entity Spawning (Future — not in first pass)

This phase fills in the `rb_entity_spawn` crate. Skip for the initial implementation.

#### Step 4.1: ChunkParameters from BiomeMap

Extract per-chunk metadata from the generated BiomeMap:
- `district_type` (residential, commercial, wilderness, etc.)
- `wealth` (from resource richness)
- `density` (from continentalness + proximity to cities)
- `biome` (dominant TileType)

#### Step 4.2: Palette-driven spawning

Load `DistrictPalette` RON files from `assets/palettes/` that map:
- Noise ranges → building types
- Biome + density → clutter objects (trees, rocks, market stalls)
- District type → NPC spawn tables

#### Step 4.3: Authored site overlay

If the chunk overlaps an `AuthoredSite` (capital, town, village, landmark), merge the authored data on top of the procedural generation. Capitals override everything; villages just shift the seed.

---

## Execution Order (Which Phase First)

```
Phase 1 (Entry/Exit)         ← Do this first. Gets F4 mode working properly.
    ↓
Phase 2 (Chunk Pipeline)     ← Core streaming. Reuses existing BiomeMap code.
    ↓
Phase 3 (Player Controller)  ← Makes it playable. Collision is the key feature.
    ↓
Phase 4 (Entity Spawning)    ← Makes it interesting. Can iterate on this forever.
```

**Minimum viable playable level = Phase 1 + Phase 2 + Phase 3.1–3.2 (no collision yet).**

That gives you: enter play mode → see micro-detail tiles streaming around the player → WASD to walk around → ESC to return to world map. Collision and entity spawning are additive.

## Files to Create

| File | Purpose |
|------|---------|
| `crates/rb_core/src/level.rs` | `PlayableLevel` resource definition |
| `crates/rb_tilemap/src/systems/chunk_load.rs` | Load/unload chunk decisions |
| `crates/rb_tilemap/src/systems/chunk_gen.rs` | Async BiomeMap generation per chunk |
| `crates/rb_tilemap/src/systems/tile_render.rs` | Spawn chunk sprites from generated data |
| `crates/rb_tilemap/src/systems/chunk_unload.rs` | Despawn distant chunks |
| `crates/rb_player/src/systems/camera.rs` | Camera follow |
| `crates/rb_player/src/systems/collision.rs` | Tile collision |

## Files to Modify

| File | Changes |
|------|---------|
| `src/main.rs` | Remove micro from world map zoom (see PLAN-river-lod-fix.md Step 8). Add level systems to Ready phase with `run_if(resource_exists::<PlayableLevel>)`. |
| `crates/rb_editor/src/launcher_ui.rs` | Insert/remove `PlayableLevel` resource on play/stop. Hide world map on entry. Delegate player spawn to `rb_player`. |
| `crates/rb_tilemap/src/lib.rs` | Register chunk systems, add `LevelChunk`, `ChunkTiles`, `LoadedChunks`, `ChunkGenQueue`. |
| `crates/rb_player/src/lib.rs` | Replace stub with `Player`, `PlayerCamera`, movement system, camera follow. |
| `crates/rb_core/src/lib.rs` | Export `PlayableLevel`, `level` module. |

## Key Design Decisions

**Q: How big is a playable chunk in tiles?**
A: 64×64 tiles. One chunk = one `BiomeMap::generate_meso_full_with_backend(..., detail_level=3, world_size=MICRO_WORLD_SIZE)` call, downsampled from 512px to 64 tiles.

**Q: How does the player position map to world coordinates?**
A: Player moves in tile-space (1 unit = 1 tile = 1 meter). Tile position → world position via `tile_pos * (MICRO_WORLD_SIZE / 64.0) + chunk_origin`. This mapping is critical for sampling the correct BiomeMap region.

**Q: What about the camera?**
A: During play mode, the camera follows the player with smooth lerp. The orthographic scale should be ~0.02–0.04 (showing roughly 30–60 tiles on screen). The existing free-scroll camera is disabled during play mode.

**Q: How do rivers look at playable scale?**
A: River tiles from the micro BiomeMap appear as blue floor tiles. After the river LOD fix (PLAN-river-lod-fix.md), these will be consistent with the macro rivers. At playable scale, a major river might be 3–8 tiles wide. Streams might be 1 tile wide.

**Q: What about performance?**
A: The load radius of 5 chunks means up to ~100 chunks loaded at once. Each chunk generation takes the same time as a meso tile (~50–200ms). With 16 concurrent async tasks, the initial load takes ~1–2 seconds. Unloading keeps memory bounded. This is the same pattern the world map already uses for meso tile streaming.

## Dependency on River LOD Fix

The playable level will generate micro-detail `BiomeMap`s. These will have the same river multiplication bug unless PLAN-river-lod-fix.md is implemented first. The river fix should land before or alongside Phase 2 of this plan.

Specifically, the chunk generation in Step 2.4 passes `macro_map` to `generate_meso_full_with_backend` — the same code path that already does river seeding correctly. So the playable level inherits the fix automatically, as long as `macro_map` is available to pass through. The `PlayableLevel` resource should cache or reference the global `BiomeMap` for this purpose.

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

## Workspace Structure

```
randlebrot/
├── crates/
│   ├── rb_core/          # Shared types: ChunkCoord, TileCoord, WorldPos
│   ├── rb_noise/         # Fractal chunk hierarchy (macro/meso/micro)
│   ├── rb_world/         # WorldDefinition, plates, coastlines, climate
│   ├── rb_tilemap/       # Tile storage, collision, chunk rendering
│   ├── rb_entity_spawn/  # Building/NPC/clutter spawning
│   ├── rb_editor/        # egui editor UI and authoring tools
│   ├── rb_player/        # Player controller and camera
│   └── rb_persistence/   # Delta storage, save/load (RON format)
├── assets/
│   ├── tilesets/         # Tileset sprite sheets
│   ├── authored/         # Hand-placed data (RON files)
│   └── palettes/         # District mappings (RON files)
└── src/main.rs           # Plugin composition, AppMode state
```

## World Design

### Tidally Locked Planet

- **West**: Frozen darkness (dark side) — impassable frozen wastes
- **Center**: Habitable twilight crescent running N-S along the terminator
- **East**: Scorched sunward ocean — deadly heat, valuable resources

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

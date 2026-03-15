use bevy::image::{ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on, poll_once};
use bevy_egui::{egui, EguiContexts};
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use rb_core::{AppMode, ModeTransitionEvent, PlayableLevel, SelectedChunk, SelectedMesoTile, SelectedMicroTile, TerrainQuery, WorldPos, handle_mode_shortcuts};
use rb_editor::{CurrentLayer, CurrentLifeGenLayer, GenerateMesoRequest, GeneratorUiState, LaunchLevelRequest, LauncherPhase, LifeGenLayer, RegenerateLifeGenRequest, RegenerationRequest, StartPlayRequest};
use rb_noise::{BiomeMap, MesoTerrainView, NoiseBackend, NormalizationHints};
use rb_player::Player;
use rb_tilemap::{LevelChunk, LoadedChunks};
use rb_world::{LifeGenData, WorldDefinition};
use bevy::window::PrimaryWindow;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

const MAP_WIDTH: usize = 1024;
const MAP_HEIGHT: usize = 512;
const CHUNK_SIZE_I: usize = 64;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Randlebrot - World Editor".into(),
                resolution: (MAP_WIDTH as f32, MAP_HEIGHT as f32).into(),
                ..default()
            }),
            ..default()
        }))
        // State and events
        .init_state::<AppMode>()
        .init_state::<AppPhase>()
        .add_event::<ModeTransitionEvent>()
        .init_resource::<CurrentLayer>()
        .init_resource::<GeneratorParams>()
        .init_resource::<CursorWorldPos>()
        .init_resource::<VisibleChunkRange>()
        .init_resource::<HighlightInfo>()
        .init_resource::<LifeGenOverlayState>()
        // Plugins
        .add_plugins((
            rb_core::RbCorePlugin,
            rb_noise::RbNoisePlugin,
            rb_world::RbWorldPlugin,
            rb_tilemap::RbTilemapPlugin,
            rb_entity_spawn::RbEntitySpawnPlugin,
            rb_editor::RbEditorPlugin,
            rb_player::RbPlayerPlugin,
            rb_persistence::RbPersistencePlugin,
        ))
        // Startup - just spawn camera
        .add_systems(Startup, setup_camera)
        // Config phase - show config UI
        .add_systems(Update, config_ui.run_if(in_state(AppPhase::Config)))
        // Generating phase - generate macro + transition to ready
        .add_systems(Update,
            start_generation
                .run_if(resource_added::<GenerationStarted>)
                .run_if(in_state(AppPhase::Generating)),
        )
        // GeneratingMacro phase - pre-generate all 128 macro tiles
        .add_systems(Update, (
            dispatch_macro_pregen,
            poll_macro_pregen,
            macro_pregen_progress_ui,
        ).run_if(in_state(AppPhase::GeneratingMacro)))
        // GeneratingLifeGen phase - async civilisation generation
        .add_systems(Update, (
            poll_lifegen_task,
            lifegen_progress_ui,
        ).run_if(in_state(AppPhase::GeneratingLifeGen)))
        .init_resource::<LevelChunkQueue>()
        // Ready phase - main game systems
        .add_systems(Update, (
            handle_mode_shortcuts,
            handle_layer_change.run_if(in_state(AppMode::WorldGenerator)),
            regenerate_world.run_if(in_state(AppMode::WorldGenerator)),
            regenerate_lifegen,
            log_mode_transition,
        ).run_if(in_state(AppPhase::Ready)))
        // Click on world map to select a chunk (no mode switch)
        .add_systems(Update,
            click_to_select_chunk
                .run_if(in_state(AppPhase::Ready)
                    .and(in_state(AppMode::WorldGenerator).or(in_state(AppMode::CivGenerator)))
                    .and(not(resource_exists::<PlayableLevel>))),
        )
        // World map systems (WorldGenerator and CivGenerator, not during play)
        .add_systems(Update, (
            camera_zoom,
            camera_pan,
            calculate_visible_chunks,
            enqueue_and_dispatch_tiles,
            poll_tile_results,
            manage_tile_sprites,
            update_cursor_world_pos,
            update_chunk_highlight,
            update_chunk_selection_highlight,
            highlight_info_ui,
        ).run_if(in_state(AppPhase::Ready).and(
            in_state(AppMode::WorldGenerator).or(in_state(AppMode::CivGenerator))
        )))
        // Lifegen overlay: render civilization data on world map
        .add_systems(Update, manage_lifegen_overlay
            .run_if(in_state(AppPhase::Ready).and(in_state(AppMode::CivGenerator))))
        .add_systems(Update, hide_lifegen_overlay
            .run_if(in_state(AppPhase::Ready).and(not(in_state(AppMode::CivGenerator)))))
        .add_systems(OnEnter(AppMode::CivGenerator), show_lifegen_overlay)
        // Launcher: enter/exit
        .add_systems(OnEnter(AppMode::LevelLauncher), enter_launcher_macro_view)
        .add_systems(OnExit(AppMode::LevelLauncher), cleanup_launcher_entities)
        // Launcher: "Generate Mesomap" button
        .add_systems(Update,
            handle_generate_meso_request
                .run_if(in_state(AppPhase::Ready)
                    .and(resource_exists::<GenerateMesoRequest>)),
        )
        // Launcher: meso generation + progress UI
        .add_systems(Update, (
            dispatch_meso_pregen,
            poll_meso_pregen,
            meso_pregen_progress_ui,
        ).run_if(in_state(AppPhase::Ready)
            .and(resource_exists::<MesoPregenState>)))
        // Launcher: camera pan (all launcher phases)
        .add_systems(Update,
            camera_pan
                .run_if(in_state(AppPhase::Ready)
                    .and(in_state(AppMode::LevelLauncher))),
        )
        // Launcher: meso view interactions
        .add_systems(Update, (
            launcher_camera_zoom,
            click_to_select_meso_tile,
            update_meso_highlight,
        ).run_if(in_state(AppPhase::Ready)
            .and(in_state(AppMode::LevelLauncher))
            .and(|phase: Option<Res<LauncherPhase>>| phase.map_or(false, |p| *p == LauncherPhase::MesoView))))
        // Launcher: re-show meso grid when ESC returns from Playing
        .add_systems(Update,
            reshow_meso_on_return
                .run_if(in_state(AppPhase::Ready)
                    .and(in_state(AppMode::LevelLauncher))),
        )
        // Launcher: "Launch Level" button (starts micro generation)
        .add_systems(Update,
            handle_launch_level_request
                .run_if(in_state(AppPhase::Ready)
                    .and(resource_exists::<LaunchLevelRequest>)),
        )
        // Launcher: micro generation + progress UI
        .add_systems(Update, (
            dispatch_micro_pregen,
            poll_micro_pregen,
            micro_pregen_progress_ui,
        ).run_if(in_state(AppPhase::Ready)
            .and(resource_exists::<MicroPregenState>)))
        // Launcher: micro view interactions
        .add_systems(Update, (
            launcher_camera_zoom,
            click_to_select_micro_tile,
            update_micro_highlight,
        ).run_if(in_state(AppPhase::Ready)
            .and(in_state(AppMode::LevelLauncher))
            .and(|phase: Option<Res<LauncherPhase>>| phase.map_or(false, |p| *p == LauncherPhase::MicroView))))
        // Launcher: ESC from MicroView → MesoView
        .add_systems(Update,
            escape_micro_view
                .run_if(in_state(AppPhase::Ready)
                    .and(in_state(AppMode::LevelLauncher))),
        )
        // Launcher: "Play" button
        .add_systems(Update,
            handle_start_play_request
                .run_if(in_state(AppPhase::Ready)
                    .and(resource_exists::<StartPlayRequest>)),
        )
        // Launcher: re-show micro grid when ESC returns from Playing
        .add_systems(Update,
            reshow_micro_on_return
                .run_if(in_state(AppPhase::Ready)
                    .and(in_state(AppMode::LevelLauncher))),
        )
        // Hide world map when play mode starts
        .add_systems(Update,
            hide_world_map_on_play
                .run_if(in_state(AppPhase::Ready).and(resource_added::<PlayableLevel>)),
        )
        // Level chunk streaming systems (only during play mode)
        .add_systems(Update, (
            level_chunk_load_system,
            level_chunk_poll_system,
            level_chunk_unload_system,
        ).run_if(in_state(AppPhase::Ready).and(resource_exists::<PlayableLevel>)))
        .run();
}

// ─── Constants ───────────────────────────────────────────────────────────────

/// Size of macro chunks in pixels (for highlighting grid).
const CHUNK_SIZE: f32 = 64.0;

/// Size of tile maps in pixels (per tile).
const TILE_MAP_SIZE: usize = 512;

/// Number of pre-spawned macro pool sprites.
const MACRO_POOL_SIZE: usize = 160;

/// Max cached macro tiles — sized to hold all 128 pre-generated tiles.
const MACRO_CACHE_MAX: usize = 128;

/// Max concurrent async tile generation tasks (streaming).
const MAX_CONCURRENT_TILES: usize = 16;

/// Meso tile world size (8×8 world units).
const MESO_WORLD_SIZE: f64 = 8.0;

/// Number of meso tiles per macro chunk edge (64/8 = 8).
const MESO_GRID_SIZE: i32 = 8;

/// Display size of each meso tile in the launcher grid.
const MESO_TILE_DISPLAY_PX: f32 = 64.0;

/// Max concurrent async tile generation tasks during macro pre-generation.
const MACRO_PREGEN_CONCURRENCY: usize = 4;

/// Max tile completions to process per frame.
const POLL_BUDGET: usize = 16;


/// Micro tile covers this many world units (1.0×1.0 area at 512×512 pixels).
const MICRO_WORLD_SIZE: f64 = 1.0;

/// Number of micro tiles per meso tile edge (8.0/1.0 = 8).
const MICRO_GRID_SIZE: i32 = 8;

/// Display size of each micro tile in the launcher grid.
const MICRO_TILE_DISPLAY_PX: f32 = 16.0;

// ─── Resources ───────────────────────────────────────────────────────────────

/// Marker resource to trigger generation start.
#[derive(Resource)]
struct GenerationStarted;

/// Global heightmap normalization hints computed after macro pregen.
/// Ensures all tiles use the same heightmap range for consistent shading.
#[derive(Resource, Clone)]
struct GlobalNormHints(NormalizationHints);

/// Parameters for world generation (editable in UI).
#[derive(Resource, Debug, Clone)]
pub struct GeneratorParams {
    pub seed: u32,
    pub needs_regenerate: bool,
}

impl Default for GeneratorParams {
    fn default() -> Self {
        Self {
            seed: 42,
            needs_regenerate: false,
        }
    }
}

/// Stores the macro BiomeMap used as parent data for meso/micro generation.
#[derive(Resource)]
struct MacroBiomeData {
    biome_map: Arc<BiomeMap>,
}

/// Global river network computed once from the macro BiomeMap.
/// All tile generations query this for consistent rivers across zoom levels.
#[derive(Resource)]
struct GlobalRiverNetwork {
    network: Arc<rb_noise::RiverNetwork>,
}

/// Marker component for the chunk highlight overlay (follows cursor).
#[derive(Component)]
struct ChunkHighlight;

/// Marker component for the persistent selection overlay (shows selected chunk).
#[derive(Component)]
struct ChunkSelectionHighlight;

/// Marker for the lifegen overlay sprite.
#[derive(Component)]
struct LifeGenOverlay;

/// Stored MesoTerrainView for LifeGen regeneration.
/// Holds Arc clones of the 128 macro BiomeMap tiles at full resolution.
#[derive(Resource)]
struct StoredTerrainView(MesoTerrainView);

/// Tracks async LifeGen generation task and progress.
#[derive(Resource)]
struct LifeGenTask {
    task: Task<LifeGenData>,
    progress: rb_world::lifegen::ProgressHandle,
}

/// Tracks which layer was last rendered to detect changes.
#[derive(Resource)]
struct LifeGenOverlayState {
    current_layer: String,
    data_generation: u64,
}

impl Default for LifeGenOverlayState {
    fn default() -> Self {
        Self {
            current_layer: String::new(),
            data_generation: 0,
        }
    }
}

/// Info about the currently highlighted tile, displayed in the UI.
#[derive(Resource, Default)]
struct HighlightInfo {
    /// Whether the cursor is over a valid tile.
    active: bool,
    /// Tile grid coordinates.
    tile_coord: (i32, i32),
    /// World-space position (top-left of tile).
    world_pos: (f32, f32),
    /// Current detail tier name.
    tier: &'static str,
    /// Which domain is active ("Terrain" / "Civilization" / "Scene").
    domain: &'static str,
}

/// Resource tracking cursor position in world space.
#[derive(Resource, Default)]
struct CursorWorldPos(Vec2);


/// Camera viewport in chunk coordinates.
#[derive(Resource, Default)]
struct VisibleChunkRange {
    min_x: i32,
    max_x: i32,
    min_y: i32,
    max_y: i32,
}

/// Marker component for pool slot sprites.
#[derive(Component)]
struct PoolSlot;

/// Pre-spawned sprite pool — no spawn/despawn during gameplay.
#[derive(Resource)]
struct SpritePool {
    macro_sprites: Vec<Entity>,
    macro_assigned: Vec<Option<(i32, i32)>>,
}

/// A cached tile with BiomeMap data and texture.
struct CachedTile {
    biome_map: Arc<BiomeMap>,
    texture: Handle<Image>,
    last_used_frame: u64,
}

/// Tile cache for macro tiles.
#[derive(Resource)]
struct TileCache {
    macro_tiles: HashMap<(i32, i32), CachedTile>,
    macro_max: usize,
    frame: u64,
}

impl Default for TileCache {
    fn default() -> Self {
        Self {
            macro_tiles: HashMap::new(),
            macro_max: MACRO_CACHE_MAX,
            frame: 0,
        }
    }
}

impl TileCache {
    fn insert_macro(&mut self, coord: (i32, i32), tile: CachedTile) {
        if self.macro_tiles.len() >= self.macro_max {
            if let Some((&evict_coord, _)) = self.macro_tiles.iter()
                .min_by_key(|(_, t)| t.last_used_frame)
            {
                self.macro_tiles.remove(&evict_coord);
            }
        }
        self.macro_tiles.insert(coord, tile);
    }

}

/// An in-flight async tile generation task.
struct InFlightTile {
    coord: (i32, i32),
    task: Task<((i32, i32), Arc<BiomeMap>)>,
}

/// Queue of in-flight tile requests.
#[derive(Resource, Default)]
struct TileRequestQueue {
    in_flight: Vec<InFlightTile>,
}

// ─── Meso Launcher Types ─────────────────────────────────────────────────────

/// Marker for the enlarged macro chunk sprite in the launcher.
#[derive(Component)]
struct LauncherMacroSprite;

/// Marker for meso tile sprites in the launcher grid.
#[derive(Component)]
struct LauncherMesoSprite;

/// Marker for the meso tile highlight overlay.
#[derive(Component)]
struct MesoHighlight;

/// Cache of generated meso tiles for the launcher.
#[derive(Resource, Default)]
struct MesoTileCache {
    tiles: HashMap<(i32, i32), MesoCachedTile>,
    sprite_entities: Vec<Entity>,
}

struct MesoCachedTile {
    _biome_map: Arc<BiomeMap>,
    texture: Handle<Image>,
}

/// State for async meso tile pre-generation.
#[derive(Resource)]
struct MesoPregenState {
    total: usize,
    completed: usize,
    remaining: Vec<(i32, i32)>,
    in_flight: Vec<MesoInFlightTile>,
}

struct MesoInFlightTile {
    task: Task<((i32, i32), Arc<BiomeMap>)>,
}

// ─── Micro Launcher Types ─────────────────────────────────────────────────────

/// Marker for micro tile sprites in the launcher grid.
#[derive(Component)]
struct LauncherMicroSprite;

/// Marker for the micro tile highlight overlay.
#[derive(Component)]
struct MicroHighlight;

/// Cache of generated micro tiles for the launcher.
#[derive(Resource, Default)]
struct MicroTileCache {
    tiles: HashMap<(i32, i32), MicroCachedTile>,
    sprite_entities: Vec<Entity>,
}

struct MicroCachedTile {
    _biome_map: Arc<BiomeMap>,
    texture: Handle<Image>,
}

/// State for async micro tile pre-generation.
#[derive(Resource)]
struct MicroPregenState {
    total: usize,
    completed: usize,
    remaining: Vec<(i32, i32)>,
    in_flight: Vec<MicroInFlightTile>,
}

struct MicroInFlightTile {
    task: Task<((i32, i32), Arc<BiomeMap>)>,
}

/// Application phase - config, generating, pre-generating macro tiles, or ready.
#[derive(States, Default, Clone, Eq, PartialEq, Hash, Debug)]
enum AppPhase {
    #[default]
    Config,
    Generating,          // Generate macro biome data
    GeneratingMacro,     // Pre-generate all 128 macro tiles
    GeneratingLifeGen,   // Run civilisation generation pipeline
    Ready,
}

/// Post-generation processing phase (one step per frame for UI updates).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MacroPostPhase {
    Generating,
    RefreshingRivers,
    Normalizing,
    SavingDebugLayers,
    BuildingTerrain,
}

/// Tracks progress of macro tile pre-generation.
#[derive(Resource)]
struct MacroPregenState {
    total: usize,
    completed: usize,
    remaining: Vec<(i32, i32)>,
    in_flight: Vec<InFlightTile>,
    post_phase: MacroPostPhase,
}

// ─── Systems ─────────────────────────────────────────────────────────────────

fn setup_camera(mut commands: Commands) {
    commands.spawn(Camera2d);
}

/// Config UI - seed input and Generate button.
fn config_ui(
    mut contexts: EguiContexts,
    mut params: ResMut<GeneratorParams>,
    mut next_phase: ResMut<NextState<AppPhase>>,
    mut commands: Commands,
) {
    let ctx = contexts.ctx_mut();

    egui::CentralPanel::default()
        .frame(egui::Frame::default().fill(egui::Color32::from_rgb(30, 30, 30)))
        .show(ctx, |_| {});

    egui::Window::new("World Generator")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .fixed_size([300.0, 150.0])
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.heading("Randlebrot");
                ui.add_space(20.0);

                ui.horizontal(|ui| {
                    ui.label("Seed:");
                    ui.add(egui::DragValue::new(&mut params.seed));
                });

                ui.add_space(20.0);

                if ui.button("Generate World").clicked() {
                    commands.insert_resource(GenerationStarted);
                    next_phase.set(AppPhase::Generating);
                }
            });
        });
}

/// Generate macro map synchronously, create sprites + pool, transition to Ready.
fn start_generation(
    mut commands: Commands,
    mut next_phase: ResMut<NextState<AppPhase>>,
    world_def: Res<WorldDefinition>,
    ui_state: Res<GeneratorUiState>,
) {
    commands.remove_resource::<GenerationStarted>();

    let seed = world_def.seed;
    let width = world_def.width;
    let height = world_def.height;
    let backend = ui_state.backend();

    // Generate macro map synchronously (fast)
    let backend_name = if backend == NoiseBackend::Gpu { "GPU" } else { "CPU" };
    println!("Generating macro map {}x{} ({})...", width, height, backend_name);
    let biome_map = BiomeMap::generate_with_backend(seed, width, height, backend);
    println!("  Macro map generated successfully");

    // Extract the global river network before wrapping in Arc
    if let Some(ref network) = biome_map.river_network {
        commands.insert_resource(GlobalRiverNetwork {
            network: network.clone(),
        });
        println!("  Global river network stored ({} segments)", network.segment_count());
    }

    commands.insert_resource(MacroBiomeData {
        biome_map: Arc::new(biome_map),
    });

    // Spawn chunk highlight (follows cursor)
    commands.spawn((
        Sprite {
            color: Color::srgba(1.0, 1.0, 0.8, 0.3),
            custom_size: Some(Vec2::splat(CHUNK_SIZE)),
            ..default()
        },
        Transform::from_xyz(-10000.0, -10000.0, 0.5),
        ChunkHighlight,
    ));

    // Spawn persistent selection highlight (shows selected chunk)
    commands.spawn((
        Sprite {
            color: Color::srgba(0.2, 0.8, 1.0, 0.45),
            custom_size: Some(Vec2::splat(CHUNK_SIZE)),
            ..default()
        },
        Transform::from_xyz(-10000.0, -10000.0, 0.4),
        Visibility::Hidden,
        ChunkSelectionHighlight,
    ));

    // Spawn sprite pool (macro + meso)
    let mut macro_sprites = Vec::with_capacity(MACRO_POOL_SIZE);
    let mut macro_assigned = Vec::with_capacity(MACRO_POOL_SIZE);
    for _i in 0..MACRO_POOL_SIZE {
        let entity = commands.spawn((
            Sprite { ..default() },
            Transform::from_xyz(-10000.0, -10000.0, 0.1),
            Visibility::Hidden,
            PoolSlot,
        )).id();
        macro_sprites.push(entity);
        macro_assigned.push(None);
    }

    commands.insert_resource(SpritePool {
        macro_sprites,
        macro_assigned,
    });
    commands.insert_resource(TileCache::default());
    commands.insert_resource(TileRequestQueue::default());

    // Queue all 128 macro tiles for pre-generation
    let chunks_x = (world_def.width as f32 / CHUNK_SIZE).ceil() as i32;
    let chunks_y = (world_def.height as f32 / CHUNK_SIZE).ceil() as i32;
    let mut remaining: Vec<(i32, i32)> = Vec::new();
    for cy in 0..chunks_y {
        for cx in 0..chunks_x {
            remaining.push((cx, cy));
        }
    }
    let total = remaining.len();
    commands.insert_resource(MacroPregenState {
        total,
        completed: 0,
        remaining,
        in_flight: Vec::new(),
        post_phase: MacroPostPhase::Generating,
    });

    next_phase.set(AppPhase::GeneratingMacro);
    println!("Macro map ready. Pre-generating {} macro tiles...", total);
}

// ─── Macro Pre-generation ────────────────────────────────────────────────────

/// Dispatch async tasks for macro tile pre-generation (higher concurrency than streaming).
fn dispatch_macro_pregen(
    mut pregen: ResMut<MacroPregenState>,
    world_textures: Option<Res<MacroBiomeData>>,
    world_def: Res<WorldDefinition>,
    ui_state: Res<GeneratorUiState>,
    global_rivers: Option<Res<GlobalRiverNetwork>>,
) {
    let Some(world_textures) = world_textures else { return };
    let seed = world_def.seed;
    let height = world_def.height as f64;
    let backend = ui_state.backend();
    let river_net = global_rivers.map(|r| r.network.clone());

    while pregen.in_flight.len() < MACRO_PREGEN_CONCURRENCY {
        let Some(coord) = pregen.remaining.pop() else { break };
        let macro_map = world_textures.biome_map.clone();
        let river_net_clone = river_net.clone();
        let world_x = coord.0 as f64 * CHUNK_SIZE as f64;
        let world_y = coord.1 as f64 * CHUNK_SIZE as f64;

        let task = AsyncComputeTaskPool::get().spawn(async move {
            let river_ref = river_net_clone.as_ref();
            let biome_map = BiomeMap::generate_meso_full_with_backend(
                seed,
                world_x,
                world_y,
                CHUNK_SIZE as f64,
                TILE_MAP_SIZE,
                height,
                1, // detail_level = macro (octave_offset)
                None,
                backend,
                Some(&macro_map),
                river_ref,
            );
            (coord, Arc::new(biome_map))
        });

        pregen.in_flight.push(InFlightTile { coord, task });
    }
}

/// Poll all completions during pre-generation (no per-frame budget on loading screen).
fn poll_macro_pregen(
    mut pregen: ResMut<MacroPregenState>,
    mut tile_cache: ResMut<TileCache>,
    mut images: ResMut<Assets<Image>>,
    current_layer: Res<CurrentLayer>,
    mut next_phase: ResMut<NextState<AppPhase>>,
    mut commands: Commands,
    global_rivers: Option<Res<GlobalRiverNetwork>>,
    world_def: Res<WorldDefinition>,
) {
    let frame = tile_cache.frame;

    let mut i = 0;
    while i < pregen.in_flight.len() {
        if let Some(result) = block_on(poll_once(&mut pregen.in_flight[i].task)) {
            pregen.in_flight.swap_remove(i);
            let (coord, biome_map) = result;

            let image_data = biome_map.to_layer_image(current_layer.0);
            let image = create_image(TILE_MAP_SIZE, TILE_MAP_SIZE, image_data);
            let texture = images.add(image);

            tile_cache.insert_macro(coord, CachedTile {
                biome_map,
                texture,
                last_used_frame: frame,
            });

            pregen.completed += 1;
        } else {
            i += 1;
        }
    }

    if pregen.completed >= pregen.total && pregen.post_phase == MacroPostPhase::Generating {
        println!("All {} macro tiles pre-generated.", pregen.total);
        pregen.post_phase = MacroPostPhase::RefreshingRivers;
        return; // yield frame for UI update
    }

    // Post-generation phases: one step per frame so the progress UI updates
    match pregen.post_phase {
        MacroPostPhase::Generating => {} // still generating tiles
        MacroPostPhase::RefreshingRivers => {
            if let Some(ref global_rivers) = global_rivers {
                use rb_noise::{rasterize_from_network, LOD_THRESHOLD_MACRO, derived, SEA_LEVEL};
                use rb_core::TileType;

                for cy in 0..8i32 {
                    for cx in 0..16i32 {
                        let coord = (cx, cy);
                        let Some(cached) = tile_cache.macro_tiles.get_mut(&coord) else { continue };
                        let Some(biome_map) = Arc::get_mut(&mut cached.biome_map) else { continue };
                        let world_x = cx as f64 * CHUNK_SIZE as f64;
                        let world_y = cy as f64 * CHUNK_SIZE as f64;
                        let new_rivers = rasterize_from_network(
                            &global_rivers.network, world_x, world_y,
                            CHUNK_SIZE as f64, TILE_MAP_SIZE, LOD_THRESHOLD_MACRO,
                        );
                        for idx in 0..biome_map.rivers.len() {
                            biome_map.rivers[idx] = new_rivers[idx];
                            biome_map.water_table[idx] = derived::derive_water_table(
                                new_rivers[idx], biome_map.humidity[idx], biome_map.heightmap[idx],
                                biome_map.precipitation_type[idx], biome_map.continentalness[idx],
                            );
                            if new_rivers[idx] > 0.0
                                && biome_map.continentalness[idx] >= SEA_LEVEL
                                && biome_map.temperature[idx] > -10.0
                                && biome_map.temperature[idx] < 70.0
                            {
                                biome_map.biomes[idx] = TileType::River;
                            }
                        }
                        let image_data = biome_map.to_layer_image(current_layer.0);
                        cached.texture = images.add(create_image(TILE_MAP_SIZE, TILE_MAP_SIZE, image_data));
                    }
                }
            }
            pregen.post_phase = MacroPostPhase::Normalizing;
        }
        MacroPostPhase::Normalizing => {
            let mut hmin = f64::MAX;
            let mut hmax = f64::MIN;
            for cached in tile_cache.macro_tiles.values() {
                for &v in &cached.biome_map.heightmap {
                    if v < hmin { hmin = v; }
                    if v > hmax { hmax = v; }
                }
            }
            let norm_hints = NormalizationHints {
                heightmap_min: if hmin < hmax { hmin } else { 0.0 },
                heightmap_max: if hmin < hmax { hmax } else { 1.0 },
            };
            commands.insert_resource(GlobalNormHints(norm_hints.clone()));

            for cached in tile_cache.macro_tiles.values_mut() {
                let image_data = cached.biome_map.to_layer_image_with_hints(current_layer.0, Some(&norm_hints));
                cached.texture = images.add(create_image(TILE_MAP_SIZE, TILE_MAP_SIZE, image_data));
            }
            pregen.post_phase = MacroPostPhase::SavingDebugLayers;
        }
        MacroPostPhase::SavingDebugLayers => {
            save_stitched_debug_layers(&tile_cache);
            pregen.post_phase = MacroPostPhase::BuildingTerrain;
        }
        MacroPostPhase::BuildingTerrain => {
            let tile_biome_maps: HashMap<(i32, i32), Arc<BiomeMap>> = tile_cache.macro_tiles.iter()
                .map(|(&coord, cached)| (coord, cached.biome_map.clone()))
                .collect();
            let chunks_x = (MAP_WIDTH as f32 / CHUNK_SIZE).ceil() as usize;
            let chunks_y = (MAP_HEIGHT as f32 / CHUNK_SIZE).ceil() as usize;

            commands.insert_resource(StoredTerrainView(
                MesoTerrainView::from_tile_map(&tile_biome_maps, chunks_x, chunks_y, TILE_MAP_SIZE),
            ));

            // Spawn async LifeGen task
            let terrain_view = Arc::new(MesoTerrainView::from_tile_map(
                &tile_biome_maps, chunks_x, chunks_y, TILE_MAP_SIZE,
            ));
            let civ_seed = world_def.civ_seed;
            let progress = rb_world::lifegen::new_progress();
            let progress_clone = progress.clone();
            let task = AsyncComputeTaskPool::get().spawn(async move {
                let data = rb_world::lifegen::generate_with_progress(
                    terrain_view.as_ref(), civ_seed, Some(progress_clone),
                );
                data.save_debug_layers(std::path::Path::new("debug_layers"));
                data
            });
            commands.insert_resource(LifeGenTask { task, progress });

            commands.remove_resource::<MacroPregenState>();
            next_phase.set(AppPhase::GeneratingLifeGen);
        }
    }
}

/// Stitch all pre-generated macro tiles into full-world debug PNGs.
/// This saves exactly what the app displays on the world map.
fn save_stitched_debug_layers(tile_cache: &TileCache) {
    use image::{ImageBuffer, Rgba, RgbaImage};
    use rb_noise::NoiseLayer;

    let chunks_x = (MAP_WIDTH as f32 / CHUNK_SIZE).ceil() as usize;  // 16
    let chunks_y = (MAP_HEIGHT as f32 / CHUNK_SIZE).ceil() as usize; // 8
    let full_w = (chunks_x * TILE_MAP_SIZE) as u32; // 8192
    let full_h = (chunks_y * TILE_MAP_SIZE) as u32; // 4096
    let tile_px = TILE_MAP_SIZE as u32;

    let out_dir = std::path::Path::new("debug_layers");
    let base_dir = out_dir.join("base");
    let derived_dir = out_dir.join("derived");
    for dir in [out_dir, &base_dir, &derived_dir] {
        let _ = std::fs::create_dir_all(dir);
    }

    println!("Saving debug layers ({full_w}x{full_h})...");

    for layer in NoiseLayer::all() {
        let name = layer.name();
        let mut full_img: RgbaImage = ImageBuffer::new(full_w, full_h);

        for cy in 0..chunks_y {
            for cx in 0..chunks_x {
                let coord = (cx as i32, cy as i32);
                let Some(cached) = tile_cache.macro_tiles.get(&coord) else { continue };
                let rgba_data = cached.biome_map.to_layer_image(*layer);
                let Some(tile_img) = ImageBuffer::<Rgba<u8>, _>::from_raw(tile_px, tile_px, rgba_data) else { continue };

                let ox = (cx as u32) * tile_px;
                let oy = (cy as u32) * tile_px;
                for py in 0..tile_px {
                    for px in 0..tile_px {
                        full_img.put_pixel(ox + px, oy + py, *tile_img.get_pixel(px, py));
                    }
                }
            }
        }

        let path = match layer {
            NoiseLayer::Biome => out_dir.join("biome.png"),
            NoiseLayer::Continentalness
            | NoiseLayer::Tectonic
            | NoiseLayer::Humidity
            | NoiseLayer::RockHardness
            | NoiseLayer::LightLevel => base_dir.join(format!("{name}.png")),
            _ => derived_dir.join(format!("{name}.png")),
        };

        // Downscale 2x to keep PNGs under 30MB (8192×4096 → 4096×2048)
        let half_w = full_w / 2;
        let half_h = full_h / 2;
        let mut small_img: RgbaImage = ImageBuffer::new(half_w, half_h);
        for sy in 0..half_h {
            for sx in 0..half_w {
                let x0 = sx * 2;
                let y0 = sy * 2;
                let [r0, g0, b0, a0] = full_img.get_pixel(x0, y0).0;
                let [r1, g1, b1, a1] = full_img.get_pixel(x0 + 1, y0).0;
                let [r2, g2, b2, a2] = full_img.get_pixel(x0, y0 + 1).0;
                let [r3, g3, b3, a3] = full_img.get_pixel(x0 + 1, y0 + 1).0;
                let avg = Rgba([
                    ((r0 as u16 + r1 as u16 + r2 as u16 + r3 as u16) / 4) as u8,
                    ((g0 as u16 + g1 as u16 + g2 as u16 + g3 as u16) / 4) as u8,
                    ((b0 as u16 + b1 as u16 + b2 as u16 + b3 as u16) / 4) as u8,
                    ((a0 as u16 + a1 as u16 + a2 as u16 + a3 as u16) / 4) as u8,
                ]);
                small_img.put_pixel(sx, sy, avg);
            }
        }

        if let Err(e) = small_img.save(&path) {
            eprintln!("Failed to save {}: {e}", path.display());
        } else {
            println!("  Saved {} ({}x{})", path.display(), half_w, half_h);
        }
    }
}

/// Show progress during macro tile pre-generation with phase list.
fn macro_pregen_progress_ui(
    mut contexts: EguiContexts,
    pregen: Res<MacroPregenState>,
) {
    let ctx = contexts.ctx_mut();

    egui::CentralPanel::default()
        .frame(egui::Frame::default().fill(egui::Color32::from_rgb(30, 30, 30)))
        .show(ctx, |_| {});

    // Overall progress: tiles are 0-80%, post-processing phases are 80-100%
    let tile_progress = pregen.completed as f32 / pregen.total.max(1) as f32;
    let post_bonus = match pregen.post_phase {
        MacroPostPhase::Generating => 0.0,
        MacroPostPhase::RefreshingRivers => 0.05,
        MacroPostPhase::Normalizing => 0.10,
        MacroPostPhase::SavingDebugLayers => 0.15,
        MacroPostPhase::BuildingTerrain => 0.19,
    };
    let overall = (tile_progress * 0.80 + post_bonus).min(1.0);

    // Current post-phase index (0 = generating, 1-4 = post phases)
    let post_idx: usize = match pregen.post_phase {
        MacroPostPhase::Generating => 0,
        MacroPostPhase::RefreshingRivers => 1,
        MacroPostPhase::Normalizing => 2,
        MacroPostPhase::SavingDebugLayers => 3,
        MacroPostPhase::BuildingTerrain => 4,
    };

    egui::Window::new("Generating World")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .fixed_size([420.0, 200.0])
        .show(ctx, |ui| {
            ui.vertical(|ui| {
                ui.add_space(5.0);
                ui.add(egui::ProgressBar::new(overall).show_percentage());
                ui.add_space(10.0);

                let phases: &[(usize, &str, &str)] = &[
                    (0, "Macro Tiles", "128 terrain tiles at 512x512"),
                    (1, "River Refresh", "Syncing rivers from global network"),
                    (2, "Normalizing", "Global heightmap range + re-render"),
                    (3, "Saving Debug Layers", "Stitching terrain PNGs"),
                    (4, "Building Terrain View", "8192x4096 meso query surface"),
                ];

                for &(idx, name, desc) in phases {
                    let (icon, color) = if idx < post_idx {
                        ("\u{2714}", egui::Color32::from_rgb(100, 200, 100))
                    } else if idx == post_idx {
                        ("\u{25B6}", egui::Color32::from_rgb(255, 220, 80))
                    } else {
                        ("\u{25CB}", egui::Color32::from_rgb(120, 120, 120))
                    };

                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(icon).color(color).size(13.0));
                        ui.label(egui::RichText::new(name).color(color).strong().size(13.0));
                        if idx == 0 && post_idx == 0 {
                            ui.label(
                                egui::RichText::new(format!(
                                    "— {}/{}",
                                    pregen.completed, pregen.total
                                ))
                                .color(egui::Color32::from_rgb(180, 180, 180))
                                .size(12.0),
                            );
                        } else {
                            ui.label(
                                egui::RichText::new(desc)
                                    .color(egui::Color32::from_rgb(90, 90, 90))
                                    .size(11.0),
                            );
                        }
                    });
                }
            });
        });
}

/// Poll async LifeGen task. When complete, insert LifeGenData and transition to Ready.
fn poll_lifegen_task(
    mut commands: Commands,
    mut task_res: ResMut<LifeGenTask>,
    mut next_phase: ResMut<NextState<AppPhase>>,
) {
    if let Some(lifegen) = block_on(poll_once(&mut task_res.task)) {
        println!(
            "LifeGen complete: {} provinces, {} factions, {} settlements, {} roads",
            lifegen.provinces.len(),
            lifegen.factions.len(),
            lifegen.settlement_seeds.len(),
            lifegen.road_segments.len(),
        );
        commands.insert_resource(lifegen);
        commands.remove_resource::<LifeGenTask>();
        next_phase.set(AppPhase::Ready);
    }
}

/// Show progress during LifeGen generation.
fn lifegen_progress_ui(
    mut contexts: EguiContexts,
    task: Res<LifeGenTask>,
) {
    let ctx = contexts.ctx_mut();

    egui::CentralPanel::default()
        .frame(egui::Frame::default().fill(egui::Color32::from_rgb(30, 30, 30)))
        .show(ctx, |_| {});

    let (phase, label, detail, progress) = {
        let p = task.progress.lock().unwrap();
        (p.phase, p.phase_label.clone(), p.detail.clone(), p.progress)
    };

    egui::Window::new("Generating Civilisation")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .fixed_size([420.0, 180.0])
        .show(ctx, |ui| {
            ui.vertical(|ui| {
                ui.add_space(5.0);
                ui.add(egui::ProgressBar::new(progress).show_percentage());
                ui.add_space(10.0);

                let phase_names = [
                    (1, "Analysis Grids", "Habitability, navigation cost, resources"),
                    (2, "Provinces", "Poisson seeding, Voronoi tessellation, river borders"),
                    (3, "Factions", "Capital placement, territory expansion"),
                    (4, "Settlements", "Site selection within provinces"),
                    (5, "Roads", "MST + A* pathfinding"),
                    (6, "Trade", "Directed trade flow network"),
                ];

                for &(num, name, desc) in &phase_names {
                    let (icon, text_color) = if num < phase {
                        ("\u{2714}", egui::Color32::from_rgb(100, 200, 100)) // checkmark, green
                    } else if num == phase {
                        ("\u{25B6}", egui::Color32::from_rgb(255, 220, 80)) // play arrow, yellow
                    } else {
                        ("\u{25CB}", egui::Color32::from_rgb(120, 120, 120)) // circle, grey
                    };

                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(icon).color(text_color).size(13.0));
                        ui.label(
                            egui::RichText::new(format!("{name}"))
                                .color(text_color)
                                .strong()
                                .size(13.0),
                        );
                        if num == phase && !detail.is_empty() {
                            ui.label(
                                egui::RichText::new(format!("— {detail}"))
                                    .color(egui::Color32::from_rgb(180, 180, 180))
                                    .size(12.0),
                            );
                        } else if num < phase {
                            ui.label(
                                egui::RichText::new(desc)
                                    .color(egui::Color32::from_rgb(90, 90, 90))
                                    .size(11.0),
                            );
                        } else {
                            ui.label(
                                egui::RichText::new(desc)
                                    .color(egui::Color32::from_rgb(90, 90, 90))
                                    .size(11.0),
                            );
                        }
                    });
                }
            });
        });
}

/// Show progress bar during meso tile pre-generation.
fn meso_pregen_progress_ui(
    mut contexts: EguiContexts,
    pregen: Res<MesoPregenState>,
) {
    let ctx = contexts.ctx_mut();
    egui::Window::new("Generating Meso Tiles")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .fixed_size([300.0, 80.0])
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                let progress = pregen.completed as f32 / pregen.total as f32;
                ui.label(format!("Generating meso tiles: {}/{}", pregen.completed, pregen.total));
                ui.add_space(10.0);
                ui.add(egui::ProgressBar::new(progress).show_percentage());
            });
        });
}

fn create_image(width: usize, height: usize, data: Vec<u8>) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: width as u32,
            height: height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        default(),
    );

    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        mag_filter: ImageFilterMode::Nearest,
        min_filter: ImageFilterMode::Nearest,
        ..default()
    });

    image
}


// ─── Tile Streaming ──────────────────────────────────────────────────────────

/// Enqueue tile generation tasks for visible chunks not yet cached.
fn enqueue_and_dispatch_tiles(
    visible_range: Res<VisibleChunkRange>,
    tile_cache: Res<TileCache>,
    mut request_queue: ResMut<TileRequestQueue>,
    world_def: Res<WorldDefinition>,
    world_textures: Option<Res<MacroBiomeData>>,
    ui_state: Res<GeneratorUiState>,
    camera_query: Query<&Transform, With<Camera2d>>,
    global_rivers: Option<Res<GlobalRiverNetwork>>,
) {
    let Some(world_textures) = world_textures else { return };
    let Ok(camera_transform) = camera_query.get_single() else { return };
    let camera_pos = camera_transform.translation;

    let seed = world_def.seed;
    let height = world_def.height as f64;
    let backend = ui_state.backend();
    let river_net = global_rivers.map(|r| r.network.clone());
    let half_map_width = world_def.width as f32 / 2.0;
    let half_map_height = world_def.height as f32 / 2.0;

    let in_flight_coords: HashSet<(i32, i32)> = request_queue.in_flight.iter()
        .map(|t| t.coord)
        .collect();

    // Collect needed macro tiles only — meso is generated on demand in LevelLauncher
    let mut needed: Vec<((i32, i32), f32)> = Vec::new();
    for cy in visible_range.min_y..=visible_range.max_y {
        for cx in visible_range.min_x..=visible_range.max_x {
            let coord = (cx, cy);
            if tile_cache.macro_tiles.contains_key(&coord) || in_flight_coords.contains(&coord) {
                continue;
            }
            let sprite_x = cx as f32 * CHUNK_SIZE + CHUNK_SIZE / 2.0 - half_map_width;
            let sprite_y = half_map_height - cy as f32 * CHUNK_SIZE - CHUNK_SIZE / 2.0;
            let dist = (camera_pos.x - sprite_x).powi(2) + (camera_pos.y - sprite_y).powi(2);
            needed.push((coord, dist));
        }
    }

    needed.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    let macro_map = world_textures.biome_map.clone();
    for (coord, _dist) in needed {
        if request_queue.in_flight.len() >= MAX_CONCURRENT_TILES {
            break;
        }

        let macro_map_clone = macro_map.clone();
        let river_net_clone = river_net.clone();
        let world_x = coord.0 as f64 * CHUNK_SIZE as f64;
        let world_y = coord.1 as f64 * CHUNK_SIZE as f64;

        let task = AsyncComputeTaskPool::get().spawn(async move {
            let river_ref = river_net_clone.as_ref();
            let biome_map = BiomeMap::generate_meso_full_with_backend(
                seed,
                world_x,
                world_y,
                CHUNK_SIZE as f64,
                TILE_MAP_SIZE,
                height,
                1, // macro detail level
                None,
                backend,
                Some(&macro_map_clone),
                river_ref,
            );
            (coord, Arc::new(biome_map))
        });

        request_queue.in_flight.push(InFlightTile { coord, task });
    }
}

/// Poll in-flight tile tasks, up to POLL_BUDGET completions per frame.
fn poll_tile_results(
    mut request_queue: ResMut<TileRequestQueue>,
    mut tile_cache: ResMut<TileCache>,
    mut images: ResMut<Assets<Image>>,
    current_layer: Res<CurrentLayer>,
    norm_hints: Option<Res<GlobalNormHints>>,
) {
    tile_cache.frame += 1;
    let frame = tile_cache.frame;
    let mut completed = 0;

    let mut i = 0;
    while i < request_queue.in_flight.len() && completed < POLL_BUDGET {
        if let Some(result) = block_on(poll_once(&mut request_queue.in_flight[i].task)) {
            request_queue.in_flight.swap_remove(i);
            let (coord, biome_map) = result;

            let hints_ref = norm_hints.as_ref().map(|h| &h.0);
            let image_data = biome_map.to_layer_image_with_hints(current_layer.0, hints_ref);
            let image = create_image(TILE_MAP_SIZE, TILE_MAP_SIZE, image_data);
            let texture = images.add(image);

            let cached = CachedTile {
                biome_map,
                texture,
                last_used_frame: frame,
            };

            tile_cache.insert_macro(coord, cached);

            completed += 1;
            // Don't increment i — swap_remove shifted an element into position i
        } else {
            i += 1;
        }
    }
}

// ─── Sprite Pool Management ─────────────────────────────────────────────────

/// Assign/unassign pool sprites based on visible tiles and cache contents.
fn manage_tile_sprites(
    visible_range: Res<VisibleChunkRange>,
    mut tile_cache: ResMut<TileCache>,
    mut pool: ResMut<SpritePool>,
    world_def: Res<WorldDefinition>,
    mut sprite_query: Query<(&mut Transform, &mut Sprite, &mut Visibility)>,
) {
    let half_map_width = world_def.width as f32 / 2.0;
    let half_map_height = world_def.height as f32 / 2.0;
    let frame = tile_cache.frame;

    let mut needed_macro: HashSet<(i32, i32)> = HashSet::new();
    for cy in visible_range.min_y..=visible_range.max_y {
        for cx in visible_range.min_x..=visible_range.max_x {
            needed_macro.insert((cx, cy));
        }
    }

    // Free slots assigned to tiles no longer visible
    for i in 0..pool.macro_assigned.len() {
        if let Some(coord) = pool.macro_assigned[i] {
            if !needed_macro.contains(&coord) || !tile_cache.macro_tiles.contains_key(&coord) {
                pool.macro_assigned[i] = None;
                let entity = pool.macro_sprites[i];
                if let Ok((mut transform, _, mut vis)) = sprite_query.get_mut(entity) {
                    *vis = Visibility::Hidden;
                    transform.translation.x = -10000.0;
                    transform.translation.y = -10000.0;
                }
            }
        }
    }

    let assigned_macro: HashSet<(i32, i32)> = pool.macro_assigned.iter()
        .filter_map(|a| *a)
        .collect();

    for coord in &needed_macro {
        if assigned_macro.contains(coord) { continue; }
        let (tex_handle, custom_size) = {
            let Some(cached) = tile_cache.macro_tiles.get(coord) else { continue };
            (cached.texture.clone(), Vec2::splat(CHUNK_SIZE))
        };

        let Some(slot_idx) = pool.macro_assigned.iter().position(|a| a.is_none()) else { break };
        pool.macro_assigned[slot_idx] = Some(*coord);
        let entity = pool.macro_sprites[slot_idx];

        let (cx, cy) = *coord;
        let sprite_x = cx as f32 * CHUNK_SIZE + CHUNK_SIZE / 2.0 - half_map_width;
        let sprite_y = half_map_height - cy as f32 * CHUNK_SIZE - CHUNK_SIZE / 2.0;

        if let Ok((mut transform, mut sprite, mut vis)) = sprite_query.get_mut(entity) {
            transform.translation = Vec3::new(sprite_x, sprite_y, 0.1);
            sprite.image = tex_handle;
            sprite.custom_size = Some(custom_size);
            *vis = Visibility::Inherited;
        }
    }

    for assignment in pool.macro_assigned.iter() {
        if let Some(coord) = assignment {
            if let Some(cached) = tile_cache.macro_tiles.get_mut(coord) {
                cached.last_used_frame = frame;
            }
        }
    }
}

// ─── Layer Change ────────────────────────────────────────────────────────────

/// Handle layer changes from the UI and re-render cached BiomeMaps.
fn handle_layer_change(
    mut ui_state: ResMut<GeneratorUiState>,
    mut current_layer: ResMut<CurrentLayer>,
    mut images: ResMut<Assets<Image>>,
    mut tile_cache: Option<ResMut<TileCache>>,
    pool: Option<Res<SpritePool>>,
    mut sprite_query: Query<(&PoolSlot, &mut Sprite)>,
    norm_hints: Option<Res<GlobalNormHints>>,
) {
    // Sync current layer to UI state
    ui_state.current_layer = Some(current_layer.0);

    let Some(new_layer) = ui_state.layer_changed.take() else {
        return;
    };

    current_layer.0 = new_layer;

    // Re-render all cached tiles from their BiomeMap data
    if let Some(ref mut cache) = tile_cache {
        // Re-render macro tiles (shrunk maps can only render Biome layer)
        for (_, cached) in cache.macro_tiles.iter_mut() {
            let effective_layer = if cached.biome_map.is_shrunk() {
                rb_noise::NoiseLayer::Biome
            } else {
                new_layer
            };
            let hints_ref = norm_hints.as_ref().map(|h| &h.0);
            let image_data = cached.biome_map.to_layer_image_with_hints(effective_layer, hints_ref);
            let new_image = create_image(TILE_MAP_SIZE, TILE_MAP_SIZE, image_data);
            cached.texture = images.add(new_image);
        }

        // Update assigned pool sprite images
        if let Some(ref pool) = pool {
            for (i, assignment) in pool.macro_assigned.iter().enumerate() {
                if let Some(coord) = assignment {
                    if let Some(cached) = cache.macro_tiles.get(coord) {
                        let entity = pool.macro_sprites[i];
                        if let Ok((_, mut sprite)) = sprite_query.get_mut(entity) {
                            sprite.image = cached.texture.clone();
                        }
                    }
                }
            }
        }
    }
}

fn log_mode_transition(
    mut events: EventReader<ModeTransitionEvent>,
) {
    for event in events.read() {
        println!("Mode: {} → {}", event.from.name(), event.to.name());
    }
}

fn highlight_info_ui(
    mut contexts: EguiContexts,
    info: Res<HighlightInfo>,
    cursor_pos: Res<CursorWorldPos>,
    selected_chunk: Option<Res<SelectedChunk>>,
) {
    egui::Window::new("Tile Info")
        .anchor(egui::Align2::RIGHT_TOP, [-8.0, 8.0])
        .resizable(false)
        .collapsible(false)
        .show(contexts.ctx_mut(), |ui| {
            if let Some(ref sel) = selected_chunk {
                let (sx, sy) = sel.chunk_coord;
                ui.label(format!("Selected: ({sx}, {sy})"));
                ui.label("Press F4 to launch");
                ui.separator();
            }
            if !info.active {
                ui.label("(no tile)");
                return;
            }
            ui.label(format!("Domain: {}", info.domain));
            ui.label(format!("Tier: {}", info.tier));
            ui.label(format!("Tile: ({}, {})", info.tile_coord.0, info.tile_coord.1));
            ui.label(format!("World: ({:.1}, {:.1})", info.world_pos.0, info.world_pos.1));
            ui.label(format!("Cursor: ({:.1}, {:.1})", cursor_pos.0.x, cursor_pos.0.y));
        });
}

/// Regenerate world: clear caches, regenerate macro, let tiles re-stream.
fn regenerate_world(
    mut commands: Commands,
    mut regen_request: ResMut<RegenerationRequest>,
    world_def: Res<WorldDefinition>,
    mut textures: ResMut<MacroBiomeData>,
    mut tile_cache: ResMut<TileCache>,
    mut request_queue: ResMut<TileRequestQueue>,
    mut pool: ResMut<SpritePool>,
    mut sprite_query: Query<(&mut Transform, &mut Visibility), With<PoolSlot>>,
    ui_state: Res<GeneratorUiState>,
    mut next_phase: ResMut<NextState<AppPhase>>,
) {
    if !regen_request.pending {
        return;
    }
    regen_request.pending = false;

    let backend = ui_state.backend();
    let backend_name = if backend == NoiseBackend::Gpu { "GPU" } else { "CPU" };
    println!("Regenerating world map with seed {} ({})...", world_def.seed, backend_name);

    // Clear caches and in-flight tasks
    tile_cache.macro_tiles.clear();
    request_queue.in_flight.clear();

    // Hide all pool sprites
    for i in 0..pool.macro_assigned.len() {
        pool.macro_assigned[i] = None;
        let entity = pool.macro_sprites[i];
        if let Ok((mut transform, mut vis)) = sprite_query.get_mut(entity) {
            *vis = Visibility::Hidden;
            transform.translation.x = -10000.0;
        }
    }
    // Regenerate macro biome data
    let biome_map = Arc::new(BiomeMap::generate_with_backend(world_def.seed, world_def.width, world_def.height, backend));
    println!("  Macro biome data generated successfully");

    textures.biome_map = biome_map;

    // Queue all macro tiles for pre-generation
    let chunks_x = (world_def.width as f32 / CHUNK_SIZE).ceil() as i32;
    let chunks_y = (world_def.height as f32 / CHUNK_SIZE).ceil() as i32;
    let mut remaining: Vec<(i32, i32)> = Vec::new();
    for cy in 0..chunks_y {
        for cx in 0..chunks_x {
            remaining.push((cx, cy));
        }
    }
    let total = remaining.len();
    commands.insert_resource(MacroPregenState {
        total,
        completed: 0,
        remaining,
        in_flight: Vec::new(),
        post_phase: MacroPostPhase::Generating,
    });

    next_phase.set(AppPhase::GeneratingMacro);
    println!("World regenerated. Pre-generating {} macro tiles...", total);
}

/// Regenerate lifegen: run the full LifeGen pipeline using stored MesoTerrainView.
fn regenerate_lifegen(
    mut commands: Commands,
    mut regen_request: ResMut<RegenerateLifeGenRequest>,
    world_def: Res<WorldDefinition>,
    stored_terrain: Option<Res<StoredTerrainView>>,
    mut overlay_state: ResMut<LifeGenOverlayState>,
) {
    if !regen_request.pending {
        return;
    }
    regen_request.pending = false;

    let Some(terrain) = stored_terrain else {
        eprintln!("Cannot regenerate lifegen: no stored MesoTerrainView available");
        return;
    };

    let civ_seed = world_def.civ_seed;
    println!("Regenerating lifegen with civ_seed {}...", civ_seed);

    let lifegen = rb_world::lifegen::generate(&terrain.0, civ_seed);
    println!(
        "  LifeGenData populated: {} provinces, {} factions, {} settlements, {} road segments",
        lifegen.provinces.len(),
        lifegen.factions.len(),
        lifegen.settlement_seeds.len(),
        lifegen.road_segments.len(),
    );
    lifegen.save_debug_layers(std::path::Path::new("debug_layers"));
    commands.insert_resource(lifegen);

    overlay_state.data_generation += 1;
    overlay_state.current_layer.clear(); // Force re-render
}

// ─── Lifegen Overlay ─────────────────────────────────────────────────────────

fn manage_lifegen_overlay(
    mut commands: Commands,
    lifegen: Option<Res<LifeGenData>>,
    current_layer: Res<CurrentLifeGenLayer>,
    mut overlay_state: ResMut<LifeGenOverlayState>,
    mut images: ResMut<Assets<Image>>,
    mut overlay_query: Query<(Entity, &mut Sprite), With<LifeGenOverlay>>,
    world_def: Res<WorldDefinition>,
) {
    let Some(lifegen) = lifegen else {
        return;
    };

    let layer_name = match current_layer.0 {
        LifeGenLayer::Habitability => "habitability",
        LifeGenLayer::NavigationCost => "navigation_cost",
        LifeGenLayer::ResourceDesirability => "resource_desirability",
        LifeGenLayer::Factions | LifeGenLayer::PoliticalState => "factions",
        LifeGenLayer::Prosperity => "habitability",
        LifeGenLayer::Settlements => "settlements",
        _ => "composite",
    };

    let overlay_exists = !overlay_query.is_empty();
    if layer_name == overlay_state.current_layer && overlay_exists {
        return;
    }

    let rgba_data = lifegen.to_layer_image(layer_name);
    let image = create_image(lifegen.width, lifegen.height, rgba_data);
    let image_handle = images.add(image);

    if let Ok((_entity, mut sprite)) = overlay_query.get_single_mut() {
        sprite.image = image_handle;
    } else {
        commands.spawn((
            Sprite {
                image: image_handle,
                custom_size: Some(Vec2::new(
                    world_def.width as f32,
                    world_def.height as f32,
                )),
                ..default()
            },
            Transform::from_xyz(0.0, 0.0, 0.5),
            LifeGenOverlay,
        ));
    }

    overlay_state.current_layer = layer_name.to_string();
}

fn hide_lifegen_overlay(
    mut overlay_query: Query<&mut Visibility, With<LifeGenOverlay>>,
) {
    for mut vis in &mut overlay_query {
        *vis = Visibility::Hidden;
    }
}

fn show_lifegen_overlay(
    mut overlay_query: Query<&mut Visibility, With<LifeGenOverlay>>,
    mut overlay_state: ResMut<LifeGenOverlayState>,
) {
    for mut vis in &mut overlay_query {
        *vis = Visibility::Inherited;
    }
    overlay_state.current_layer.clear(); // Force re-render
}

// ─── Camera & Input ──────────────────────────────────────────────────────────

fn camera_zoom(
    mut scroll_events: EventReader<MouseWheel>,
    mut query: Query<&mut OrthographicProjection, With<Camera2d>>,
) {
    let mut scroll_delta = 0.0;

    for event in scroll_events.read() {
        scroll_delta += match event.unit {
            MouseScrollUnit::Line => event.y * 0.1,
            MouseScrollUnit::Pixel => event.y * 0.001,
        };
    }

    if scroll_delta == 0.0 {
        return;
    }

    for mut projection in &mut query {
        let zoom_factor = 1.0 - scroll_delta;
        projection.scale = (projection.scale * zoom_factor).clamp(0.05, 10.0);
    }
}

fn camera_pan(
    keyboard: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut motion_events: EventReader<bevy::input::mouse::MouseMotion>,
    mut query: Query<(&mut Transform, &OrthographicProjection), With<Camera2d>>,
    time: Res<Time>,
    mut contexts: EguiContexts,
) {
    let mut pan_delta = Vec2::ZERO;

    let pan_speed = 300.0;
    if keyboard.pressed(KeyCode::ArrowLeft) {
        pan_delta.x -= pan_speed * time.delta_secs();
    }
    if keyboard.pressed(KeyCode::ArrowRight) {
        pan_delta.x += pan_speed * time.delta_secs();
    }
    if keyboard.pressed(KeyCode::ArrowUp) {
        pan_delta.y += pan_speed * time.delta_secs();
    }
    if keyboard.pressed(KeyCode::ArrowDown) {
        pan_delta.y -= pan_speed * time.delta_secs();
    }

    let over_ui = contexts.ctx_mut().is_pointer_over_area();
    if mouse.pressed(MouseButton::Left) && !over_ui {
        for event in motion_events.read() {
            pan_delta.x -= event.delta.x;
            pan_delta.y += event.delta.y;
        }
    } else {
        motion_events.clear();
    }

    if pan_delta == Vec2::ZERO {
        return;
    }

    for (mut transform, projection) in &mut query {
        transform.translation.x += pan_delta.x * projection.scale;
        transform.translation.y += pan_delta.y * projection.scale;
    }
}

fn update_cursor_world_pos(
    windows: Query<&Window>,
    camera_query: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    mut cursor_pos: ResMut<CursorWorldPos>,
) {
    let Ok(window) = windows.get_single() else { return };
    let Some(cursor_screen_pos) = window.cursor_position() else { return };
    let Ok((camera, camera_transform)) = camera_query.get_single() else { return };

    if let Ok(world_pos) = camera.viewport_to_world_2d(camera_transform, cursor_screen_pos) {
        cursor_pos.0 = world_pos;
    }
}

fn update_chunk_highlight(
    cursor_pos: Res<CursorWorldPos>,
    world_def: Res<WorldDefinition>,
    mut highlight_query: Query<(&mut Transform, &mut Sprite), With<ChunkHighlight>>,
    mut contexts: EguiContexts,
    mut highlight_info: ResMut<HighlightInfo>,
) {
    let Ok((mut highlight_transform, mut highlight_sprite)) = highlight_query.get_single_mut() else { return };

    if contexts.ctx_mut().is_pointer_over_area() {
        highlight_transform.translation.x = -10000.0;
        highlight_info.active = false;
        return;
    }

    let chunk_size = CHUNK_SIZE;
    let tier_name = "Macro";
    highlight_sprite.custom_size = Some(Vec2::splat(chunk_size));

    let half_width = world_def.width as f32 / 2.0;
    let half_height = world_def.height as f32 / 2.0;

    let map_x = cursor_pos.0.x + half_width;
    let map_y = half_height - cursor_pos.0.y;

    if map_x < 0.0 || map_x >= world_def.width as f32 || map_y < 0.0 || map_y >= world_def.height as f32 {
        highlight_transform.translation.x = -10000.0;
        highlight_info.active = false;
        return;
    }

    let chunk_x = (map_x / chunk_size).floor() * chunk_size;
    let chunk_y = (map_y / chunk_size).floor() * chunk_size;

    let tile_ix = (map_x / chunk_size).floor() as i32;
    let tile_iy = (map_y / chunk_size).floor() as i32;

    let world_x = chunk_x + chunk_size / 2.0 - half_width;
    let world_y = half_height - chunk_y - chunk_size / 2.0;

    highlight_transform.translation.x = world_x;
    highlight_transform.translation.y = world_y;

    highlight_info.active = true;
    highlight_info.tile_coord = (tile_ix, tile_iy);
    highlight_info.world_pos = (chunk_x, chunk_y);
    highlight_info.tier = tier_name;
    highlight_info.domain = "Terrain";
}

/// Calculate which chunks are visible in the camera viewport.
fn calculate_visible_chunks(
    camera_query: Query<(&Transform, &OrthographicProjection), With<Camera2d>>,
    windows: Query<&Window>,
    mut visible_range: ResMut<VisibleChunkRange>,
    world_def: Res<WorldDefinition>,
) {
    let Ok((camera_transform, projection)) = camera_query.get_single() else { return };
    let Ok(window) = windows.get_single() else { return };

    let camera_pos = camera_transform.translation;
    let scale = projection.scale;

    let half_viewport_width = (window.width() / 2.0) * scale;
    let half_viewport_height = (window.height() / 2.0) * scale;

    let world_min_x = camera_pos.x - half_viewport_width;
    let world_max_x = camera_pos.x + half_viewport_width;
    let world_min_y = camera_pos.y - half_viewport_height;
    let world_max_y = camera_pos.y + half_viewport_height;

    let half_map_width = world_def.width as f32 / 2.0;
    let half_map_height = world_def.height as f32 / 2.0;

    let map_min_x = world_min_x + half_map_width;
    let map_max_x = world_max_x + half_map_width;
    let map_min_y = half_map_height - world_max_y;
    let map_max_y = half_map_height - world_min_y;

    let padding = 1;
    visible_range.min_x = ((map_min_x / CHUNK_SIZE).floor() as i32 - padding).max(0);
    visible_range.max_x = ((map_max_x / CHUNK_SIZE).ceil() as i32 + padding)
        .min((world_def.width as f32 / CHUNK_SIZE).ceil() as i32 - 1);
    visible_range.min_y = ((map_min_y / CHUNK_SIZE).floor() as i32 - padding).max(0);
    visible_range.max_y = ((map_max_y / CHUNK_SIZE).ceil() as i32 + padding)
        .min((world_def.height as f32 / CHUNK_SIZE).ceil() as i32 - 1);
}

// ─── Level Chunk Streaming ──────────────────────────────────────────────────

/// Level chunk load radius (in level chunks around the player).
const LEVEL_LOAD_RADIUS: i32 = 5;

/// Level chunk unload radius.
const LEVEL_UNLOAD_RADIUS: i32 = 7;

/// Each level chunk is 64 tiles.
const LEVEL_CHUNK_TILES: f32 = 64.0;

/// An in-flight level chunk generation task.
struct LevelChunkTask {
    coord: (i32, i32),
    task: Task<((i32, i32), Arc<BiomeMap>)>,
}

/// Queue of in-flight level chunk generation tasks.
#[derive(Resource, Default)]
struct LevelChunkQueue {
    in_flight: Vec<LevelChunkTask>,
}

/// Hide world map sprites when entering play mode.
fn hide_world_map_on_play(
    pool: Option<ResMut<SpritePool>>,
    mut sprite_query: Query<(&mut Transform, &mut Visibility), With<PoolSlot>>,
    highlight_query: Query<Entity, With<ChunkHighlight>>,
) {
    // Hide all pool sprites
    if let Some(mut pool) = pool {
        for i in 0..pool.macro_assigned.len() {
            pool.macro_assigned[i] = None;
            let entity = pool.macro_sprites[i];
            if let Ok((mut transform, mut vis)) = sprite_query.get_mut(entity) {
                *vis = Visibility::Hidden;
                transform.translation.x = -10000.0;
            }
        }
    }

    // Hide chunk highlight
    for entity in &highlight_query {
        if let Ok((mut transform, mut vis)) = sprite_query.get_mut(entity) {
            *vis = Visibility::Hidden;
            transform.translation.x = -10000.0;
        }
    }
}

/// Load level chunks around the player's position.
fn level_chunk_load_system(
    level: Res<PlayableLevel>,
    player_query: Query<&Transform, With<Player>>,
    loaded_chunks: Res<LoadedChunks>,
    mut queue: ResMut<LevelChunkQueue>,
    world_textures: Option<Res<MacroBiomeData>>,
    ui_state: Res<GeneratorUiState>,
    global_rivers: Option<Res<GlobalRiverNetwork>>,
) {
    let Some(world_textures) = world_textures else { return };
    let Ok(player_transform) = player_query.get_single() else { return };

    let player_pos = player_transform.translation;
    let player_chunk_x = (player_pos.x / LEVEL_CHUNK_TILES).floor() as i32;
    let player_chunk_y = ((-player_pos.y) / LEVEL_CHUNK_TILES).floor() as i32; // Y-flip

    let seed = level.seed;
    let height = level.world_height;
    let backend = ui_state.backend();
    let river_net = global_rivers.map(|r| r.network.clone());

    // Collect in-flight coords
    let in_flight_coords: HashSet<(i32, i32)> = queue.in_flight.iter()
        .map(|t| t.coord)
        .collect();

    for dy in -LEVEL_LOAD_RADIUS..=LEVEL_LOAD_RADIUS {
        for dx in -LEVEL_LOAD_RADIUS..=LEVEL_LOAD_RADIUS {
            let cx = player_chunk_x + dx;
            let cy = player_chunk_y + dy;
            let coord = (cx, cy);

            if loaded_chunks.chunks.contains_key(&coord) || in_flight_coords.contains(&coord) {
                continue;
            }

            if queue.in_flight.len() >= MAX_CONCURRENT_TILES {
                return;
            }

            // Map level chunk to world coordinates
            let world_x = level.origin.x + cx as f64 * MICRO_WORLD_SIZE;
            let world_y = level.origin.y + cy as f64 * MICRO_WORLD_SIZE;

            let macro_map = world_textures.biome_map.clone();
            let river_net_clone = river_net.clone();
            let task = AsyncComputeTaskPool::get().spawn(async move {
                let river_ref = river_net_clone.as_ref();
                let biome_map = BiomeMap::generate_meso_full_with_backend(
                    seed,
                    world_x,
                    world_y,
                    MICRO_WORLD_SIZE,
                    TILE_MAP_SIZE,
                    height,
                    3, // micro detail level
                    None,
                    backend,
                    Some(&macro_map),
                    river_ref,
                );
                (coord, Arc::new(biome_map))
            });

            queue.in_flight.push(LevelChunkTask { coord, task });
        }
    }
}

/// Poll completed level chunk tasks and spawn sprites.
fn level_chunk_poll_system(
    mut commands: Commands,
    mut queue: ResMut<LevelChunkQueue>,
    mut loaded_chunks: ResMut<LoadedChunks>,
    mut images: ResMut<Assets<Image>>,
    current_layer: Res<CurrentLayer>,
) {
    let mut completed = 0;
    let mut i = 0;
    while i < queue.in_flight.len() && completed < POLL_BUDGET {
        if let Some(result) = block_on(poll_once(&mut queue.in_flight[i].task)) {
            queue.in_flight.swap_remove(i);
            let (coord, biome_map) = result;

            let image_data = biome_map.to_layer_image(current_layer.0);
            let image = create_image(TILE_MAP_SIZE, TILE_MAP_SIZE, image_data);
            let texture = images.add(image);

            // Position in level space: each chunk is LEVEL_CHUNK_TILES pixels
            // Y is flipped (positive Y = up in Bevy, but chunks grow downward)
            let sprite_x = coord.0 as f32 * LEVEL_CHUNK_TILES + LEVEL_CHUNK_TILES / 2.0;
            let sprite_y = -(coord.1 as f32 * LEVEL_CHUNK_TILES + LEVEL_CHUNK_TILES / 2.0);

            let entity = commands.spawn((
                Sprite {
                    image: texture,
                    custom_size: Some(Vec2::splat(LEVEL_CHUNK_TILES)),
                    ..default()
                },
                Transform::from_xyz(sprite_x, sprite_y, 0.0),
                LevelChunk { coord },
            )).id();

            loaded_chunks.chunks.insert(coord, entity);
            completed += 1;
        } else {
            i += 1;
        }
    }
}

/// Unload level chunks beyond the unload radius.
fn level_chunk_unload_system(
    mut commands: Commands,
    mut loaded_chunks: ResMut<LoadedChunks>,
    player_query: Query<&Transform, With<Player>>,
) {
    let Ok(player_transform) = player_query.get_single() else { return };

    let player_pos = player_transform.translation;
    let player_chunk_x = (player_pos.x / LEVEL_CHUNK_TILES).floor() as i32;
    let player_chunk_y = ((-player_pos.y) / LEVEL_CHUNK_TILES).floor() as i32;

    let to_remove: Vec<(i32, i32)> = loaded_chunks.chunks.keys()
        .filter(|(cx, cy)| {
            let dx = (cx - player_chunk_x).abs();
            let dy = (cy - player_chunk_y).abs();
            dx > LEVEL_UNLOAD_RADIUS || dy > LEVEL_UNLOAD_RADIUS
        })
        .copied()
        .collect();

    for coord in to_remove {
        if let Some(entity) = loaded_chunks.chunks.remove(&coord) {
            commands.entity(entity).despawn();
        }
    }
}

// ─── Click-to-Select-Chunk ───────────────────────────────────────────────────

/// Click on the world map to select a chunk. Just stores SelectedChunk — no mode switch.
/// The user then presses F4 to enter LevelLauncher, which auto-starts micro generation.
fn click_to_select_chunk(
    mut commands: Commands,
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_query: Query<(&Camera, &GlobalTransform)>,
    world_def: Res<WorldDefinition>,
    mut contexts: EguiContexts,
) {
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }

    if contexts.ctx_mut().is_pointer_over_area() {
        return;
    }

    let Ok(window) = windows.get_single() else { return };
    let Some(cursor_pos) = window.cursor_position() else { return };
    let Ok((camera, camera_transform)) = camera_query.get_single() else { return };
    let Ok(world_pos) = camera.viewport_to_world_2d(camera_transform, cursor_pos) else { return };

    let map_x = world_pos.x + (world_def.width as f32 / 2.0);
    let map_y = (world_def.height as f32 / 2.0) - world_pos.y;

    if map_x < 0.0 || map_x >= world_def.width as f32 || map_y < 0.0 || map_y >= world_def.height as f32 {
        return;
    }

    let chunk_x = (map_x as i32) / CHUNK_SIZE_I as i32;
    let chunk_y = (map_y as i32) / CHUNK_SIZE_I as i32;

    let origin = WorldPos::new(
        chunk_x as f64 * CHUNK_SIZE as f64,
        chunk_y as f64 * CHUNK_SIZE as f64,
    );

    commands.insert_resource(SelectedChunk {
        chunk_coord: (chunk_x, chunk_y),
        origin,
    });

    println!("Selected chunk ({}, {}) — press F4 to launch", chunk_x, chunk_y);
}

/// Update the persistent selection highlight to show the selected chunk.
fn update_chunk_selection_highlight(
    selected: Option<Res<SelectedChunk>>,
    world_def: Res<WorldDefinition>,
    mut query: Query<(&mut Transform, &mut Visibility), With<ChunkSelectionHighlight>>,
) {
    let Ok((mut transform, mut vis)) = query.get_single_mut() else { return };

    let Some(selected) = selected else {
        *vis = Visibility::Hidden;
        transform.translation.x = -10000.0;
        return;
    };

    let half_width = world_def.width as f32 / 2.0;
    let half_height = world_def.height as f32 / 2.0;

    let (cx, cy) = selected.chunk_coord;
    let sprite_x = cx as f32 * CHUNK_SIZE + CHUNK_SIZE / 2.0 - half_width;
    let sprite_y = half_height - cy as f32 * CHUNK_SIZE - CHUNK_SIZE / 2.0;

    transform.translation.x = sprite_x;
    transform.translation.y = sprite_y;
    *vis = Visibility::Inherited;
}

// ─── Launcher Systems ────────────────────────────────────────────────────────

/// On entering LevelLauncher: hide world map, show the selected macro chunk enlarged.
fn enter_launcher_macro_view(
    mut commands: Commands,
    selected: Option<Res<SelectedChunk>>,
    tile_cache: Option<Res<TileCache>>,
    mut camera_query: Query<(&mut Transform, &mut OrthographicProjection), With<Camera2d>>,
    mut pool_query: Query<&mut Visibility, With<PoolSlot>>,
    mut highlight_query: Query<&mut Visibility, (With<ChunkHighlight>, Without<PoolSlot>, Without<ChunkSelectionHighlight>)>,
    mut selection_query: Query<&mut Visibility, (With<ChunkSelectionHighlight>, Without<PoolSlot>, Without<ChunkHighlight>)>,
) {
    commands.insert_resource(LauncherPhase::MacroView);

    // Hide all world map pool sprites
    for mut vis in &mut pool_query {
        *vis = Visibility::Hidden;
    }
    // Hide chunk highlight and selection highlight
    for mut vis in &mut highlight_query {
        *vis = Visibility::Hidden;
    }
    for mut vis in &mut selection_query {
        *vis = Visibility::Hidden;
    }

    let Some(selected) = selected else { return };
    let Some(tile_cache) = tile_cache else { return };
    let Ok((mut cam_transform, mut projection)) = camera_query.get_single_mut() else { return };

    // Show the macro tile as a large centered sprite
    if let Some(cached) = tile_cache.macro_tiles.get(&selected.chunk_coord) {
        commands.spawn((
            Sprite {
                image: cached.texture.clone(),
                custom_size: Some(Vec2::splat(CHUNK_SIZE * 8.0)), // 512px display
                ..default()
            },
            Transform::from_xyz(0.0, 0.0, 0.5),
            LauncherMacroSprite,
        ));
    }

    // Center camera
    cam_transform.translation = Vec3::new(0.0, 0.0, cam_transform.translation.z);
    projection.scale = 1.0;
}

/// Handle "Generate Mesomap" — start async generation of 64 meso tiles.
fn handle_generate_meso_request(
    mut commands: Commands,
    selected: Option<Res<SelectedChunk>>,
    macro_sprites: Query<Entity, With<LauncherMacroSprite>>,
) {
    commands.remove_resource::<GenerateMesoRequest>();
    let Some(selected) = selected else { return };

    // Despawn the macro sprite
    for entity in &macro_sprites {
        commands.entity(entity).despawn();
    }

    // Queue 64 meso tiles for generation
    let mut remaining = Vec::with_capacity(64);
    for my in 0..MESO_GRID_SIZE {
        for mx in 0..MESO_GRID_SIZE {
            remaining.push((mx, my));
        }
    }

    commands.insert_resource(MesoPregenState {
        total: remaining.len(),
        completed: 0,
        remaining,
        in_flight: Vec::new(),
    });
    commands.insert_resource(MesoTileCache::default());
    commands.insert_resource(LauncherPhase::GeneratingMeso);

    println!(
        "Generating 64 meso tiles for chunk ({}, {})",
        selected.chunk_coord.0, selected.chunk_coord.1
    );
}

/// Dispatch async meso tile generation tasks.
fn dispatch_meso_pregen(
    mut pregen: ResMut<MesoPregenState>,
    world_textures: Option<Res<MacroBiomeData>>,
    world_def: Res<WorldDefinition>,
    selected: Option<Res<SelectedChunk>>,
    ui_state: Res<GeneratorUiState>,
    global_rivers: Option<Res<GlobalRiverNetwork>>,
) {
    let Some(world_textures) = world_textures else { return };
    let Some(selected) = selected else { return };

    let seed = world_def.seed;
    let height = world_def.height as f64;
    let backend = ui_state.backend();
    let chunk_origin = selected.origin;
    let river_net = global_rivers.map(|r| r.network.clone());

    while pregen.in_flight.len() < MACRO_PREGEN_CONCURRENCY {
        let Some(coord) = pregen.remaining.pop() else { break };
        let macro_map = world_textures.biome_map.clone();
        let river_net_clone = river_net.clone();

        let world_x = chunk_origin.x + coord.0 as f64 * MESO_WORLD_SIZE;
        let world_y = chunk_origin.y + coord.1 as f64 * MESO_WORLD_SIZE;

        let task = AsyncComputeTaskPool::get().spawn(async move {
            let river_ref = river_net_clone.as_ref();
            let biome_map = BiomeMap::generate_meso_full_with_backend(
                seed, world_x, world_y, MESO_WORLD_SIZE, TILE_MAP_SIZE, height,
                2, // meso detail level
                None, backend, Some(&macro_map),
                river_ref,
            );
            (coord, Arc::new(biome_map))
        });

        pregen.in_flight.push(MesoInFlightTile { task });
    }
}

/// Poll meso tile tasks; when all done, spawn the grid and transition to MesoView.
fn poll_meso_pregen(
    mut commands: Commands,
    mut pregen: ResMut<MesoPregenState>,
    mut meso_cache: ResMut<MesoTileCache>,
    mut images: ResMut<Assets<Image>>,
    current_layer: Res<CurrentLayer>,
) {
    let mut i = 0;
    while i < pregen.in_flight.len() {
        if let Some(result) = block_on(poll_once(&mut pregen.in_flight[i].task)) {
            pregen.in_flight.swap_remove(i);
            let (coord, biome_map) = result;

            let image_data = biome_map.to_layer_image(current_layer.0);
            let image = create_image(TILE_MAP_SIZE, TILE_MAP_SIZE, image_data);
            let texture = images.add(image);

            meso_cache.tiles.insert(coord, MesoCachedTile { _biome_map: biome_map, texture });
            pregen.completed += 1;
        } else {
            i += 1;
        }
    }

    if pregen.completed >= pregen.total {
        println!("All {} meso tiles generated.", pregen.total);
        commands.remove_resource::<MesoPregenState>();

        // Spawn 8×8 grid of meso tile sprites
        let grid_size = MESO_GRID_SIZE as f32 * MESO_TILE_DISPLAY_PX;
        let half_grid = grid_size / 2.0;

        for my in 0..MESO_GRID_SIZE {
            for mx in 0..MESO_GRID_SIZE {
                let coord = (mx, my);
                let Some(cached) = meso_cache.tiles.get(&coord) else { continue };

                let sprite_x = mx as f32 * MESO_TILE_DISPLAY_PX + MESO_TILE_DISPLAY_PX / 2.0 - half_grid;
                let sprite_y = half_grid - my as f32 * MESO_TILE_DISPLAY_PX - MESO_TILE_DISPLAY_PX / 2.0;

                let entity = commands.spawn((
                    Sprite {
                        image: cached.texture.clone(),
                        custom_size: Some(Vec2::splat(MESO_TILE_DISPLAY_PX)),
                        ..default()
                    },
                    Transform::from_xyz(sprite_x, sprite_y, 0.5),
                    LauncherMesoSprite,
                )).id();
                meso_cache.sprite_entities.push(entity);
            }
        }

        // Spawn meso highlight
        commands.spawn((
            Sprite {
                color: Color::srgba(1.0, 1.0, 0.0, 0.3),
                custom_size: Some(Vec2::splat(MESO_TILE_DISPLAY_PX)),
                ..default()
            },
            Transform::from_xyz(-10000.0, -10000.0, 0.8),
            MesoHighlight,
        ));

        commands.insert_resource(LauncherPhase::MesoView);
    }
}

/// Allow zoom in meso view.
fn launcher_camera_zoom(
    mut scroll_events: EventReader<MouseWheel>,
    mut query: Query<&mut OrthographicProjection, With<Camera2d>>,
) {
    let mut scroll_delta = 0.0;
    for event in scroll_events.read() {
        scroll_delta += match event.unit {
            MouseScrollUnit::Line => event.y * 0.1,
            MouseScrollUnit::Pixel => event.y * 0.001,
        };
    }
    if scroll_delta == 0.0 { return; }
    for mut projection in &mut query {
        let zoom_factor = 1.0 - scroll_delta;
        projection.scale = (projection.scale * zoom_factor).clamp(0.2, 3.0);
    }
}

/// Click to select a meso tile in the grid.
fn click_to_select_meso_tile(
    mut commands: Commands,
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_query: Query<(&Camera, &GlobalTransform)>,
    selected_chunk: Option<Res<SelectedChunk>>,
    mut contexts: EguiContexts,
) {
    if !mouse.just_pressed(MouseButton::Left) { return; }
    if contexts.ctx_mut().is_pointer_over_area() { return; }
    let Some(selected_chunk) = selected_chunk else { return };

    let Ok(window) = windows.get_single() else { return };
    let Some(cursor_pos) = window.cursor_position() else { return };
    let Ok((camera, camera_transform)) = camera_query.get_single() else { return };
    let Ok(world_pos) = camera.viewport_to_world_2d(camera_transform, cursor_pos) else { return };

    let grid_size = MESO_GRID_SIZE as f32 * MESO_TILE_DISPLAY_PX;
    let half_grid = grid_size / 2.0;

    let mx = ((world_pos.x + half_grid) / MESO_TILE_DISPLAY_PX).floor() as i32;
    let my = ((half_grid - world_pos.y) / MESO_TILE_DISPLAY_PX).floor() as i32;

    if mx < 0 || mx >= MESO_GRID_SIZE || my < 0 || my >= MESO_GRID_SIZE { return; }

    let origin = WorldPos::new(
        selected_chunk.origin.x + mx as f64 * MESO_WORLD_SIZE,
        selected_chunk.origin.y + my as f64 * MESO_WORLD_SIZE,
    );

    commands.insert_resource(SelectedMesoTile {
        meso_coord: (mx, my),
        origin,
    });

    println!("Selected meso tile ({}, {})", mx, my);
}

/// Update the meso highlight overlay to follow cursor.
fn update_meso_highlight(
    mut highlight_query: Query<&mut Transform, With<MesoHighlight>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_query: Query<(&Camera, &GlobalTransform)>,
    mut contexts: EguiContexts,
) {
    let Ok(mut highlight_tf) = highlight_query.get_single_mut() else { return };

    if contexts.ctx_mut().is_pointer_over_area() {
        highlight_tf.translation.x = -10000.0;
        return;
    }

    let Ok(window) = windows.get_single() else { return };
    let Some(cursor_screen) = window.cursor_position() else {
        highlight_tf.translation.x = -10000.0;
        return;
    };
    let Ok((camera, camera_transform)) = camera_query.get_single() else { return };
    let Ok(world_pos) = camera.viewport_to_world_2d(camera_transform, cursor_screen) else { return };

    let grid_size = MESO_GRID_SIZE as f32 * MESO_TILE_DISPLAY_PX;
    let half_grid = grid_size / 2.0;

    let mx = ((world_pos.x + half_grid) / MESO_TILE_DISPLAY_PX).floor() as i32;
    let my = ((half_grid - world_pos.y) / MESO_TILE_DISPLAY_PX).floor() as i32;

    if mx < 0 || mx >= MESO_GRID_SIZE || my < 0 || my >= MESO_GRID_SIZE {
        highlight_tf.translation.x = -10000.0;
        return;
    }

    let sprite_x = mx as f32 * MESO_TILE_DISPLAY_PX + MESO_TILE_DISPLAY_PX / 2.0 - half_grid;
    let sprite_y = half_grid - my as f32 * MESO_TILE_DISPLAY_PX - MESO_TILE_DISPLAY_PX / 2.0;
    highlight_tf.translation.x = sprite_x;
    highlight_tf.translation.y = sprite_y;
}

/// Handle "Launch Level" — start async generation of 64 micro tiles.
fn handle_launch_level_request(
    mut commands: Commands,
    selected_meso: Option<Res<SelectedMesoTile>>,
    meso_sprites: Query<Entity, With<LauncherMesoSprite>>,
    meso_highlight: Query<Entity, With<MesoHighlight>>,
) {
    commands.remove_resource::<LaunchLevelRequest>();
    let Some(selected_meso) = selected_meso else { return };

    // Hide meso grid sprites (keep cache for ESC to return)
    for entity in &meso_sprites {
        commands.entity(entity).insert(Visibility::Hidden);
    }
    for entity in &meso_highlight {
        commands.entity(entity).insert(Visibility::Hidden);
    }

    // Queue 64 micro tiles for generation
    let mut remaining = Vec::with_capacity((MICRO_GRID_SIZE * MICRO_GRID_SIZE) as usize);
    for uy in 0..MICRO_GRID_SIZE {
        for ux in 0..MICRO_GRID_SIZE {
            remaining.push((ux, uy));
        }
    }

    commands.insert_resource(MicroPregenState {
        total: remaining.len(),
        completed: 0,
        remaining,
        in_flight: Vec::new(),
    });
    commands.insert_resource(MicroTileCache::default());
    commands.insert_resource(LauncherPhase::GeneratingMicro);

    println!(
        "Generating 64 micro tiles for meso tile ({}, {})",
        selected_meso.meso_coord.0, selected_meso.meso_coord.1
    );
}

/// Re-show meso grid sprites when returning from Playing to MesoView (ESC).
fn reshow_meso_on_return(
    phase: Option<Res<LauncherPhase>>,
    meso_sprites: Query<Entity, With<LauncherMesoSprite>>,
    meso_highlight: Query<Entity, With<MesoHighlight>>,
    mut commands: Commands,
    playing: Option<Res<PlayableLevel>>,
) {
    // Only act when phase just changed to MesoView and no PlayableLevel exists
    let Some(phase) = phase else { return };
    if *phase != LauncherPhase::MesoView || playing.is_some() { return; }
    if !phase.is_changed() { return; }

    for entity in &meso_sprites {
        commands.entity(entity).insert(Visibility::Inherited);
    }
    for entity in &meso_highlight {
        commands.entity(entity).insert(Visibility::Inherited);
    }
}

// ─── Micro Launcher Systems ──────────────────────────────────────────────────

/// Dispatch async micro tile generation tasks.
fn dispatch_micro_pregen(
    mut pregen: ResMut<MicroPregenState>,
    world_textures: Option<Res<MacroBiomeData>>,
    world_def: Res<WorldDefinition>,
    selected_meso: Option<Res<SelectedMesoTile>>,
    ui_state: Res<GeneratorUiState>,
    global_rivers: Option<Res<GlobalRiverNetwork>>,
) {
    let Some(world_textures) = world_textures else { return };
    let Some(selected_meso) = selected_meso else { return };

    let seed = world_def.seed;
    let height = world_def.height as f64;
    let backend = ui_state.backend();
    let meso_origin = selected_meso.origin;
    let river_net = global_rivers.map(|r| r.network.clone());

    while pregen.in_flight.len() < MACRO_PREGEN_CONCURRENCY {
        let Some(coord) = pregen.remaining.pop() else { break };
        let macro_map = world_textures.biome_map.clone();
        let river_net_clone = river_net.clone();

        let world_x = meso_origin.x + coord.0 as f64 * MICRO_WORLD_SIZE;
        let world_y = meso_origin.y + coord.1 as f64 * MICRO_WORLD_SIZE;

        let task = AsyncComputeTaskPool::get().spawn(async move {
            let river_ref = river_net_clone.as_ref();
            let biome_map = BiomeMap::generate_meso_full_with_backend(
                seed, world_x, world_y, MICRO_WORLD_SIZE, TILE_MAP_SIZE, height,
                3, // micro detail level
                None, backend, Some(&macro_map),
                river_ref,
            );
            (coord, Arc::new(biome_map))
        });

        pregen.in_flight.push(MicroInFlightTile { task });
    }
}

/// Poll micro tile tasks; when all done, spawn the grid and transition to MicroView.
fn poll_micro_pregen(
    mut commands: Commands,
    mut pregen: ResMut<MicroPregenState>,
    mut micro_cache: ResMut<MicroTileCache>,
    mut images: ResMut<Assets<Image>>,
    current_layer: Res<CurrentLayer>,
) {
    let mut i = 0;
    while i < pregen.in_flight.len() {
        if let Some(result) = block_on(poll_once(&mut pregen.in_flight[i].task)) {
            pregen.in_flight.swap_remove(i);
            let (coord, biome_map) = result;

            let image_data = biome_map.to_layer_image(current_layer.0);
            let image = create_image(TILE_MAP_SIZE, TILE_MAP_SIZE, image_data);
            let texture = images.add(image);

            micro_cache.tiles.insert(coord, MicroCachedTile { _biome_map: biome_map, texture });
            pregen.completed += 1;
        } else {
            i += 1;
        }
    }

    if pregen.completed >= pregen.total {
        println!("All {} micro tiles generated.", pregen.total);
        commands.remove_resource::<MicroPregenState>();

        // Spawn 32×32 grid of micro tile sprites
        let grid_size = MICRO_GRID_SIZE as f32 * MICRO_TILE_DISPLAY_PX;
        let half_grid = grid_size / 2.0;

        for uy in 0..MICRO_GRID_SIZE {
            for ux in 0..MICRO_GRID_SIZE {
                let coord = (ux, uy);
                let Some(cached) = micro_cache.tiles.get(&coord) else { continue };

                let sprite_x = ux as f32 * MICRO_TILE_DISPLAY_PX + MICRO_TILE_DISPLAY_PX / 2.0 - half_grid;
                let sprite_y = half_grid - uy as f32 * MICRO_TILE_DISPLAY_PX - MICRO_TILE_DISPLAY_PX / 2.0;

                let entity = commands.spawn((
                    Sprite {
                        image: cached.texture.clone(),
                        custom_size: Some(Vec2::splat(MICRO_TILE_DISPLAY_PX)),
                        ..default()
                    },
                    Transform::from_xyz(sprite_x, sprite_y, 0.5),
                    LauncherMicroSprite,
                )).id();
                micro_cache.sprite_entities.push(entity);
            }
        }

        // Spawn micro highlight
        commands.spawn((
            Sprite {
                color: Color::srgba(1.0, 0.5, 0.0, 0.3),
                custom_size: Some(Vec2::splat(MICRO_TILE_DISPLAY_PX)),
                ..default()
            },
            Transform::from_xyz(-10000.0, -10000.0, 0.8),
            MicroHighlight,
        ));

        commands.insert_resource(LauncherPhase::MicroView);
    }
}

/// Show progress bar during micro tile pre-generation.
fn micro_pregen_progress_ui(
    mut contexts: EguiContexts,
    pregen: Res<MicroPregenState>,
) {
    let ctx = contexts.ctx_mut();
    egui::Window::new("Generating Micro Tiles")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .fixed_size([300.0, 80.0])
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                let progress = pregen.completed as f32 / pregen.total as f32;
                ui.label(format!("Generating micro tiles: {}/{}", pregen.completed, pregen.total));
                ui.add_space(10.0);
                ui.add(egui::ProgressBar::new(progress).show_percentage());
            });
        });
}

/// Click to select a micro tile in the grid.
fn click_to_select_micro_tile(
    mut commands: Commands,
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_query: Query<(&Camera, &GlobalTransform)>,
    selected_meso: Option<Res<SelectedMesoTile>>,
    mut contexts: EguiContexts,
) {
    if !mouse.just_pressed(MouseButton::Left) { return; }
    if contexts.ctx_mut().is_pointer_over_area() { return; }
    let Some(selected_meso) = selected_meso else { return };

    let Ok(window) = windows.get_single() else { return };
    let Some(cursor_pos) = window.cursor_position() else { return };
    let Ok((camera, camera_transform)) = camera_query.get_single() else { return };
    let Ok(world_pos) = camera.viewport_to_world_2d(camera_transform, cursor_pos) else { return };

    let grid_size = MICRO_GRID_SIZE as f32 * MICRO_TILE_DISPLAY_PX;
    let half_grid = grid_size / 2.0;

    let ux = ((world_pos.x + half_grid) / MICRO_TILE_DISPLAY_PX).floor() as i32;
    let uy = ((half_grid - world_pos.y) / MICRO_TILE_DISPLAY_PX).floor() as i32;

    if ux < 0 || ux >= MICRO_GRID_SIZE || uy < 0 || uy >= MICRO_GRID_SIZE { return; }

    let origin = WorldPos::new(
        selected_meso.origin.x + ux as f64 * MICRO_WORLD_SIZE,
        selected_meso.origin.y + uy as f64 * MICRO_WORLD_SIZE,
    );

    commands.insert_resource(SelectedMicroTile {
        micro_coord: (ux, uy),
        origin,
    });

    println!("Selected micro tile ({}, {})", ux, uy);
}

/// Update the micro highlight overlay to follow cursor.
fn update_micro_highlight(
    mut highlight_query: Query<&mut Transform, With<MicroHighlight>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_query: Query<(&Camera, &GlobalTransform)>,
    mut contexts: EguiContexts,
) {
    let Ok(mut highlight_tf) = highlight_query.get_single_mut() else { return };

    if contexts.ctx_mut().is_pointer_over_area() {
        highlight_tf.translation.x = -10000.0;
        return;
    }

    let Ok(window) = windows.get_single() else { return };
    let Some(cursor_screen) = window.cursor_position() else {
        highlight_tf.translation.x = -10000.0;
        return;
    };
    let Ok((camera, camera_transform)) = camera_query.get_single() else { return };
    let Ok(world_pos) = camera.viewport_to_world_2d(camera_transform, cursor_screen) else { return };

    let grid_size = MICRO_GRID_SIZE as f32 * MICRO_TILE_DISPLAY_PX;
    let half_grid = grid_size / 2.0;

    let ux = ((world_pos.x + half_grid) / MICRO_TILE_DISPLAY_PX).floor() as i32;
    let uy = ((half_grid - world_pos.y) / MICRO_TILE_DISPLAY_PX).floor() as i32;

    if ux < 0 || ux >= MICRO_GRID_SIZE || uy < 0 || uy >= MICRO_GRID_SIZE {
        highlight_tf.translation.x = -10000.0;
        return;
    }

    let sprite_x = ux as f32 * MICRO_TILE_DISPLAY_PX + MICRO_TILE_DISPLAY_PX / 2.0 - half_grid;
    let sprite_y = half_grid - uy as f32 * MICRO_TILE_DISPLAY_PX - MICRO_TILE_DISPLAY_PX / 2.0;
    highlight_tf.translation.x = sprite_x;
    highlight_tf.translation.y = sprite_y;
}

/// Handle "Play" — consume StartPlayRequest, hide micro sprites, create PlayableLevel.
fn handle_start_play_request(
    mut commands: Commands,
    selected_micro: Option<Res<SelectedMicroTile>>,
    world_def: Res<WorldDefinition>,
    selected_chunk: Option<Res<SelectedChunk>>,
    micro_sprites: Query<Entity, With<LauncherMicroSprite>>,
    micro_highlight: Query<Entity, With<MicroHighlight>>,
) {
    commands.remove_resource::<StartPlayRequest>();
    let Some(selected_micro) = selected_micro else { return };
    let Some(selected_chunk) = selected_chunk else { return };

    // Hide micro grid sprites (keep cache for ESC to return)
    for entity in &micro_sprites {
        commands.entity(entity).insert(Visibility::Hidden);
    }
    for entity in &micro_highlight {
        commands.entity(entity).insert(Visibility::Hidden);
    }

    commands.insert_resource(PlayableLevel {
        origin: selected_micro.origin,
        chunk_coord: selected_chunk.chunk_coord,
        seed: world_def.seed,
        world_height: world_def.height as f64,
    });
    commands.init_resource::<LoadedChunks>();
    commands.insert_resource(LauncherPhase::Playing);

    println!(
        "Playing at micro tile ({}, {}), world ({:.2}, {:.2})",
        selected_micro.micro_coord.0, selected_micro.micro_coord.1,
        selected_micro.origin.x, selected_micro.origin.y
    );
}

/// ESC in MicroView: despawn micro grid, go back to MesoView.
fn escape_micro_view(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    phase: Option<Res<LauncherPhase>>,
    micro_sprites: Query<Entity, With<LauncherMicroSprite>>,
    micro_highlight: Query<Entity, With<MicroHighlight>>,
) {
    if !keyboard.just_pressed(KeyCode::Escape) { return; }
    if !phase.map_or(false, |p| *p == LauncherPhase::MicroView) { return; }

    for entity in &micro_sprites {
        commands.entity(entity).despawn();
    }
    for entity in &micro_highlight {
        commands.entity(entity).despawn();
    }
    commands.remove_resource::<MicroTileCache>();
    commands.remove_resource::<SelectedMicroTile>();
    commands.insert_resource(LauncherPhase::MesoView);

    println!("Returned to MesoView");
}

/// Re-show micro grid sprites when returning from Playing to MicroView (ESC).
fn reshow_micro_on_return(
    phase: Option<Res<LauncherPhase>>,
    micro_sprites: Query<Entity, With<LauncherMicroSprite>>,
    micro_highlight: Query<Entity, With<MicroHighlight>>,
    mut commands: Commands,
    playing: Option<Res<PlayableLevel>>,
) {
    let Some(phase) = phase else { return };
    if *phase != LauncherPhase::MicroView || playing.is_some() { return; }
    if !phase.is_changed() { return; }

    for entity in &micro_sprites {
        commands.entity(entity).insert(Visibility::Inherited);
    }
    for entity in &micro_highlight {
        commands.entity(entity).insert(Visibility::Inherited);
    }
}

/// Clean up all launcher-specific entities when leaving LevelLauncher mode.
fn cleanup_launcher_entities(
    mut commands: Commands,
    macro_sprites: Query<Entity, With<LauncherMacroSprite>>,
    meso_sprites: Query<Entity, With<LauncherMesoSprite>>,
    meso_highlight: Query<Entity, With<MesoHighlight>>,
    micro_sprites: Query<Entity, With<LauncherMicroSprite>>,
    micro_highlight: Query<Entity, With<MicroHighlight>>,
    mut pool_query: Query<&mut Visibility, With<PoolSlot>>,
    mut highlight_query: Query<&mut Visibility, (With<ChunkHighlight>, Without<PoolSlot>, Without<ChunkSelectionHighlight>)>,
    mut selection_query: Query<&mut Visibility, (With<ChunkSelectionHighlight>, Without<PoolSlot>, Without<ChunkHighlight>)>,
) {
    for entity in &macro_sprites {
        commands.entity(entity).despawn();
    }
    for entity in &meso_sprites {
        commands.entity(entity).despawn();
    }
    for entity in &meso_highlight {
        commands.entity(entity).despawn();
    }
    for entity in &micro_sprites {
        commands.entity(entity).despawn();
    }
    for entity in &micro_highlight {
        commands.entity(entity).despawn();
    }
    commands.remove_resource::<MesoTileCache>();
    commands.remove_resource::<MesoPregenState>();
    commands.remove_resource::<MicroTileCache>();
    commands.remove_resource::<MicroPregenState>();
    commands.remove_resource::<SelectedMicroTile>();

    // Re-show world map pool sprites, highlight, and selection highlight
    for mut vis in &mut pool_query {
        *vis = Visibility::Inherited;
    }
    for mut vis in &mut highlight_query {
        *vis = Visibility::Inherited;
    }
    for mut vis in &mut selection_query {
        *vis = Visibility::Inherited;
    }
}

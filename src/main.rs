use bevy::image::{ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on, poll_once};
use bevy_egui::{egui, EguiContexts};
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use rb_core::{AppMode, ModeTransitionEvent, PlayableLevel, SelectedChunk, WorldPos, handle_mode_shortcuts};
use rb_editor::{CurrentLayer, GeneratorUiState, RegenerationRequest};
use rb_noise::{BiomeMap, NoiseBackend};
use rb_player::Player;
use rb_tilemap::{LevelChunk, LoadedChunks};
use rb_world::WorldDefinition;
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
        .init_resource::<LevelChunkQueue>()
        // Ready phase - main game systems
        .add_systems(Update, (
            handle_mode_shortcuts,
            handle_layer_change.run_if(in_state(AppMode::WorldGenerator)),
            regenerate_world.run_if(in_state(AppMode::WorldGenerator)),
            log_mode_transition,
        ).run_if(in_state(AppPhase::Ready)))
        // Click on world map to select a chunk (no mode switch)
        .add_systems(Update,
            click_to_select_chunk
                .run_if(in_state(AppPhase::Ready)
                    .and(in_state(AppMode::WorldGenerator))
                    .and(not(resource_exists::<PlayableLevel>))),
        )
        // When entering LevelLauncher with a selected chunk, auto-start micro generation
        .add_systems(Update,
            auto_play_on_launcher_enter
                .run_if(in_state(AppPhase::Ready)
                    .and(in_state(AppMode::LevelLauncher))
                    .and(resource_exists::<SelectedChunk>)
                    .and(not(resource_exists::<PlayableLevel>))),
        )
        // World map systems (disabled during play mode)
        .add_systems(Update, (
            camera_zoom,
            camera_pan,
            calculate_visible_chunks,
            update_view_level,
            enqueue_and_dispatch_tiles,
            poll_tile_results,
            manage_tile_sprites,
            update_cursor_world_pos,
            update_chunk_highlight,
            highlight_info_ui,
        ).run_if(in_state(AppPhase::Ready).and(not(resource_exists::<PlayableLevel>))))
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

/// Number of pre-spawned meso pool sprites.
const MESO_POOL_SIZE: usize = 24;

/// Max cached macro tiles — sized to hold all 128 pre-generated tiles.
const MACRO_CACHE_MAX: usize = 128;

/// Max cached meso tiles (LRU eviction beyond this).
const MESO_CACHE_MAX: usize = 32;

/// Max concurrent async tile generation tasks (streaming).
const MAX_CONCURRENT_TILES: usize = 16;

/// Max concurrent async tile generation tasks during macro pre-generation.
const MACRO_PREGEN_CONCURRENCY: usize = 12;

/// Max tile completions to process per frame.
const POLL_BUDGET: usize = 16;

/// Meso tile covers this many world units (8×8 area at 512×512 pixels).
const MESO_WORLD_SIZE: f64 = 8.0;

/// Micro tile covers this many world units (0.25×0.25 area at 512×512 pixels).
const MICRO_WORLD_SIZE: f64 = 0.25;

// ─── Resources ───────────────────────────────────────────────────────────────

/// Marker resource to trigger generation start.
#[derive(Resource)]
struct GenerationStarted;

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

/// Marker component for the chunk highlight overlay.
#[derive(Component)]
struct ChunkHighlight;

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
}

/// Resource tracking cursor position in world space.
#[derive(Resource, Default)]
struct CursorWorldPos(Vec2);

/// Current zoom detail tier with hysteresis.
#[derive(Resource)]
struct ViewLevel {
    current: DetailTier,
    pending: Option<DetailTier>,
    frames_at_pending: u8,
}

impl Default for ViewLevel {
    fn default() -> Self {
        Self {
            current: DetailTier::Macro,
            pending: None,
            frames_at_pending: 0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
enum DetailTier {
    Macro,
    Meso,
}

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
    meso_sprites: Vec<Entity>,
    meso_assigned: Vec<Option<(i32, i32)>>,
}

/// A cached tile with BiomeMap data and texture.
struct CachedTile {
    biome_map: Arc<BiomeMap>,
    texture: Handle<Image>,
    last_used_frame: u64,
}

/// LRU tile cache for macro and meso tiers.
#[derive(Resource)]
struct TileCache {
    macro_tiles: HashMap<(i32, i32), CachedTile>,
    macro_max: usize,
    meso_tiles: HashMap<(i32, i32), CachedTile>,
    meso_max: usize,
    frame: u64,
}

impl Default for TileCache {
    fn default() -> Self {
        Self {
            macro_tiles: HashMap::new(),
            macro_max: MACRO_CACHE_MAX,
            meso_tiles: HashMap::new(),
            meso_max: MESO_CACHE_MAX,
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

    fn insert_meso(&mut self, coord: (i32, i32), tile: CachedTile) {
        if self.meso_tiles.len() >= self.meso_max {
            if let Some((&evict_coord, _)) = self.meso_tiles.iter()
                .min_by_key(|(_, t)| t.last_used_frame)
            {
                self.meso_tiles.remove(&evict_coord);
            }
        }
        self.meso_tiles.insert(coord, tile);
    }

}

/// An in-flight async tile generation task.
struct InFlightTile {
    coord: (i32, i32),
    tier: DetailTier,
    task: Task<((i32, i32), Arc<BiomeMap>)>,
}

/// Queue of in-flight tile requests.
#[derive(Resource, Default)]
struct TileRequestQueue {
    in_flight: Vec<InFlightTile>,
}

/// Application phase - config, generating, pre-generating macro tiles, or ready.
#[derive(States, Default, Clone, Eq, PartialEq, Hash, Debug)]
enum AppPhase {
    #[default]
    Config,
    Generating,        // Generate macro biome data
    GeneratingMacro,   // Pre-generate all 128 macro tiles
    Ready,
}

/// Tracks progress of macro tile pre-generation.
#[derive(Resource)]
struct MacroPregenState {
    total: usize,
    completed: usize,
    remaining: Vec<(i32, i32)>,
    in_flight: Vec<InFlightTile>,
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
    let biome_map = Arc::new(BiomeMap::generate_with_backend(seed, width, height, backend));
    println!("  Macro map generated successfully");

    commands.insert_resource(MacroBiomeData {
        biome_map,
    });

    // Spawn chunk highlight
    commands.spawn((
        Sprite {
            color: Color::srgba(1.0, 1.0, 0.8, 0.3),
            custom_size: Some(Vec2::splat(CHUNK_SIZE)),
            ..default()
        },
        Transform::from_xyz(-10000.0, -10000.0, 0.5),
        ChunkHighlight,
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

    let mut meso_sprites = Vec::with_capacity(MESO_POOL_SIZE);
    let mut meso_assigned = Vec::with_capacity(MESO_POOL_SIZE);
    for _i in 0..MESO_POOL_SIZE {
        let entity = commands.spawn((
            Sprite { ..default() },
            Transform::from_xyz(-10000.0, -10000.0, 0.2),
            Visibility::Hidden,
            PoolSlot,
        )).id();
        meso_sprites.push(entity);
        meso_assigned.push(None);
    }

    commands.insert_resource(SpritePool {
        macro_sprites,
        macro_assigned,
        meso_sprites,
        meso_assigned,
    });
    commands.insert_resource(TileCache::default());
    commands.insert_resource(TileRequestQueue::default());
    commands.insert_resource(ViewLevel::default());

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
) {
    let Some(world_textures) = world_textures else { return };
    let seed = world_def.seed;
    let height = world_def.height as f64;
    let backend = ui_state.backend();

    while pregen.in_flight.len() < MACRO_PREGEN_CONCURRENCY {
        let Some(coord) = pregen.remaining.pop() else { break };
        let macro_map = world_textures.biome_map.clone();
        let world_x = coord.0 as f64 * CHUNK_SIZE as f64;
        let world_y = coord.1 as f64 * CHUNK_SIZE as f64;

        let task = AsyncComputeTaskPool::get().spawn(async move {
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
            );
            (coord, Arc::new(biome_map))
        });

        pregen.in_flight.push(InFlightTile { coord, tier: DetailTier::Macro, task });
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

    if pregen.completed >= pregen.total {
        println!("All {} macro tiles pre-generated.", pregen.total);

        // Save debug layers by stitching all macro tiles — same data the app displays
        save_stitched_debug_layers(&tile_cache);

        commands.remove_resource::<MacroPregenState>();
        next_phase.set(AppPhase::Ready);
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

        if let Err(e) = full_img.save(&path) {
            eprintln!("Failed to save {}: {e}", path.display());
        } else {
            println!("  Saved {}", path.display());
        }
    }
}

/// Show progress bar during macro tile pre-generation.
fn macro_pregen_progress_ui(
    mut contexts: EguiContexts,
    pregen: Res<MacroPregenState>,
) {
    let ctx = contexts.ctx_mut();

    egui::CentralPanel::default()
        .frame(egui::Frame::default().fill(egui::Color32::from_rgb(30, 30, 30)))
        .show(ctx, |_| {});

    egui::Window::new("Pre-generating Macro Maps")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .fixed_size([350.0, 100.0])
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                let progress = pregen.completed as f32 / pregen.total as f32;
                ui.label(format!("Generating macro maps: {}/{}", pregen.completed, pregen.total));
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

// ─── View Level with Hysteresis ──────────────────────────────────────────────

/// Update view level based on camera scale with hysteresis to prevent flicker.
fn update_view_level(
    camera_query: Query<&OrthographicProjection, With<Camera2d>>,
    mut view_level: ResMut<ViewLevel>,
) {
    let Ok(projection) = camera_query.get_single() else { return };
    let scale = projection.scale;

    // Determine target tier based on scale with dead zones
    let target = match view_level.current {
        DetailTier::Macro => {
            if scale < 0.4 {
                Some(DetailTier::Meso)
            } else {
                None
            }
        }
        DetailTier::Meso => {
            if scale > 0.6 {
                Some(DetailTier::Macro)
            } else {
                None
            }
        }
    };

    match target {
        Some(tier) => {
            if view_level.pending == Some(tier) {
                view_level.frames_at_pending += 1;
                if view_level.frames_at_pending >= 3 {
                    view_level.current = tier;
                    view_level.pending = None;
                    view_level.frames_at_pending = 0;
                }
            } else {
                view_level.pending = Some(tier);
                view_level.frames_at_pending = 1;
            }
        }
        None => {
            view_level.pending = None;
            view_level.frames_at_pending = 0;
        }
    }
}

// ─── Tile Streaming ──────────────────────────────────────────────────────────

/// Enqueue tile generation tasks for visible chunks not yet cached.
fn enqueue_and_dispatch_tiles(
    visible_range: Res<VisibleChunkRange>,
    view_level: Res<ViewLevel>,
    tile_cache: Res<TileCache>,
    mut request_queue: ResMut<TileRequestQueue>,
    world_def: Res<WorldDefinition>,
    world_textures: Option<Res<MacroBiomeData>>,
    ui_state: Res<GeneratorUiState>,
    camera_query: Query<&Transform, With<Camera2d>>,
) {
    let Some(world_textures) = world_textures else { return };
    let Ok(camera_transform) = camera_query.get_single() else { return };
    let camera_pos = camera_transform.translation;

    let seed = world_def.seed;
    let height = world_def.height as f64;
    let backend = ui_state.backend();
    let half_map_width = world_def.width as f32 / 2.0;
    let half_map_height = world_def.height as f32 / 2.0;

    // Collect in-flight coords to avoid duplicates
    let in_flight_coords: HashSet<((i32, i32), DetailTier)> = request_queue.in_flight.iter()
        .map(|t| (t.coord, t.tier))
        .collect();

    // Collect needed macro tiles (always needed as base layer)
    let mut needed: Vec<((i32, i32), DetailTier, f32)> = Vec::new();

    {
        for cy in visible_range.min_y..=visible_range.max_y {
            for cx in visible_range.min_x..=visible_range.max_x {
                let coord = (cx, cy);
                if tile_cache.macro_tiles.contains_key(&coord) || in_flight_coords.contains(&(coord, DetailTier::Macro)) {
                    continue;
                }
                let sprite_x = cx as f32 * CHUNK_SIZE + CHUNK_SIZE / 2.0 - half_map_width;
                let sprite_y = half_map_height - cy as f32 * CHUNK_SIZE - CHUNK_SIZE / 2.0;
                let dist = (camera_pos.x - sprite_x).powi(2) + (camera_pos.y - sprite_y).powi(2);
                needed.push((coord, DetailTier::Macro, dist));
            }
        }
    }

    // Collect needed meso tiles when at meso level
    if view_level.current == DetailTier::Meso {
        let meso_per_chunk = (CHUNK_SIZE_I as f64 / MESO_WORLD_SIZE) as i32; // 8 meso tiles per macro chunk edge
        for cy in visible_range.min_y..=visible_range.max_y {
            for cx in visible_range.min_x..=visible_range.max_x {
                for my in 0..meso_per_chunk {
                    for mx in 0..meso_per_chunk {
                        let meso_coord = (cx * meso_per_chunk + mx, cy * meso_per_chunk + my);
                        if tile_cache.meso_tiles.contains_key(&meso_coord) || in_flight_coords.contains(&(meso_coord, DetailTier::Meso)) {
                            continue;
                        }
                        let world_x = meso_coord.0 as f64 * MESO_WORLD_SIZE;
                        let world_y = meso_coord.1 as f64 * MESO_WORLD_SIZE;
                        let sprite_x = world_x as f32 + MESO_WORLD_SIZE as f32 / 2.0 - half_map_width;
                        let sprite_y = half_map_height - world_y as f32 - MESO_WORLD_SIZE as f32 / 2.0;
                        let dist = (camera_pos.x - sprite_x).powi(2) + (camera_pos.y - sprite_y).powi(2);
                        needed.push((meso_coord, DetailTier::Meso, dist));
                    }
                }
            }
        }
    }

    // Sort by distance (closest first)
    needed.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));

    // Dispatch tasks up to max concurrent
    let macro_map = world_textures.biome_map.clone();
    for (coord, tier, _dist) in needed {
        if request_queue.in_flight.len() >= MAX_CONCURRENT_TILES {
            break;
        }

        let macro_map_clone = macro_map.clone();
        let (world_x, world_y, world_size, detail_level) = match tier {
            DetailTier::Macro => {
                let wx = coord.0 as f64 * CHUNK_SIZE as f64;
                let wy = coord.1 as f64 * CHUNK_SIZE as f64;
                (wx, wy, CHUNK_SIZE as f64, 1u32)
            }
            DetailTier::Meso => {
                let wx = coord.0 as f64 * MESO_WORLD_SIZE;
                let wy = coord.1 as f64 * MESO_WORLD_SIZE;
                (wx, wy, MESO_WORLD_SIZE, 2u32)
            }
        };

        let task = AsyncComputeTaskPool::get().spawn(async move {
            let biome_map = BiomeMap::generate_meso_full_with_backend(
                seed,
                world_x,
                world_y,
                world_size,
                TILE_MAP_SIZE,
                height,
                detail_level,
                None, // no progress tracking for streaming tiles
                backend,
                Some(&macro_map_clone),
            );
            (coord, Arc::new(biome_map))
        });

        request_queue.in_flight.push(InFlightTile { coord, tier, task });
    }
}

/// Poll in-flight tile tasks, up to POLL_BUDGET completions per frame.
fn poll_tile_results(
    mut request_queue: ResMut<TileRequestQueue>,
    mut tile_cache: ResMut<TileCache>,
    mut images: ResMut<Assets<Image>>,
    current_layer: Res<CurrentLayer>,
) {
    tile_cache.frame += 1;
    let frame = tile_cache.frame;
    let mut completed = 0;

    let mut i = 0;
    while i < request_queue.in_flight.len() && completed < POLL_BUDGET {
        if let Some(result) = block_on(poll_once(&mut request_queue.in_flight[i].task)) {
            let tier = request_queue.in_flight[i].tier;
            let _in_flight = request_queue.in_flight.swap_remove(i);
            let (coord, biome_map) = result;

            let image_data = biome_map.to_layer_image(current_layer.0);
            let image = create_image(TILE_MAP_SIZE, TILE_MAP_SIZE, image_data);
            let texture = images.add(image);

            let cached = CachedTile {
                biome_map,
                texture,
                last_used_frame: frame,
            };

            match tier {
                DetailTier::Macro => tile_cache.insert_macro(coord, cached),
                DetailTier::Meso => tile_cache.insert_meso(coord, cached),
            }

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
    view_level: Res<ViewLevel>,
    visible_range: Res<VisibleChunkRange>,
    mut tile_cache: ResMut<TileCache>,
    mut pool: ResMut<SpritePool>,
    world_def: Res<WorldDefinition>,
    mut sprite_query: Query<(&mut Transform, &mut Sprite, &mut Visibility)>,
) {
    let half_map_width = world_def.width as f32 / 2.0;
    let half_map_height = world_def.height as f32 / 2.0;
    let frame = tile_cache.frame;

    // --- Macro layer (always visible as base) ---
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

    // --- Meso layer ---
    if view_level.current != DetailTier::Meso {
        for i in 0..pool.meso_assigned.len() {
            if pool.meso_assigned[i].is_some() {
                pool.meso_assigned[i] = None;
                if let Ok((mut transform, _, mut vis)) = sprite_query.get_mut(pool.meso_sprites[i]) {
                    *vis = Visibility::Hidden;
                    transform.translation.x = -10000.0;
                }
            }
        }
        return;
    }

    let meso_per_chunk = (CHUNK_SIZE_I as f64 / MESO_WORLD_SIZE) as i32;

    let mut needed_meso: HashSet<(i32, i32)> = HashSet::new();
    for cy in visible_range.min_y..=visible_range.max_y {
        for cx in visible_range.min_x..=visible_range.max_x {
            for my in 0..meso_per_chunk {
                for mx in 0..meso_per_chunk {
                    needed_meso.insert((cx * meso_per_chunk + mx, cy * meso_per_chunk + my));
                }
            }
        }
    }

    for i in 0..pool.meso_assigned.len() {
        if let Some(coord) = pool.meso_assigned[i] {
            if !needed_meso.contains(&coord) || !tile_cache.meso_tiles.contains_key(&coord) {
                pool.meso_assigned[i] = None;
                let entity = pool.meso_sprites[i];
                if let Ok((mut transform, _, mut vis)) = sprite_query.get_mut(entity) {
                    *vis = Visibility::Hidden;
                    transform.translation.x = -10000.0;
                }
            }
        }
    }

    let assigned_meso: HashSet<(i32, i32)> = pool.meso_assigned.iter()
        .filter_map(|a| *a)
        .collect();

    for coord in &needed_meso {
        if assigned_meso.contains(coord) { continue; }
        let tex_handle = {
            let Some(cached) = tile_cache.meso_tiles.get(coord) else { continue };
            cached.texture.clone()
        };

        let Some(slot_idx) = pool.meso_assigned.iter().position(|a| a.is_none()) else { break };
        pool.meso_assigned[slot_idx] = Some(*coord);
        let entity = pool.meso_sprites[slot_idx];

        let world_x = coord.0 as f64 * MESO_WORLD_SIZE;
        let world_y = coord.1 as f64 * MESO_WORLD_SIZE;
        let sprite_x = world_x as f32 + MESO_WORLD_SIZE as f32 / 2.0 - half_map_width;
        let sprite_y = half_map_height - world_y as f32 - MESO_WORLD_SIZE as f32 / 2.0;

        if let Ok((mut transform, mut sprite, mut vis)) = sprite_query.get_mut(entity) {
            transform.translation = Vec3::new(sprite_x, sprite_y, 0.2);
            sprite.image = tex_handle;
            sprite.custom_size = Some(Vec2::new(MESO_WORLD_SIZE as f32, MESO_WORLD_SIZE as f32));
            *vis = Visibility::Inherited;
        }
    }

    for assignment in pool.meso_assigned.iter() {
        if let Some(coord) = assignment {
            if let Some(cached) = tile_cache.meso_tiles.get_mut(coord) {
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
) {
    // Sync current layer to UI state
    ui_state.current_layer = Some(current_layer.0);

    let Some(new_layer) = ui_state.layer_changed.take() else {
        return;
    };

    current_layer.0 = new_layer;

    // Re-render all cached tiles from their BiomeMap data
    if let Some(ref mut cache) = tile_cache {
        // Re-render macro tiles
        for (_, cached) in cache.macro_tiles.iter_mut() {
            let image_data = cached.biome_map.to_layer_image(new_layer);
            let new_image = create_image(TILE_MAP_SIZE, TILE_MAP_SIZE, image_data);
            cached.texture = images.add(new_image);
        }

        // Re-render meso tiles
        for (_, cached) in cache.meso_tiles.iter_mut() {
            let image_data = cached.biome_map.to_layer_image(new_layer);
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
            for (i, assignment) in pool.meso_assigned.iter().enumerate() {
                if let Some(coord) = assignment {
                    if let Some(cached) = cache.meso_tiles.get(coord) {
                        let entity = pool.meso_sprites[i];
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
) {
    egui::Window::new("Tile Info")
        .anchor(egui::Align2::RIGHT_TOP, [-8.0, 8.0])
        .resizable(false)
        .collapsible(false)
        .show(contexts.ctx_mut(), |ui| {
            if !info.active {
                ui.label("(no tile)");
                return;
            }
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
    tile_cache.meso_tiles.clear();
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
    for i in 0..pool.meso_assigned.len() {
        pool.meso_assigned[i] = None;
        let entity = pool.meso_sprites[i];
        if let Ok((mut transform, mut vis)) = sprite_query.get_mut(entity) {
            *vis = Visibility::Hidden;
            transform.translation.x = -10000.0;
        }
    }
    // Regenerate macro biome data
    let biome_map = Arc::new(BiomeMap::generate_with_backend(world_def.seed, world_def.width, world_def.height, backend));
    println!("  Macro biome data generated successfully");

    let debug_path = std::path::Path::new("debug_layers");
    biome_map.save_debug_layers(debug_path);

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
    });

    next_phase.set(AppPhase::GeneratingMacro);
    println!("World regenerated. Pre-generating {} macro tiles...", total);
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
        // Clamp to 0.15 min — meso is the deepest zoom on the world map.
        // Micro detail is only accessible via LevelLauncher.
        projection.scale = (projection.scale * zoom_factor).clamp(0.15, 10.0);
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
    view_level: Res<ViewLevel>,
    mut highlight_info: ResMut<HighlightInfo>,
) {
    let Ok((mut highlight_transform, mut highlight_sprite)) = highlight_query.get_single_mut() else { return };

    if contexts.ctx_mut().is_pointer_over_area() {
        highlight_transform.translation.x = -10000.0;
        highlight_info.active = false;
        return;
    }

    // Adapt highlight size to current tier
    let (chunk_size, tier_name) = match view_level.current {
        DetailTier::Macro => (CHUNK_SIZE, "Macro"),
        DetailTier::Meso => (MESO_WORLD_SIZE as f32, "Meso"),
    };
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
        for i in 0..pool.meso_assigned.len() {
            pool.meso_assigned[i] = None;
            let entity = pool.meso_sprites[i];
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
) {
    let Some(world_textures) = world_textures else { return };
    let Ok(player_transform) = player_query.get_single() else { return };

    let player_pos = player_transform.translation;
    let player_chunk_x = (player_pos.x / LEVEL_CHUNK_TILES).floor() as i32;
    let player_chunk_y = ((-player_pos.y) / LEVEL_CHUNK_TILES).floor() as i32; // Y-flip

    let seed = level.seed;
    let height = level.world_height;
    let backend = ui_state.backend();

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
            let task = AsyncComputeTaskPool::get().spawn(async move {
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

    println!("Selected chunk ({}, {}) — press F4 to play", chunk_x, chunk_y);
}

/// When LevelLauncher mode is entered with a SelectedChunk, auto-start micro generation.
fn auto_play_on_launcher_enter(
    mut commands: Commands,
    selected: Res<SelectedChunk>,
    world_def: Res<WorldDefinition>,
) {
    commands.insert_resource(PlayableLevel {
        origin: selected.origin,
        chunk_coord: selected.chunk_coord,
        seed: world_def.seed,
        world_height: world_def.height as f64,
    });
    commands.init_resource::<LoadedChunks>();

    println!("Entering play mode at chunk ({}, {})", selected.chunk_coord.0, selected.chunk_coord.1);
}

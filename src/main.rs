use bevy::image::{ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on, poll_once};
use bevy_egui::{egui, EguiContexts};
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use rb_core::{AppMode, ModeTransitionEvent, handle_mode_shortcuts};
use rb_editor::{CurrentLayer, GeneratorUiState, RegenerationRequest};
use rb_noise::{BiomeMap, NoiseBackend};
use rb_world::WorldDefinition;
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
        // GeneratingMeso phase - pre-generate all 128 meso tiles
        .add_systems(Update, (
            dispatch_meso_pregen,
            poll_meso_pregen,
            meso_pregen_progress_ui,
        ).run_if(in_state(AppPhase::GeneratingMeso)))
        // Ready phase - main game systems
        .add_systems(Update, (
            handle_mode_shortcuts,
            handle_layer_change.run_if(in_state(AppMode::WorldGenerator)),
            regenerate_world.run_if(in_state(AppMode::WorldGenerator)),
            camera_zoom,
            camera_pan,
            calculate_visible_chunks,
            update_view_level,
            enqueue_and_dispatch_tiles,
            poll_tile_results,
            manage_meso_tiles,
            update_cursor_world_pos,
            update_chunk_highlight,
            highlight_info_ui,
            log_mode_transition,
        ).run_if(in_state(AppPhase::Ready)))
        .run();
}

// ─── Constants ───────────────────────────────────────────────────────────────

/// Size of macro chunks in pixels (for highlighting grid).
const CHUNK_SIZE: f32 = 64.0;

/// Size of meso map in pixels (per tile).
const MESO_MAP_SIZE: usize = 512;

/// Number of pre-spawned meso pool sprites.
const MESO_POOL_SIZE: usize = 160;

/// Number of pre-spawned micro pool sprites.
const MICRO_POOL_SIZE: usize = 24;

/// Max cached meso tiles — sized to hold all 128 pre-generated tiles.
const MESO_CACHE_MAX: usize = 128;

/// Max cached micro tiles (LRU eviction beyond this).
const MICRO_CACHE_MAX: usize = 32;

/// Max concurrent async tile generation tasks (streaming).
const MAX_CONCURRENT_TILES: usize = 16;

/// Max concurrent async tile generation tasks during meso pre-generation.
const MESO_PREGEN_CONCURRENCY: usize = 12;

/// Max tile completions to process per frame.
const POLL_BUDGET: usize = 16;

/// Micro tile covers this many world units (8×8 area at 512×512 pixels).
const MICRO_WORLD_SIZE: f64 = 8.0;

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
            current: DetailTier::Meso,
            pending: None,
            frames_at_pending: 0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
enum DetailTier {
    Meso,
    Micro,
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
    meso_sprites: Vec<Entity>,
    meso_assigned: Vec<Option<(i32, i32)>>,
    micro_sprites: Vec<Entity>,
    micro_assigned: Vec<Option<(i32, i32)>>,
}

/// A cached tile with BiomeMap data and texture.
struct CachedTile {
    biome_map: Arc<BiomeMap>,
    texture: Handle<Image>,
    last_used_frame: u64,
}

/// LRU tile cache for meso and micro tiers.
#[derive(Resource)]
struct TileCache {
    meso: HashMap<(i32, i32), CachedTile>,
    meso_max: usize,
    micro: HashMap<(i32, i32), CachedTile>,
    micro_max: usize,
    frame: u64,
}

impl Default for TileCache {
    fn default() -> Self {
        Self {
            meso: HashMap::new(),
            meso_max: MESO_CACHE_MAX,
            micro: HashMap::new(),
            micro_max: MICRO_CACHE_MAX,
            frame: 0,
        }
    }
}

impl TileCache {
    fn insert_meso(&mut self, coord: (i32, i32), tile: CachedTile) {
        if self.meso.len() >= self.meso_max {
            // Evict LRU
            if let Some((&evict_coord, _)) = self.meso.iter()
                .min_by_key(|(_, t)| t.last_used_frame)
            {
                self.meso.remove(&evict_coord);
            }
        }
        self.meso.insert(coord, tile);
    }

    fn insert_micro(&mut self, coord: (i32, i32), tile: CachedTile) {
        if self.micro.len() >= self.micro_max {
            if let Some((&evict_coord, _)) = self.micro.iter()
                .min_by_key(|(_, t)| t.last_used_frame)
            {
                self.micro.remove(&evict_coord);
            }
        }
        self.micro.insert(coord, tile);
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

/// Application phase - config, generating, pre-generating meso, or ready.
#[derive(States, Default, Clone, Eq, PartialEq, Hash, Debug)]
enum AppPhase {
    #[default]
    Config,
    Generating,      // Generate macro map
    GeneratingMeso,  // Pre-generate all 128 meso tiles
    Ready,
}

/// Tracks progress of meso tile pre-generation.
#[derive(Resource)]
struct MesoPregenState {
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

    // Save debug layer images
    let debug_path = std::path::Path::new("debug_layers");
    biome_map.save_debug_layers(debug_path);

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

    // Spawn sprite pool (meso + micro)
    let mut meso_sprites = Vec::with_capacity(MESO_POOL_SIZE);
    let mut meso_assigned = Vec::with_capacity(MESO_POOL_SIZE);
    for _i in 0..MESO_POOL_SIZE {
        let entity = commands.spawn((
            Sprite { ..default() },
            Transform::from_xyz(-10000.0, -10000.0, 0.1),
            Visibility::Hidden,
            PoolSlot,
        )).id();
        meso_sprites.push(entity);
        meso_assigned.push(None);
    }

    let mut micro_sprites = Vec::with_capacity(MICRO_POOL_SIZE);
    let mut micro_assigned = Vec::with_capacity(MICRO_POOL_SIZE);
    for _i in 0..MICRO_POOL_SIZE {
        let entity = commands.spawn((
            Sprite { ..default() },
            Transform::from_xyz(-10000.0, -10000.0, 0.2),
            Visibility::Hidden,
            PoolSlot,
        )).id();
        micro_sprites.push(entity);
        micro_assigned.push(None);
    }

    commands.insert_resource(SpritePool {
        meso_sprites,
        meso_assigned,
        micro_sprites,
        micro_assigned,
    });
    commands.insert_resource(TileCache::default());
    commands.insert_resource(TileRequestQueue::default());
    commands.insert_resource(ViewLevel::default());

    // Queue all 128 meso chunks for pre-generation
    let chunks_x = (world_def.width as f32 / CHUNK_SIZE).ceil() as i32;
    let chunks_y = (world_def.height as f32 / CHUNK_SIZE).ceil() as i32;
    let mut remaining: Vec<(i32, i32)> = Vec::new();
    for cy in 0..chunks_y {
        for cx in 0..chunks_x {
            remaining.push((cx, cy));
        }
    }
    let total = remaining.len();
    commands.insert_resource(MesoPregenState {
        total,
        completed: 0,
        remaining,
        in_flight: Vec::new(),
    });

    next_phase.set(AppPhase::GeneratingMeso);
    println!("Macro map ready. Pre-generating {} meso tiles...", total);
}

// ─── Meso Pre-generation ─────────────────────────────────────────────────────

/// Dispatch async tasks for meso pre-generation (higher concurrency than streaming).
fn dispatch_meso_pregen(
    mut pregen: ResMut<MesoPregenState>,
    world_textures: Option<Res<MacroBiomeData>>,
    world_def: Res<WorldDefinition>,
    ui_state: Res<GeneratorUiState>,
) {
    let Some(world_textures) = world_textures else { return };
    let seed = world_def.seed;
    let height = world_def.height as f64;
    let backend = ui_state.backend();

    while pregen.in_flight.len() < MESO_PREGEN_CONCURRENCY {
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
                MESO_MAP_SIZE,
                height,
                1, // detail_level = meso
                None,
                backend,
                Some(&macro_map),
            );
            (coord, Arc::new(biome_map))
        });

        pregen.in_flight.push(InFlightTile { coord, tier: DetailTier::Meso, task });
    }
}

/// Poll all completions during pre-generation (no per-frame budget on loading screen).
fn poll_meso_pregen(
    mut pregen: ResMut<MesoPregenState>,
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
            let image = create_image(MESO_MAP_SIZE, MESO_MAP_SIZE, image_data);
            let texture = images.add(image);

            tile_cache.insert_meso(coord, CachedTile {
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
        println!("All {} meso tiles pre-generated.", pregen.total);
        commands.remove_resource::<MesoPregenState>();
        next_phase.set(AppPhase::Ready);
    }
}

/// Show progress bar during meso pre-generation.
fn meso_pregen_progress_ui(
    mut contexts: EguiContexts,
    pregen: Res<MesoPregenState>,
) {
    let ctx = contexts.ctx_mut();

    egui::CentralPanel::default()
        .frame(egui::Frame::default().fill(egui::Color32::from_rgb(30, 30, 30)))
        .show(ctx, |_| {});

    egui::Window::new("Pre-generating Meso Maps")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .fixed_size([350.0, 100.0])
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                let progress = pregen.completed as f32 / pregen.total as f32;
                ui.label(format!("Generating meso maps: {}/{}", pregen.completed, pregen.total));
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
        DetailTier::Meso => {
            if scale < 0.08 {
                Some(DetailTier::Micro)
            } else {
                None
            }
        }
        DetailTier::Micro => {
            if scale > 0.12 { Some(DetailTier::Meso) } else { None }
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

    // Collect needed meso tiles
    let mut needed: Vec<((i32, i32), DetailTier, f32)> = Vec::new();

    {
        for cy in visible_range.min_y..=visible_range.max_y {
            for cx in visible_range.min_x..=visible_range.max_x {
                let coord = (cx, cy);
                if tile_cache.meso.contains_key(&coord) || in_flight_coords.contains(&(coord, DetailTier::Meso)) {
                    continue;
                }
                // Distance from camera center for priority
                let sprite_x = cx as f32 * CHUNK_SIZE + CHUNK_SIZE / 2.0 - half_map_width;
                let sprite_y = half_map_height - cy as f32 * CHUNK_SIZE - CHUNK_SIZE / 2.0;
                let dist = (camera_pos.x - sprite_x).powi(2) + (camera_pos.y - sprite_y).powi(2);
                needed.push((coord, DetailTier::Meso, dist));
            }
        }
    }

    // Collect needed micro tiles when at micro level
    if view_level.current == DetailTier::Micro {
        // Micro tiles subdivide each visible meso chunk into sub-tiles
        let micro_per_chunk = (CHUNK_SIZE_I as f64 / MICRO_WORLD_SIZE) as i32; // 8 micro tiles per meso chunk edge
        for cy in visible_range.min_y..=visible_range.max_y {
            for cx in visible_range.min_x..=visible_range.max_x {
                for my in 0..micro_per_chunk {
                    for mx in 0..micro_per_chunk {
                        let micro_coord = (cx * micro_per_chunk + mx, cy * micro_per_chunk + my);
                        if tile_cache.micro.contains_key(&micro_coord) || in_flight_coords.contains(&(micro_coord, DetailTier::Micro)) {
                            continue;
                        }
                        let world_x = micro_coord.0 as f64 * MICRO_WORLD_SIZE;
                        let world_y = micro_coord.1 as f64 * MICRO_WORLD_SIZE;
                        let sprite_x = world_x as f32 + MICRO_WORLD_SIZE as f32 / 2.0 - half_map_width;
                        let sprite_y = half_map_height - world_y as f32 - MICRO_WORLD_SIZE as f32 / 2.0;
                        let dist = (camera_pos.x - sprite_x).powi(2) + (camera_pos.y - sprite_y).powi(2);
                        needed.push((micro_coord, DetailTier::Micro, dist));
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
            DetailTier::Meso => {
                let wx = coord.0 as f64 * CHUNK_SIZE as f64;
                let wy = coord.1 as f64 * CHUNK_SIZE as f64;
                (wx, wy, CHUNK_SIZE as f64, 1u32)
            }
            DetailTier::Micro => {
                let wx = coord.0 as f64 * MICRO_WORLD_SIZE;
                let wy = coord.1 as f64 * MICRO_WORLD_SIZE;
                (wx, wy, MICRO_WORLD_SIZE, 2u32)
            }
        };

        let task = AsyncComputeTaskPool::get().spawn(async move {
            let biome_map = BiomeMap::generate_meso_full_with_backend(
                seed,
                world_x,
                world_y,
                world_size,
                MESO_MAP_SIZE,
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
            let image = create_image(MESO_MAP_SIZE, MESO_MAP_SIZE, image_data);
            let texture = images.add(image);

            let cached = CachedTile {
                biome_map,
                texture,
                last_used_frame: frame,
            };

            match tier {
                DetailTier::Meso => tile_cache.insert_meso(coord, cached),
                DetailTier::Micro => tile_cache.insert_micro(coord, cached),
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
fn manage_meso_tiles(
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

    // --- Meso layer ---
    // Collect needed meso tiles
    let mut needed_meso: HashSet<(i32, i32)> = HashSet::new();
    for cy in visible_range.min_y..=visible_range.max_y {
        for cx in visible_range.min_x..=visible_range.max_x {
            needed_meso.insert((cx, cy));
        }
    }

    // Free slots assigned to tiles no longer visible
    for i in 0..pool.meso_assigned.len() {
        if let Some(coord) = pool.meso_assigned[i] {
            if !needed_meso.contains(&coord) || !tile_cache.meso.contains_key(&coord) {
                pool.meso_assigned[i] = None;
                let entity = pool.meso_sprites[i];
                if let Ok((mut transform, _, mut vis)) = sprite_query.get_mut(entity) {
                    *vis = Visibility::Hidden;
                    transform.translation.x = -10000.0;
                    transform.translation.y = -10000.0;
                }
            }
        }
    }

    // Collect already-assigned coords
    let assigned_meso: HashSet<(i32, i32)> = pool.meso_assigned.iter()
        .filter_map(|a| *a)
        .collect();

    // Assign free slots to newly visible cached tiles
    for coord in &needed_meso {
        if assigned_meso.contains(coord) { continue; }
        // Get texture handle from cache (immutable borrow scope)
        let (tex_handle, custom_size) = {
            let Some(cached) = tile_cache.meso.get(coord) else { continue };
            (cached.texture.clone(), Vec2::splat(CHUNK_SIZE))
        };

        let Some(slot_idx) = pool.meso_assigned.iter().position(|a| a.is_none()) else { break };
        pool.meso_assigned[slot_idx] = Some(*coord);
        let entity = pool.meso_sprites[slot_idx];

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

    // Touch used meso tiles for LRU
    for assignment in pool.meso_assigned.iter() {
        if let Some(coord) = assignment {
            if let Some(cached) = tile_cache.meso.get_mut(coord) {
                cached.last_used_frame = frame;
            }
        }
    }

    // --- Micro layer ---
    if view_level.current != DetailTier::Micro {
        hide_micro_pool(&mut pool, &mut sprite_query);
        return;
    }

    let micro_per_chunk = (CHUNK_SIZE_I as f64 / MICRO_WORLD_SIZE) as i32;

    let mut needed_micro: HashSet<(i32, i32)> = HashSet::new();
    for cy in visible_range.min_y..=visible_range.max_y {
        for cx in visible_range.min_x..=visible_range.max_x {
            for my in 0..micro_per_chunk {
                for mx in 0..micro_per_chunk {
                    needed_micro.insert((cx * micro_per_chunk + mx, cy * micro_per_chunk + my));
                }
            }
        }
    }

    // Free slots no longer needed
    for i in 0..pool.micro_assigned.len() {
        if let Some(coord) = pool.micro_assigned[i] {
            if !needed_micro.contains(&coord) || !tile_cache.micro.contains_key(&coord) {
                pool.micro_assigned[i] = None;
                let entity = pool.micro_sprites[i];
                if let Ok((mut transform, _, mut vis)) = sprite_query.get_mut(entity) {
                    *vis = Visibility::Hidden;
                    transform.translation.x = -10000.0;
                }
            }
        }
    }

    let assigned_micro: HashSet<(i32, i32)> = pool.micro_assigned.iter()
        .filter_map(|a| *a)
        .collect();

    for coord in &needed_micro {
        if assigned_micro.contains(coord) { continue; }
        let tex_handle = {
            let Some(cached) = tile_cache.micro.get(coord) else { continue };
            cached.texture.clone()
        };

        let Some(slot_idx) = pool.micro_assigned.iter().position(|a| a.is_none()) else { break };
        pool.micro_assigned[slot_idx] = Some(*coord);
        let entity = pool.micro_sprites[slot_idx];

        let world_x = coord.0 as f64 * MICRO_WORLD_SIZE;
        let world_y = coord.1 as f64 * MICRO_WORLD_SIZE;
        let sprite_x = world_x as f32 + MICRO_WORLD_SIZE as f32 / 2.0 - half_map_width;
        let sprite_y = half_map_height - world_y as f32 - MICRO_WORLD_SIZE as f32 / 2.0;

        if let Ok((mut transform, mut sprite, mut vis)) = sprite_query.get_mut(entity) {
            transform.translation = Vec3::new(sprite_x, sprite_y, 0.2);
            sprite.image = tex_handle;
            sprite.custom_size = Some(Vec2::new(MICRO_WORLD_SIZE as f32, MICRO_WORLD_SIZE as f32));
            *vis = Visibility::Inherited;
        }
    }

    // Touch used micro tiles for LRU
    for assignment in pool.micro_assigned.iter() {
        if let Some(coord) = assignment {
            if let Some(cached) = tile_cache.micro.get_mut(coord) {
                cached.last_used_frame = frame;
            }
        }
    }
}

/// Hide all micro pool sprites and clear assignments.
fn hide_micro_pool(
    pool: &mut SpritePool,
    sprite_query: &mut Query<(&mut Transform, &mut Sprite, &mut Visibility)>,
) {
    for i in 0..pool.micro_assigned.len() {
        if pool.micro_assigned[i].is_some() {
            pool.micro_assigned[i] = None;
            if let Ok((mut transform, _, mut vis)) = sprite_query.get_mut(pool.micro_sprites[i]) {
                *vis = Visibility::Hidden;
                transform.translation.x = -10000.0;
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

    // Re-render all cached meso/micro tiles from their BiomeMap data
    if let Some(ref mut cache) = tile_cache {
        // Re-render meso tiles
        for (_, cached) in cache.meso.iter_mut() {
            let image_data = cached.biome_map.to_layer_image(new_layer);
            let new_image = create_image(MESO_MAP_SIZE, MESO_MAP_SIZE, image_data);
            cached.texture = images.add(new_image);
        }

        // Re-render micro tiles
        for (_, cached) in cache.micro.iter_mut() {
            let image_data = cached.biome_map.to_layer_image(new_layer);
            let new_image = create_image(MESO_MAP_SIZE, MESO_MAP_SIZE, image_data);
            cached.texture = images.add(new_image);
        }

        // Update assigned pool sprite images
        if let Some(ref pool) = pool {
            for (i, assignment) in pool.meso_assigned.iter().enumerate() {
                if let Some(coord) = assignment {
                    if let Some(cached) = cache.meso.get(coord) {
                        let entity = pool.meso_sprites[i];
                        if let Ok((_, mut sprite)) = sprite_query.get_mut(entity) {
                            sprite.image = cached.texture.clone();
                        }
                    }
                }
            }
            for (i, assignment) in pool.micro_assigned.iter().enumerate() {
                if let Some(coord) = assignment {
                    if let Some(cached) = cache.micro.get(coord) {
                        let entity = pool.micro_sprites[i];
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
    tile_cache.meso.clear();
    tile_cache.micro.clear();
    request_queue.in_flight.clear();

    // Hide all pool sprites
    for i in 0..pool.meso_assigned.len() {
        pool.meso_assigned[i] = None;
        let entity = pool.meso_sprites[i];
        if let Ok((mut transform, mut vis)) = sprite_query.get_mut(entity) {
            *vis = Visibility::Hidden;
            transform.translation.x = -10000.0;
        }
    }
    for i in 0..pool.micro_assigned.len() {
        pool.micro_assigned[i] = None;
        let entity = pool.micro_sprites[i];
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

    // Queue all meso chunks for pre-generation
    let chunks_x = (world_def.width as f32 / CHUNK_SIZE).ceil() as i32;
    let chunks_y = (world_def.height as f32 / CHUNK_SIZE).ceil() as i32;
    let mut remaining: Vec<(i32, i32)> = Vec::new();
    for cy in 0..chunks_y {
        for cx in 0..chunks_x {
            remaining.push((cx, cy));
        }
    }
    let total = remaining.len();
    commands.insert_resource(MesoPregenState {
        total,
        completed: 0,
        remaining,
        in_flight: Vec::new(),
    });

    next_phase.set(AppPhase::GeneratingMeso);
    println!("World regenerated. Pre-generating {} meso tiles...", total);
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
        projection.scale = (projection.scale * zoom_factor).clamp(0.005, 10.0);
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
        DetailTier::Meso => (CHUNK_SIZE, "Meso"),
        DetailTier::Micro => (MICRO_WORLD_SIZE as f32, "Micro"),
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

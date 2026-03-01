use bevy::image::{ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on, poll_once};
use bevy_egui::{egui, EguiContexts};
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use rayon::prelude::*;
use rb_core::{AppMode, ModeTransitionEvent, handle_mode_shortcuts};
use rb_editor::{CurrentLayer, GeneratorUiState, RegenerationRequest};
use rb_noise::{BiomeMap, LayerId, LayerProgress, NoiseLayer};
use rb_world::{CivilizationConfig, CivilizationGenerator, CivilizationResult, WorldDefinition};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const MAP_WIDTH: usize = 1024;
const MAP_HEIGHT: usize = 512;
const CHUNK_SIZE_I: usize = 64;
const CHUNKS_X: usize = MAP_WIDTH / CHUNK_SIZE_I;   // 16
const CHUNKS_Y: usize = MAP_HEIGHT / CHUNK_SIZE_I;  // 8
const TOTAL_CHUNKS: usize = CHUNKS_X * CHUNKS_Y;    // 128

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
        .init_resource::<ViewLevel>()
        .init_resource::<LoadedMesoTiles>()
        .init_resource::<VisibleChunkRange>()
        .init_resource::<MesoTileCache>()
        .init_resource::<GenerationTask>()
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
        // Generating phase - poll task, show progress
        .add_systems(Update, (
            start_generation.run_if(resource_added::<GenerationStarted>),
            poll_generation,
            generation_progress_ui,
        ).run_if(in_state(AppPhase::Generating)))
        // Ready phase - main game systems
        .add_systems(Update, (
            handle_mode_shortcuts,
            handle_layer_change.run_if(in_state(AppMode::WorldGenerator)),
            regenerate_world.run_if(in_state(AppMode::WorldGenerator)),
            camera_zoom,
            camera_pan,
            calculate_visible_chunks,
            handle_view_level_transition,
            manage_meso_tiles,
            update_cursor_world_pos,
            update_chunk_highlight,
            log_mode_transition,
        ).run_if(in_state(AppPhase::Ready)))
        .run();
}

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

/// Stores the BiomeMap and current texture handle.
#[derive(Resource)]
struct WorldMapTextures {
    /// The generated biome map with all noise layers
    biome_map: Arc<BiomeMap>,
    /// Current layer texture handle
    current_handle: Handle<Image>,
    /// Territory overlay from civilization generation
    territory_overlay: Option<Vec<u8>>,
}

/// Marker component for the world map sprite.
#[derive(Component)]
struct WorldMapSprite;

/// Marker component for the chunk highlight overlay.
#[derive(Component)]
struct ChunkHighlight;

/// Resource tracking cursor position in world space.
#[derive(Resource, Default)]
struct CursorWorldPos(Vec2);

/// Current detail level being displayed.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
enum ViewLevel {
    #[default]
    Macro,
    Meso,
}

/// Marker component for individual meso tile sprites.
#[derive(Component)]
#[allow(dead_code)]
struct MesoTile {
    chunk_x: i32,
    chunk_y: i32,
}

/// Tracks spawned meso tile sprite entities.
#[derive(Resource, Default)]
struct LoadedMesoTiles {
    /// Map from (chunk_x, chunk_y) to spawned sprite entity
    tiles: HashMap<(i32, i32), Entity>,
}

/// Camera viewport in chunk coordinates.
#[derive(Resource, Default)]
struct VisibleChunkRange {
    min_x: i32,
    max_x: i32,
    min_y: i32,
    max_y: i32,
}

/// Cache of pre-generated meso tiles with full BiomeMap data.
/// Stores both the full noise data (for layer switching) and pre-rendered textures.
#[derive(Resource, Default)]
struct MesoTileCache {
    /// Full BiomeMap for each tile - enables instant layer switching
    maps: HashMap<(i32, i32), Arc<BiomeMap>>,
    /// Pre-rendered texture handles for current layer view
    textures: HashMap<(i32, i32), Handle<Image>>,
}

/// Application phase - config, generating, or ready.
#[derive(States, Default, Clone, Eq, PartialEq, Hash, Debug)]
enum AppPhase {
    #[default]
    Config,      // Show config UI, no map yet
    Generating,  // Generating world in background
    Ready,       // Map ready, can interact
}

/// Background generation task and progress tracking.
#[derive(Resource, Default)]
struct GenerationTask {
    /// The async task generating full BiomeMap tiles
    task: Option<Task<Vec<((i32, i32), Arc<BiomeMap>)>>>,
    /// Per-layer progress tracking (7 progress bars)
    layer_progress: Option<Arc<LayerProgress>>,
    /// Tile completion counter
    tile_progress: Option<Arc<AtomicUsize>>,
    /// Generated macro biome map with all layers
    biome_map: Option<Arc<BiomeMap>>,
    /// Civilization generation result
    civ_result: Option<CivilizationResult>,
    /// Territory overlay image data
    territory_image: Option<Vec<u8>>,
}

/// Size of macro chunks in pixels (for highlighting grid).
const CHUNK_SIZE: f32 = 64.0;

/// Zoom threshold for switching to meso view.
const MESO_ZOOM_THRESHOLD: f32 = 0.5;

/// Size of meso map in pixels (per tile).
const MESO_MAP_SIZE: usize = 512;

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

/// Start background generation task.
fn start_generation(
    mut commands: Commands,
    mut task_res: ResMut<GenerationTask>,
    mut world_def: ResMut<WorldDefinition>,
) {
    commands.remove_resource::<GenerationStarted>();

    let seed = world_def.seed;
    let width = world_def.width;
    let height = world_def.height;

    // First generate macro map synchronously (fast)
    println!("Generating macro map {}x{}...", width, height);
    let biome_map = Arc::new(BiomeMap::generate(seed, width, height));
    task_res.biome_map = Some(biome_map.clone());
    println!("  Resources: {} cells with deposits", biome_map.resources.cells_with_resources());

    // Generate civilization
    println!("Generating civilization...");
    let civ_config = CivilizationConfig {
        max_settlements: 40,
        generate_roads: true,
        generate_trade_routes: true,
        generate_territories: true,
        territory_threshold: 0.1,
    };
    let civ_generator = CivilizationGenerator::new(seed, civ_config);
    let civ_result = civ_generator.generate(&biome_map, &mut world_def);
    println!(
        "Civilization: {} settlements, {} factions, {} roads",
        civ_result.settlements_placed,
        civ_result.factions_created,
        civ_result.roads_built
    );

    // Generate territory overlay image
    let territory_image = if let Some(ref territory) = world_def.territory_cache {
        let faction_colors: Vec<_> = world_def.factions.iter()
            .map(|f| (f.id, f.color))
            .collect();
        Some(territory.to_image(&faction_colors))
    } else {
        None
    };
    task_res.territory_image = territory_image;
    task_res.civ_result = Some(civ_result);

    // Per-layer progress tracking for all meso tiles
    let total_pixels_per_tile = MESO_MAP_SIZE * MESO_MAP_SIZE;
    let total_pixels = total_pixels_per_tile * TOTAL_CHUNKS;
    let layer_progress = Arc::new(LayerProgress::new(total_pixels));
    let layer_progress_clone = layer_progress.clone();

    // Tile completion counter
    let tile_progress = Arc::new(AtomicUsize::new(0));
    let tile_progress_clone = tile_progress.clone();

    // Spawn async task for meso tiles with full 7-layer generation
    println!("Generating {} meso tiles with 7-layer parallel generation...", TOTAL_CHUNKS);
    let task = AsyncComputeTaskPool::get().spawn(async move {
        (0..TOTAL_CHUNKS).into_par_iter().map(|chunk_idx| {
            let cx = (chunk_idx % CHUNKS_X) as i32;
            let cy = (chunk_idx / CHUNKS_X) as i32;

            let world_x = cx as f64 * CHUNK_SIZE as f64;
            let world_y = cy as f64 * CHUNK_SIZE as f64;

            // Generate full BiomeMap with all 7 layers + derived
            let meso_map = BiomeMap::generate_meso_full(
                seed,
                world_x,
                world_y,
                CHUNK_SIZE as f64,
                MESO_MAP_SIZE,
                height as f64,
                1, // detail_level = meso
                &layer_progress_clone,
            );

            tile_progress_clone.fetch_add(1, Ordering::Relaxed);
            ((cx, cy), Arc::new(meso_map))
        }).collect()
    });

    task_res.task = Some(task);
    task_res.layer_progress = Some(layer_progress);
    task_res.tile_progress = Some(tile_progress);
}

/// Marker for settlement sprites.
#[derive(Component)]
struct SettlementMarker {
    city_id: u32,
}

/// Marker for road line sprites.
#[derive(Component)]
struct RoadMarker {
    road_id: u32,
}

/// Poll generation task and transition when complete.
fn poll_generation(
    mut commands: Commands,
    mut task_res: ResMut<GenerationTask>,
    mut images: ResMut<Assets<Image>>,
    mut cache: ResMut<MesoTileCache>,
    mut next_phase: ResMut<NextState<AppPhase>>,
    world_def: Res<WorldDefinition>,
    current_layer: Res<CurrentLayer>,
) {
    let Some(ref mut task) = task_res.task else { return };

    if let Some(result) = block_on(poll_once(task)) {
        // Meso tiles complete - store BiomeMap and create textures
        for ((cx, cy), meso_map) in result {
            // Generate texture for current layer view
            let image_data = meso_map.to_layer_image(current_layer.0);
            let meso_image = create_image(MESO_MAP_SIZE, MESO_MAP_SIZE, image_data);
            let handle = images.add(meso_image);

            // Store both the full BiomeMap and the texture
            cache.maps.insert((cx, cy), meso_map);
            cache.textures.insert((cx, cy), handle);
        }

        // Create macro map textures and sprites
        if let Some(biome_map) = task_res.biome_map.take() {
            // Create initial texture from biome layer
            let biome_data = biome_map.to_biome_image();
            let biome_image = create_image(world_def.width, world_def.height, biome_data);
            let biome_handle = images.add(biome_image);

            // Store territory overlay for Political layer
            let territory_overlay = task_res.territory_image.take();

            commands.insert_resource(WorldMapTextures {
                biome_map,
                current_handle: biome_handle.clone(),
                territory_overlay,
            });

            commands.spawn((
                Sprite { image: biome_handle, ..default() },
                WorldMapSprite,
            ));

            commands.spawn((
                Sprite {
                    color: Color::srgba(1.0, 1.0, 0.8, 0.3),
                    custom_size: Some(Vec2::splat(CHUNK_SIZE)),
                    ..default()
                },
                Transform::from_xyz(-10000.0, -10000.0, 0.5),
                ChunkHighlight,
            ));

            // Spawn settlement markers
            let half_width = world_def.width as f32 / 2.0;
            let half_height = world_def.height as f32 / 2.0;

            for city in &world_def.cities {
                let world_x = city.position.x as f32 - half_width;
                let world_y = half_height - city.position.y as f32;

                let (size, color) = match city.tier {
                    rb_world::CityTier::Capital => (12.0, Color::srgb(1.0, 0.8, 0.2)), // Gold
                    rb_world::CityTier::Town => (8.0, Color::srgb(0.8, 0.8, 0.8)),     // Silver
                    rb_world::CityTier::Village => (5.0, Color::srgb(0.6, 0.5, 0.4)),  // Brown
                };

                commands.spawn((
                    Sprite {
                        color,
                        custom_size: Some(Vec2::splat(size)),
                        ..default()
                    },
                    Transform::from_xyz(world_x, world_y, 0.3),
                    SettlementMarker { city_id: city.id },
                ));
            }

            // Spawn road markers (simplified - just spawn dots along waypoints)
            for road in &world_def.roads {
                let color = match road.road_type {
                    rb_world::RoadType::Imperial => Color::srgba(0.9, 0.7, 0.2, 0.8),
                    rb_world::RoadType::Provincial => Color::srgba(0.7, 0.7, 0.7, 0.6),
                    rb_world::RoadType::Trail => Color::srgba(0.5, 0.4, 0.3, 0.4),
                };
                let width = match road.road_type {
                    rb_world::RoadType::Imperial => 3.0,
                    rb_world::RoadType::Provincial => 2.0,
                    rb_world::RoadType::Trail => 1.5,
                };

                // Draw dots along the road path
                for waypoint in &road.waypoints {
                    let world_x = waypoint.x as f32 - half_width;
                    let world_y = half_height - waypoint.y as f32;

                    commands.spawn((
                        Sprite {
                            color,
                            custom_size: Some(Vec2::splat(width)),
                            ..default()
                        },
                        Transform::from_xyz(world_x, world_y, 0.2),
                        RoadMarker { road_id: road.id },
                    ));
                }
            }
        }

        // Clean up and transition
        task_res.task = None;
        task_res.layer_progress = None;
        task_res.tile_progress = None;
        task_res.civ_result = None;
        next_phase.set(AppPhase::Ready);
        println!("World ready! {} meso tiles cached ({} BiomeMaps).", cache.textures.len(), cache.maps.len());
    }
}

/// Show progress during generation with 7 per-layer progress bars.
fn generation_progress_ui(
    mut contexts: EguiContexts,
    task_res: Res<GenerationTask>,
) {
    let ctx = contexts.ctx_mut();

    egui::CentralPanel::default()
        .frame(egui::Frame::default().fill(egui::Color32::from_rgb(30, 30, 30)))
        .show(ctx, |_| {});

    let tile_progress = task_res.tile_progress.as_ref()
        .map(|p| p.load(Ordering::Relaxed))
        .unwrap_or(0);

    egui::Window::new("Generating")
        .collapsible(false)
        .resizable(false)
        .title_bar(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .fixed_size([350.0, 280.0])
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.heading("Generating World...");
                ui.add_space(10.0);

                // Overall tile progress
                let tile_fraction = tile_progress as f32 / TOTAL_CHUNKS as f32;
                ui.label("Tiles:");
                ui.add_sized(
                    [320.0, 18.0],
                    egui::ProgressBar::new(tile_fraction)
                        .text(format!("{}/{}", tile_progress, TOTAL_CHUNKS))
                );
                ui.add_space(10.0);

                // Per-layer progress bars
                if let Some(ref layer_progress) = task_res.layer_progress {
                    ui.separator();
                    ui.label("Layer Generation:");
                    ui.add_space(5.0);

                    for layer_id in LayerId::all() {
                        let fraction = layer_progress.fraction(*layer_id);

                        ui.horizontal(|ui| {
                            ui.label(format!("{:14}", layer_id.name()));
                            ui.add_sized(
                                [200.0, 14.0],
                                egui::ProgressBar::new(fraction)
                                    .text(format!("{:.0}%", fraction * 100.0))
                            );
                        });
                    }
                }
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

    // Use nearest-neighbor filtering for crisp pixels when zoomed
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        mag_filter: ImageFilterMode::Nearest,
        min_filter: ImageFilterMode::Nearest,
        ..default()
    });

    image
}

/// System to handle layer changes from the UI and sync CurrentLayer with GeneratorUiState.
fn handle_layer_change(
    mut ui_state: ResMut<GeneratorUiState>,
    mut current_layer: ResMut<CurrentLayer>,
    mut textures: Option<ResMut<WorldMapTextures>>,
    mut images: ResMut<Assets<Image>>,
    mut query: Query<&mut Sprite, With<WorldMapSprite>>,
    world_def: Res<WorldDefinition>,
    mut meso_cache: Option<ResMut<MesoTileCache>>,
    mut meso_sprites: Query<(&MesoTile, &mut Sprite), Without<WorldMapSprite>>,
    mut settlement_query: Query<&mut Visibility, With<SettlementMarker>>,
    mut road_query: Query<&mut Visibility, (With<RoadMarker>, Without<SettlementMarker>)>,
) {
    // Sync current layer to UI state so the dropdown shows the correct value
    ui_state.current_layer = Some(current_layer.0);

    // Check if a layer change was requested
    let Some(new_layer) = ui_state.layer_changed.take() else {
        return;
    };

    // Update current layer
    current_layer.0 = new_layer;

    // Only show settlements/roads on the Biome (game) layer
    let show_overlays = new_layer == NoiseLayer::Aggregate;
    let overlay_visibility = if show_overlays { Visibility::Inherited } else { Visibility::Hidden };

    for mut vis in settlement_query.iter_mut() {
        *vis = overlay_visibility;
    }
    for mut vis in road_query.iter_mut() {
        *vis = overlay_visibility;
    }

    // Update macro map texture
    if let Some(ref mut tex) = textures {
        let image_data = if new_layer == NoiseLayer::Political {
            tex.territory_overlay.clone().unwrap_or_else(|| tex.biome_map.to_layer_image(new_layer))
        } else {
            tex.biome_map.to_layer_image(new_layer)
        };

        let new_image = create_image(world_def.width, world_def.height, image_data);
        let new_handle = images.add(new_image);
        tex.current_handle = new_handle.clone();

        for mut sprite in query.iter_mut() {
            sprite.image = new_handle.clone();
        }
    }

    // Update meso tile textures from cached BiomeMap data
    if let Some(ref mut cache) = meso_cache {
        let new_textures: Vec<_> = cache.maps.iter()
            .map(|(coord, biome_map)| {
                let image_data = biome_map.to_layer_image(new_layer);
                let new_image = create_image(MESO_MAP_SIZE, MESO_MAP_SIZE, image_data);
                let new_handle = images.add(new_image);
                (*coord, new_handle)
            })
            .collect();

        for (coord, handle) in new_textures {
            cache.textures.insert(coord, handle);
        }

        for (meso_tile, mut sprite) in meso_sprites.iter_mut() {
            let coord = (meso_tile.chunk_x, meso_tile.chunk_y);
            if let Some(handle) = cache.textures.get(&coord) {
                sprite.image = handle.clone();
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

fn regenerate_world(
    mut regen_request: ResMut<RegenerationRequest>,
    world_def: Res<WorldDefinition>,
    mut images: ResMut<Assets<Image>>,
    mut textures: ResMut<WorldMapTextures>,
    mut query: Query<&mut Sprite, With<WorldMapSprite>>,
    current_layer: Res<CurrentLayer>,
) {
    if !regen_request.pending {
        return;
    }
    regen_request.pending = false;

    println!("Regenerating world map with seed {}...", world_def.seed);

    // Generate new biome map with all layers
    let biome_map = Arc::new(BiomeMap::generate(world_def.seed, world_def.width, world_def.height));
    println!("  Resources: {} cells with deposits", biome_map.resources.cells_with_resources());

    // Generate image for current layer
    let image_data = biome_map.to_layer_image(current_layer.0);
    let new_image = create_image(world_def.width, world_def.height, image_data);
    let new_handle = images.add(new_image);

    // Update textures resource
    textures.biome_map = biome_map;
    textures.current_handle = new_handle.clone();
    textures.territory_overlay = None; // Clear territory overlay (would need to regenerate civilization)

    // Update sprite
    for mut sprite in &mut query {
        sprite.image = new_handle.clone();
    }

    println!("World regenerated.");
}

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
        // Zoom in (scroll up) decreases scale, zoom out (scroll down) increases scale
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

    // Keyboard panning (arrow keys)
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

    // Left click drag panning (when not over UI)
    // Invert Y axis for natural "grab and drag" feel
    let over_ui = contexts.ctx_mut().is_pointer_over_area();
    if mouse.pressed(MouseButton::Left) && !over_ui {
        for event in motion_events.read() {
            pan_delta.x -= event.delta.x;
            pan_delta.y += event.delta.y; // Inverted Y
        }
    } else {
        // Clear motion events if not panning
        motion_events.clear();
    }

    if pan_delta == Vec2::ZERO {
        return;
    }

    for (mut transform, projection) in &mut query {
        // Scale pan speed by current zoom level
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
    view_level: Res<ViewLevel>,
    mut highlight_query: Query<(&mut Transform, &mut Sprite), With<ChunkHighlight>>,
    mut contexts: EguiContexts,
) {
    let Ok((mut highlight_transform, mut highlight_sprite)) = highlight_query.get_single_mut() else { return };

    // Hide highlight if cursor is over UI
    if contexts.ctx_mut().is_pointer_over_area() {
        highlight_transform.translation.x = -10000.0;
        return;
    }

    // Adjust highlight size based on view level
    let chunk_size = match *view_level {
        ViewLevel::Macro => CHUNK_SIZE,
        ViewLevel::Meso => CHUNK_SIZE / 8.0, // Smaller grid at meso level
    };
    highlight_sprite.custom_size = Some(Vec2::splat(chunk_size));

    // Convert world position to map coordinates
    let half_width = world_def.width as f32 / 2.0;
    let half_height = world_def.height as f32 / 2.0;

    let map_x = cursor_pos.0.x + half_width;
    let map_y = half_height - cursor_pos.0.y; // Flip Y

    // Check if cursor is within map bounds
    if map_x < 0.0 || map_x >= world_def.width as f32 || map_y < 0.0 || map_y >= world_def.height as f32 {
        // Hide highlight when outside map
        highlight_transform.translation.x = -10000.0;
        return;
    }

    // Snap to chunk grid
    let chunk_x = (map_x / chunk_size).floor() * chunk_size;
    let chunk_y = (map_y / chunk_size).floor() * chunk_size;

    // Convert back to world coordinates (center of chunk)
    let world_x = chunk_x + chunk_size / 2.0 - half_width;
    let world_y = half_height - chunk_y - chunk_size / 2.0;

    highlight_transform.translation.x = world_x;
    highlight_transform.translation.y = world_y;
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

    // Calculate visible world-space bounds
    let half_viewport_width = (window.width() / 2.0) * scale;
    let half_viewport_height = (window.height() / 2.0) * scale;

    let world_min_x = camera_pos.x - half_viewport_width;
    let world_max_x = camera_pos.x + half_viewport_width;
    let world_min_y = camera_pos.y - half_viewport_height;
    let world_max_y = camera_pos.y + half_viewport_height;

    // Convert world coords to map coords
    let half_map_width = world_def.width as f32 / 2.0;
    let half_map_height = world_def.height as f32 / 2.0;

    let map_min_x = world_min_x + half_map_width;
    let map_max_x = world_max_x + half_map_width;
    let map_min_y = half_map_height - world_max_y; // Flip Y
    let map_max_y = half_map_height - world_min_y;

    // Convert to chunk coordinates (with padding for smooth loading)
    let padding = 1;
    visible_range.min_x = ((map_min_x / CHUNK_SIZE).floor() as i32 - padding).max(0);
    visible_range.max_x = ((map_max_x / CHUNK_SIZE).ceil() as i32 + padding)
        .min((world_def.width as f32 / CHUNK_SIZE).ceil() as i32 - 1);
    visible_range.min_y = ((map_min_y / CHUNK_SIZE).floor() as i32 - padding).max(0);
    visible_range.max_y = ((map_max_y / CHUNK_SIZE).ceil() as i32 + padding)
        .min((world_def.height as f32 / CHUNK_SIZE).ceil() as i32 - 1);
}

/// Simple view level transition - just tracks zoom threshold.
fn handle_view_level_transition(
    camera_query: Query<&OrthographicProjection, With<Camera2d>>,
    mut view_level: ResMut<ViewLevel>,
) {
    let Ok(projection) = camera_query.get_single() else { return };

    let target_level = if projection.scale <= MESO_ZOOM_THRESHOLD {
        ViewLevel::Meso
    } else {
        ViewLevel::Macro
    };

    if *view_level != target_level {
        *view_level = target_level;
        println!("View level: {:?}", target_level);
    }
}

/// Manage meso tile sprites - spawn/despawn based on viewport.
/// Uses pre-cached textures for instant display.
fn manage_meso_tiles(
    mut commands: Commands,
    view_level: Res<ViewLevel>,
    visible_range: Res<VisibleChunkRange>,
    mut loaded_tiles: ResMut<LoadedMesoTiles>,
    cache: Res<MesoTileCache>,
    world_def: Res<WorldDefinition>,
    tiles_query: Query<(Entity, &MesoTile)>,
) {
    let half_map_width = world_def.width as f32 / 2.0;
    let half_map_height = world_def.height as f32 / 2.0;

    if *view_level != ViewLevel::Meso {
        // Despawn all meso tile sprites when at macro level
        for (entity, _) in &tiles_query {
            commands.entity(entity).despawn();
        }
        loaded_tiles.tiles.clear();
        return;
    }

    // Collect currently needed tiles
    let mut needed_tiles: HashMap<(i32, i32), bool> = HashMap::new();
    for cy in visible_range.min_y..=visible_range.max_y {
        for cx in visible_range.min_x..=visible_range.max_x {
            needed_tiles.insert((cx, cy), true);
        }
    }

    // Despawn tile sprites that are no longer visible
    let mut to_remove = Vec::new();
    for (&coord, &entity) in &loaded_tiles.tiles {
        if !needed_tiles.contains_key(&coord) {
            commands.entity(entity).despawn();
            to_remove.push(coord);
        }
    }
    for coord in to_remove {
        loaded_tiles.tiles.remove(&coord);
    }

    // Spawn sprites for visible tiles (instant from cache)
    for &coord in needed_tiles.keys() {
        if loaded_tiles.tiles.contains_key(&coord) {
            continue; // Already spawned
        }

        let (cx, cy) = coord;

        // Get from cache (all tiles should be pre-generated)
        let Some(handle) = cache.textures.get(&coord) else {
            continue;
        };

        // Calculate sprite position (center of chunk in world coords)
        let world_x = cx as f64 * CHUNK_SIZE as f64;
        let world_y = cy as f64 * CHUNK_SIZE as f64;
        let sprite_x = world_x as f32 + CHUNK_SIZE / 2.0 - half_map_width;
        let sprite_y = half_map_height - world_y as f32 - CHUNK_SIZE / 2.0;

        // Spawn meso tile sprite
        let entity = commands.spawn((
            Sprite {
                image: handle.clone(),
                custom_size: Some(Vec2::splat(CHUNK_SIZE)),
                ..default()
            },
            Transform::from_xyz(sprite_x, sprite_y, 0.1), // z=0.1 above macro map
            MesoTile { chunk_x: cx, chunk_y: cy },
        )).id();

        loaded_tiles.tiles.insert(coord, entity);
    }
}

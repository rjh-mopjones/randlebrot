//! Launch a playable level from a previously generated level artifact.
//!
//! `randlebrot launch <level-tag>` opens a minimal Bevy window with the player
//! spawned at the level's micro coordinate. Surrounding micro tiles stream in
//! as the player moves, using the parent layers artifact (macro `BiomeMap` +
//! `RiverNetwork`) for on-the-fly generation. If the parent layers artifact is
//! missing but the level manifest has a seed, the macro data is regenerated at
//! startup.
//!
//! A world map overlay is available (press M to toggle) showing the full biome
//! layer from the parent artifact with the player's position marked.
//!
//! Controls:
//!   WASD            — move player
//!   M               — toggle world map overlay
//!   Scroll wheel    — zoom (in map overlay mode)
//!   ESC             — exit

use std::collections::HashSet;
use std::sync::Arc;

use bevy::app::AppExit;
use bevy::image::{ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on, poll_once};
use bevy::window::PrimaryWindow;
use bevy_egui::{egui, EguiContexts, EguiPlugin};

use rb_artifacts::ArtifactStore;
use rb_core::{PlayableLevel, WorldPos};
use rb_noise::{BiomeMap, NoiseBackend, NoiseLayer};
use rb_player::{Player, PlayerCamera, RbPlayerPlugin};
use rb_tilemap::{LevelChunk, LoadedChunks, RbTilemapPlugin};

use crate::cli::coords::{
    micro_coord_to_world_pos, MICRO_WORLD_SIZE, WORLD_HEIGHT, WORLD_WIDTH,
};

// ─── Constants ─────────────────────────────────────────────────────────────

/// Output resolution per micro tile (512x512).
const TILE_MAP_SIZE: usize = 512;

/// Level chunk load radius (in level chunks around the player).
const LEVEL_LOAD_RADIUS: i32 = 5;

/// Level chunk unload radius.
const LEVEL_UNLOAD_RADIUS: i32 = 7;

/// Each level chunk occupies this many pixels in screen space.
const LEVEL_CHUNK_TILES: f32 = 64.0;

/// Max concurrent async tile generation tasks.
const MAX_CONCURRENT_TILES: usize = 16;

/// Max tile completions to process per frame.
const POLL_BUDGET: usize = 16;

// ─── Entry Point ───────────────────────────────────────────────────────────

/// Launch a playable level from a previously generated level artifact.
///
/// Validates the tag, loads the level manifest + micro BiomeMap, loads
/// (or regenerates) the parent layers artifact, and runs a minimal Bevy app.
pub fn run(level_tag: String) -> Result<(), String> {
    // ─── 1. Load the level artifact ────────────────────────────────────
    let store = ArtifactStore::new()
        .map_err(|e| format!("failed to initialise artifact store at ~/.randlebrot: {e}"))?;

    let (micro_biome, level_manifest) = store.load_level(&level_tag).map_err(|e| match e {
        rb_artifacts::ArtifactError::NotFound { .. } => {
            match store.list_levels() {
                Ok(entries) if !entries.is_empty() => {
                    let available: Vec<&str> = entries.iter().map(|(t, _)| t.as_str()).collect();
                    format!(
                        "level artifact '{level_tag}' not found. Available: {}",
                        available.join(", ")
                    )
                }
                _ => format!(
                    "level artifact '{level_tag}' not found. \
                     Run `randlebrot generate level <layers-tag|--seed N> <x,y> <tag>` to create one."
                ),
            }
        }
        other => format!("failed to load level artifact '{level_tag}': {other}"),
    })?;

    let (world_x, world_y) = micro_coord_to_world_pos(level_manifest.micro_coord);
    let seed = level_manifest.seed;

    println!(
        "Launching level '{level_tag}': seed={seed}, coord=({},{}), world=({world_x:.1},{world_y:.1})",
        level_manifest.micro_coord.0, level_manifest.micro_coord.1,
    );

    // ─── 2. Load parent layers for macro context ───────────────────────
    let (macro_biome, river_network) =
        load_parent_layers(&store, &level_manifest, seed)?;

    let macro_biome_arc = Arc::new(macro_biome);
    let river_network_arc = river_network;

    // ─── 3. Load the biome map image for the map overlay ───────────────
    // Try to load the pre-rendered biome.png from the parent layers artifact.
    // If not available, render one from the macro BiomeMap.
    let map_image_data = load_or_render_map_image(&store, &level_manifest, &macro_biome_arc);

    // ─── 4. Build and run the Bevy app ─────────────────────────────────
    let origin = WorldPos::new(world_x, world_y);
    // Compute the macro chunk that contains this micro coordinate.
    let chunk_x = (world_x / 64.0).floor() as i32;
    let chunk_y = (world_y / 64.0).floor() as i32;

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: format!("Randlebrot - Playing: {level_tag}"),
            resolution: (1280u32, 720u32).into(),
            ..default()
        }),
        ..default()
    }));

    app.add_plugins(EguiPlugin {
        enable_multipass_for_primary_context: false,
        ..Default::default()
    });

    // Player + tilemap plugins
    app.add_plugins(RbPlayerPlugin);
    app.add_plugins(RbTilemapPlugin);

    // Insert resources
    app.insert_resource(PlayableLevel {
        origin,
        chunk_coord: (chunk_x, chunk_y),
        seed,
        world_height: WORLD_HEIGHT as f64,
    });
    app.init_resource::<LoadedChunks>();
    app.insert_resource(LaunchMacroBiomeData {
        biome_map: macro_biome_arc,
    });
    if let Some(net) = river_network_arc {
        app.insert_resource(LaunchRiverNetwork { network: net });
    }
    app.insert_resource(LaunchLevelChunkQueue::default());
    app.insert_resource(MapOverlayState::default());

    // Pre-render the initial micro tile sprite from the loaded level artifact
    let initial_tile_data = micro_biome.to_layer_image(NoiseLayer::Biome);
    app.insert_resource(InitialMicroTile {
        biome_map: Arc::new(micro_biome),
        image_data: initial_tile_data,
    });

    // Store the map image for the overlay
    if let Some((width, height, rgba_data)) = map_image_data {
        app.insert_resource(MapImageData {
            width,
            height,
            rgba_data,
            world_width: WORLD_WIDTH as f32,
            world_height: WORLD_HEIGHT as f32,
        });
    }

    app.insert_resource(LaunchState {
        level_tag,
        micro_coord: level_manifest.micro_coord,
    });

    app.add_systems(Startup, launch_setup);
    app.add_systems(
        Update,
        (
            launch_chunk_load_system,
            launch_chunk_poll_system,
            launch_chunk_unload_system,
            toggle_map_overlay,
            update_map_player_marker,
            map_overlay_zoom,
            launch_hud_system,
            exit_on_esc,
            exit_on_window_close,
        ),
    );

    app.run();
    Ok(())
}

// ─── Parent Layers Loading ─────────────────────────────────────────────────

/// Load the parent layers artifact (macro BiomeMap + RiverNetwork) for context.
///
/// Three paths:
/// 1. Parent layers tag exists and loads successfully -> use it
/// 2. Parent layers tag missing/broken but seed available -> regenerate macro data
/// 3. No parent and no seed -> return error
fn load_parent_layers(
    store: &ArtifactStore,
    manifest: &rb_artifacts::LevelManifest,
    seed: u32,
) -> Result<(BiomeMap, Option<Arc<rb_noise::RiverNetwork>>), String> {
    // Try loading parent layers artifact
    if let Some(ref parent_tag) = manifest.parent_layers_tag {
        match store.load_layers_data(parent_tag) {
            Ok((mut biome_map, river_network, _lifegen)) => {
                // Wrap the river network in an Arc and reconnect to BiomeMap
                let river_arc = Arc::new(river_network);
                biome_map.river_network = Some(river_arc.clone());
                println!("Loaded parent layers artifact '{parent_tag}'");
                return Ok((biome_map, Some(river_arc)));
            }
            Err(e) => {
                println!(
                    "Warning: could not load parent layers '{parent_tag}': {e}\n\
                     Regenerating macro data from seed {seed}..."
                );
            }
        }
    }

    // Fallback: regenerate macro BiomeMap from seed
    println!("Generating macro BiomeMap from seed {seed} (this may take a moment)...");
    let biome_map = BiomeMap::generate_with_backend(
        seed,
        WORLD_WIDTH,
        WORLD_HEIGHT,
        NoiseBackend::Cpu,
    );
    let river_network = biome_map.river_network.clone();
    println!("Macro BiomeMap generated");
    Ok((biome_map, river_network))
}

/// Load the biome.png from the parent layers, or render one from the macro BiomeMap.
fn load_or_render_map_image(
    store: &ArtifactStore,
    manifest: &rb_artifacts::LevelManifest,
    macro_biome: &Arc<BiomeMap>,
) -> Option<(u32, u32, Vec<u8>)> {
    // Try loading pre-rendered biome.png from parent layers
    if let Some(ref parent_tag) = manifest.parent_layers_tag {
        let biome_path = store.layer_image_path(parent_tag, "biome.png");
        if biome_path.exists() {
            match image::open(&biome_path) {
                Ok(img) => {
                    let rgba = img.to_rgba8();
                    let (w, h) = rgba.dimensions();
                    println!("Loaded map image from '{parent_tag}' ({w}x{h})");
                    return Some((w, h, rgba.into_raw()));
                }
                Err(e) => {
                    println!("Warning: failed to load biome.png from '{parent_tag}': {e}");
                }
            }
        }
    }

    // Fallback: render from the macro BiomeMap
    let image_data = macro_biome.to_layer_image(NoiseLayer::Biome);
    let w = macro_biome.width as u32;
    let h = macro_biome.height as u32;
    if image_data.len() == (w * h * 4) as usize {
        println!("Rendered map image from macro BiomeMap ({w}x{h})");
        Some((w, h, image_data))
    } else {
        None
    }
}

// ─── Resources ─────────────────────────────────────────────────────────────

/// Macro BiomeMap for generating micro tiles on the fly.
#[derive(Resource)]
struct LaunchMacroBiomeData {
    biome_map: Arc<BiomeMap>,
}

/// Global river network for consistent rivers across zoom levels.
#[derive(Resource)]
struct LaunchRiverNetwork {
    network: Arc<rb_noise::RiverNetwork>,
}

/// Queue of in-flight level chunk generation tasks.
#[derive(Resource, Default)]
struct LaunchLevelChunkQueue {
    in_flight: Vec<LaunchLevelChunkTask>,
}

/// An in-flight level chunk generation task.
struct LaunchLevelChunkTask {
    coord: (i32, i32),
    task: Task<((i32, i32), Arc<BiomeMap>)>,
}

/// The pre-generated micro tile from the level artifact, displayed immediately.
#[derive(Resource)]
struct InitialMicroTile {
    biome_map: Arc<BiomeMap>,
    image_data: Vec<u8>,
}

/// Map overlay state (toggled with M key).
#[derive(Resource)]
struct MapOverlayState {
    /// Whether the map overlay is currently visible.
    visible: bool,
}

impl Default for MapOverlayState {
    fn default() -> Self {
        Self { visible: false }
    }
}

/// Pre-loaded map image data for the overlay.
#[derive(Resource)]
struct MapImageData {
    width: u32,
    height: u32,
    rgba_data: Vec<u8>,
    world_width: f32,
    world_height: f32,
}

/// General launch state info for the HUD.
#[derive(Resource)]
struct LaunchState {
    level_tag: String,
    micro_coord: (i32, i32),
}

/// Marker for the map overlay sprite.
#[derive(Component)]
struct MapOverlaySprite;

/// Marker for the player position dot on the map overlay.
#[derive(Component)]
struct MapPlayerMarker;

/// Marker for the map overlay camera (separate from the main play camera).
/// The map overlay is rendered using UI-space sprites at a fixed z-level
/// above the game world, so it doesn't need a separate camera. But we
/// track map overlay entities to despawn them cleanly.
#[derive(Component)]
struct MapOverlayEntity;

// ─── Startup ───────────────────────────────────────────────────────────────

/// Set up the camera and spawn the initial micro tile.
fn launch_setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    initial_tile: Option<Res<InitialMicroTile>>,
    map_image_data: Option<Res<MapImageData>>,
) {
    // Camera is already spawned by RbPlayerPlugin when PlayableLevel is inserted.
    // But we need to ensure Camera2d exists for the player plugin to work.
    // The player plugin's `spawn_player` runs on `resource_added::<PlayableLevel>`,
    // but since PlayableLevel is already inserted before App::run(), it will
    // trigger on the first frame. We just need the Camera2d to exist.
    commands.spawn(Camera2d);

    // Spawn the initial micro tile (pre-generated from the level artifact)
    if let Some(tile) = initial_tile {
        let image = create_image(TILE_MAP_SIZE, TILE_MAP_SIZE, tile.image_data.clone());
        let texture = images.add(image);

        // The initial tile is at level chunk (0,0) — the player spawn point
        let sprite_x = 0.0 * LEVEL_CHUNK_TILES + LEVEL_CHUNK_TILES / 2.0;
        let sprite_y = -(0.0 * LEVEL_CHUNK_TILES + LEVEL_CHUNK_TILES / 2.0);

        commands.spawn((
            Sprite {
                image: texture,
                custom_size: Some(Vec2::splat(LEVEL_CHUNK_TILES)),
                ..default()
            },
            Transform::from_xyz(sprite_x, sprite_y, 0.0),
            LevelChunk { coord: (0, 0) },
        ));
    }

    // Create the map overlay sprite (initially hidden)
    if let Some(map_data) = map_image_data {
        let map_image = create_image(
            map_data.width as usize,
            map_data.height as usize,
            map_data.rgba_data.clone(),
        );
        let map_texture = images.add(map_image);

        // Map overlay is rendered at a high z-level, centered on screen.
        // It follows the camera via a system.
        commands.spawn((
            Sprite {
                image: map_texture,
                custom_size: Some(Vec2::new(map_data.width as f32, map_data.height as f32)),
                color: Color::srgba(1.0, 1.0, 1.0, 0.85),
                ..default()
            },
            Transform::from_xyz(0.0, 0.0, 50.0),
            Visibility::Hidden,
            MapOverlaySprite,
            MapOverlayEntity,
        ));

        // Player position marker (red dot on the map)
        commands.spawn((
            Sprite {
                color: Color::srgb(1.0, 0.1, 0.1),
                custom_size: Some(Vec2::new(12.0, 12.0)),
                ..default()
            },
            Transform::from_xyz(0.0, 0.0, 51.0),
            Visibility::Hidden,
            MapPlayerMarker,
            MapOverlayEntity,
        ));
    }
}

// ─── Level Chunk Streaming ─────────────────────────────────────────────────

/// Load level chunks around the player's position.
fn launch_chunk_load_system(
    level: Res<PlayableLevel>,
    player_query: Query<&Transform, With<Player>>,
    loaded_chunks: Res<LoadedChunks>,
    mut queue: ResMut<LaunchLevelChunkQueue>,
    world_textures: Res<LaunchMacroBiomeData>,
    global_rivers: Option<Res<LaunchRiverNetwork>>,
) {
    let Ok(player_transform) = player_query.single() else { return };

    let player_pos = player_transform.translation;
    let player_chunk_x = (player_pos.x / LEVEL_CHUNK_TILES).floor() as i32;
    let player_chunk_y = ((-player_pos.y) / LEVEL_CHUNK_TILES).floor() as i32;

    let seed = level.seed;
    let height = level.world_height;
    let river_net = global_rivers.map(|r| r.network.clone());

    let in_flight_coords: HashSet<(i32, i32)> = queue
        .in_flight
        .iter()
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
                    NoiseBackend::Cpu,
                    Some(&macro_map),
                    river_ref,
                );
                (coord, Arc::new(biome_map))
            });

            queue
                .in_flight
                .push(LaunchLevelChunkTask { coord, task });
        }
    }
}

/// Poll completed level chunk tasks and spawn sprites.
fn launch_chunk_poll_system(
    mut commands: Commands,
    mut queue: ResMut<LaunchLevelChunkQueue>,
    mut loaded_chunks: ResMut<LoadedChunks>,
    mut images: ResMut<Assets<Image>>,
) {
    let mut completed = 0;
    let mut i = 0;
    while i < queue.in_flight.len() && completed < POLL_BUDGET {
        if let Some(result) = block_on(poll_once(&mut queue.in_flight[i].task)) {
            queue.in_flight.swap_remove(i);
            let (coord, biome_map) = result;

            let image_data = biome_map.to_layer_image(NoiseLayer::Biome);
            let image = create_image(TILE_MAP_SIZE, TILE_MAP_SIZE, image_data);
            let texture = images.add(image);

            let sprite_x = coord.0 as f32 * LEVEL_CHUNK_TILES + LEVEL_CHUNK_TILES / 2.0;
            let sprite_y = -(coord.1 as f32 * LEVEL_CHUNK_TILES + LEVEL_CHUNK_TILES / 2.0);

            let entity = commands
                .spawn((
                    Sprite {
                        image: texture,
                        custom_size: Some(Vec2::splat(LEVEL_CHUNK_TILES)),
                        ..default()
                    },
                    Transform::from_xyz(sprite_x, sprite_y, 0.0),
                    LevelChunk { coord },
                ))
                .id();

            loaded_chunks.chunks.insert(coord, entity);
            completed += 1;
        } else {
            i += 1;
        }
    }
}

/// Unload level chunks beyond the unload radius.
fn launch_chunk_unload_system(
    mut commands: Commands,
    mut loaded_chunks: ResMut<LoadedChunks>,
    player_query: Query<&Transform, With<Player>>,
) {
    let Ok(player_transform) = player_query.single() else { return };

    let player_pos = player_transform.translation;
    let player_chunk_x = (player_pos.x / LEVEL_CHUNK_TILES).floor() as i32;
    let player_chunk_y = ((-player_pos.y) / LEVEL_CHUNK_TILES).floor() as i32;

    let to_remove: Vec<(i32, i32)> = loaded_chunks
        .chunks
        .keys()
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

// ─── Map Overlay ───────────────────────────────────────────────────────────

/// Toggle the map overlay on/off with the M key.
fn toggle_map_overlay(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<MapOverlayState>,
    mut overlay_query: Query<&mut Visibility, With<MapOverlaySprite>>,
    mut marker_query: Query<
        &mut Visibility,
        (With<MapPlayerMarker>, Without<MapOverlaySprite>),
    >,
    camera_query: Query<&Transform, With<PlayerCamera>>,
    mut map_sprite_query: Query<
        &mut Transform,
        (With<MapOverlaySprite>, Without<PlayerCamera>, Without<MapPlayerMarker>),
    >,
    map_data: Option<Res<MapImageData>>,
) {
    if !keyboard.just_pressed(KeyCode::KeyM) {
        return;
    }

    state.visible = !state.visible;

    let new_vis = if state.visible {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };

    for mut vis in &mut overlay_query {
        *vis = new_vis;
    }
    for mut vis in &mut marker_query {
        *vis = new_vis;
    }

    // When showing the map, position it at the camera center
    if state.visible {
        if let Ok(camera_transform) = camera_query.single() {
            let cam_pos = camera_transform.translation;
            // Scale the map to fit roughly 80% of the screen
            // The map is at a high z-level, positioned at camera center
            if let Some(map_data) = map_data {
                for mut transform in &mut map_sprite_query {
                    transform.translation.x = cam_pos.x;
                    transform.translation.y = cam_pos.y;
                    // Scale the map to a reasonable display size relative to
                    // the camera's view. The map image can be 4096x2048 or
                    // 1024x512; we scale it down to a displayable overlay size.
                    let display_width = 800.0;
                    let scale = display_width / map_data.width as f32;
                    transform.scale = Vec3::splat(scale);
                }
            }
        }
    }
}

/// Update the player position marker on the map overlay.
fn update_map_player_marker(
    state: Res<MapOverlayState>,
    level: Res<PlayableLevel>,
    player_query: Query<&Transform, With<Player>>,
    camera_query: Query<&Transform, (With<PlayerCamera>, Without<Player>, Without<MapOverlaySprite>, Without<MapPlayerMarker>)>,
    map_data: Option<Res<MapImageData>>,
    mut marker_query: Query<
        &mut Transform,
        (With<MapPlayerMarker>, Without<Player>, Without<PlayerCamera>, Without<MapOverlaySprite>),
    >,
    overlay_query: Query<
        &Transform,
        (With<MapOverlaySprite>, Without<Player>, Without<PlayerCamera>, Without<MapPlayerMarker>),
    >,
) {
    if !state.visible {
        return;
    }
    let Some(map_data) = map_data else { return };
    let Ok(player_transform) = player_query.single() else { return };
    let Ok(camera_transform) = camera_query.single() else { return };
    let Ok(overlay_transform) = overlay_query.single() else { return };
    let Ok(mut marker_transform) = marker_query.single_mut() else { return };

    // Convert player's level-space position to world coordinates
    let player_pos = player_transform.translation;
    let world_x = level.origin.x + (player_pos.x as f64 / LEVEL_CHUNK_TILES as f64) * MICRO_WORLD_SIZE;
    let world_y = level.origin.y + ((-player_pos.y) as f64 / LEVEL_CHUNK_TILES as f64) * MICRO_WORLD_SIZE;

    // Normalize to [0,1] in world space
    let norm_x = (world_x / map_data.world_width as f64) as f32;
    let norm_y = (world_y / map_data.world_height as f64) as f32;

    // Map to overlay sprite position (overlay is centered at overlay_transform.translation)
    let overlay_scale = overlay_transform.scale.x;
    let map_pixel_x = (norm_x - 0.5) * map_data.width as f32 * overlay_scale;
    let map_pixel_y = (0.5 - norm_y) * map_data.height as f32 * overlay_scale;

    marker_transform.translation.x = overlay_transform.translation.x + map_pixel_x;
    marker_transform.translation.y = overlay_transform.translation.y + map_pixel_y;
    marker_transform.translation.z = 51.0;

    // Scale the marker relative to the overlay so it stays visible
    let marker_scale = overlay_scale * 1.5;
    marker_transform.scale = Vec3::splat(marker_scale);
}

/// Scroll-wheel zoom on the map overlay (when visible).
fn map_overlay_zoom(
    state: Res<MapOverlayState>,
    mut scroll_events: MessageReader<MouseWheel>,
    mut overlay_query: Query<
        &mut Transform,
        (With<MapOverlaySprite>, Without<MapPlayerMarker>),
    >,
) {
    if !state.visible {
        scroll_events.clear();
        return;
    }

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

    for mut transform in &mut overlay_query {
        let zoom_factor = 1.0 - scroll_delta;
        let current_scale = transform.scale.x;
        let new_scale = (current_scale * zoom_factor).clamp(0.01, 5.0);
        transform.scale = Vec3::splat(new_scale);
    }
}

// ─── HUD ───────────────────────────────────────────────────────────────────

/// Minimal HUD showing level info and controls.
fn launch_hud_system(
    mut contexts: EguiContexts,
    launch_state: Res<LaunchState>,
    state: Res<MapOverlayState>,
    player_query: Query<&Transform, With<Player>>,
    level: Res<PlayableLevel>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };

    let player_info = if let Ok(pt) = player_query.single() {
        format!(
            "Pos: ({:.1}, {:.1})",
            pt.translation.x, pt.translation.y,
        )
    } else {
        "Pos: —".to_string()
    };

    egui::Window::new("Randlebrot")
        .anchor(egui::Align2::LEFT_TOP, [8.0, 8.0])
        .resizable(false)
        .collapsible(false)
        .show(ctx, |ui| {
            ui.label(format!("Level: {}", launch_state.level_tag));
            ui.label(format!(
                "Coord: ({}, {})",
                launch_state.micro_coord.0, launch_state.micro_coord.1
            ));
            ui.label(format!("Seed: {}", level.seed));
            ui.label(player_info);
            if state.visible {
                ui.label("Map: ON");
            }
            ui.separator();
            ui.small("WASD: move  M: map  ESC: exit");
        });
}

// ─── Exit ──────────────────────────────────────────────────────────────────

/// Exit on ESC key.
fn exit_on_esc(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut exit_events: MessageWriter<AppExit>,
) {
    if keyboard.just_pressed(KeyCode::Escape) {
        exit_events.write(AppExit::Success);
    }
}

/// Exit when the window is closed.
fn exit_on_window_close(
    windows: Query<(), With<PrimaryWindow>>,
    mut exit_events: MessageWriter<AppExit>,
) {
    if windows.is_empty() {
        exit_events.write(AppExit::Success);
    }
}

// ─── Image Helper ──────────────────────────────────────────────────────────

/// Create a Bevy Image from RGBA data with nearest-neighbor sampling.
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

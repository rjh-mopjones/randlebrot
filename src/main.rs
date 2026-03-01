use bevy::image::{ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy_egui::EguiContexts;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use rb_core::{AppMode, ModeTransitionEvent, handle_mode_shortcuts};
use rb_editor::RegenerationRequest;
use rb_noise::BiomeMap;
use rb_world::WorldDefinition;
use std::collections::HashMap;

const MAP_WIDTH: usize = 1024;
const MAP_HEIGHT: usize = 512;

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
        .add_event::<ModeTransitionEvent>()
        .init_resource::<CurrentLayer>()
        .init_resource::<GeneratorParams>()
        .init_resource::<CursorWorldPos>()
        .init_resource::<ViewLevel>()
        .init_resource::<LoadedMesoTiles>()
        .init_resource::<VisibleChunkRange>()
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
        // Startup
        .add_systems(Startup, setup_world_map)
        // Update systems
        .add_systems(Update, (
            handle_mode_shortcuts,
            toggle_layer.run_if(in_state(AppMode::WorldGenerator)),
            regenerate_world.run_if(in_state(AppMode::WorldGenerator)),
            camera_zoom,
            camera_pan,
            calculate_visible_chunks,
            handle_view_level_transition,
            manage_meso_tiles,
            update_cursor_world_pos,
            update_chunk_highlight,
            log_mode_transition,
        ))
        .run();
}

/// Current visualization layer for World Generator mode.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum CurrentLayer {
    #[default]
    Biome,
    Temperature,
    Continentalness,
}

impl CurrentLayer {
    fn next(&self) -> Self {
        match self {
            Self::Biome => Self::Temperature,
            Self::Temperature => Self::Continentalness,
            Self::Continentalness => Self::Biome,
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Biome => "Biome",
            Self::Temperature => "Temperature",
            Self::Continentalness => "Continentalness",
        }
    }
}

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

/// Stores handles to all layer textures.
#[derive(Resource)]
struct WorldMapTextures {
    biome: Handle<Image>,
    temperature: Handle<Image>,
    continentalness: Handle<Image>,
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

/// Tracks all loaded meso tiles.
#[derive(Resource, Default)]
struct LoadedMesoTiles {
    /// Map from (chunk_x, chunk_y) to entity
    tiles: HashMap<(i32, i32), Entity>,
    /// Texture handles to prevent garbage collection
    textures: HashMap<(i32, i32), Handle<Image>>,
    /// Tiles queued for generation (processed 1 per frame)
    pending: Vec<(i32, i32)>,
}

/// Camera viewport in chunk coordinates.
#[derive(Resource, Default)]
struct VisibleChunkRange {
    min_x: i32,
    max_x: i32,
    min_y: i32,
    max_y: i32,
}

/// Size of macro chunks in pixels (for highlighting grid).
const CHUNK_SIZE: f32 = 64.0;

/// Zoom threshold for switching to meso view.
const MESO_ZOOM_THRESHOLD: f32 = 0.5;

/// Size of meso map in pixels (per tile).
const MESO_MAP_SIZE: usize = 128;

fn setup_world_map(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    world_def: Res<WorldDefinition>,
) {
    // Spawn 2D camera
    commands.spawn(Camera2d);

    println!("Generating world map {}x{}...", world_def.width, world_def.height);

    // Generate the biome map
    let biome_map = BiomeMap::generate(world_def.seed, world_def.width, world_def.height);

    println!("World map generated. Creating textures...");

    // Create biome texture
    let biome_image = create_image(world_def.width, world_def.height, biome_map.to_biome_image());
    let biome_handle = images.add(biome_image);

    // Create temperature texture
    let temp_image = create_image(world_def.width, world_def.height, biome_map.to_temperature_image());
    let temperature_handle = images.add(temp_image);

    // Create continentalness texture
    let cont_image = create_image(world_def.width, world_def.height, biome_map.to_continentalness_image());
    let continentalness_handle = images.add(cont_image);

    // Store texture handles
    commands.insert_resource(WorldMapTextures {
        biome: biome_handle.clone(),
        temperature: temperature_handle,
        continentalness: continentalness_handle,
    });

    // Spawn sprite with biome texture (default view)
    commands.spawn((
        Sprite {
            image: biome_handle,
            ..default()
        },
        WorldMapSprite,
    ));

    // Spawn chunk highlight overlay (semi-transparent rectangle)
    commands.spawn((
        Sprite {
            color: Color::srgba(1.0, 1.0, 0.8, 0.3),
            custom_size: Some(Vec2::splat(CHUNK_SIZE)),
            ..default()
        },
        Transform::from_xyz(-10000.0, -10000.0, 0.5), // Start off-screen
        ChunkHighlight,
    ));

    println!("World map ready.");
    println!("  F1: World Generator | F2: Map Editor | F3: Chunk Editor | F4: Launcher");
    println!("  Space: Toggle layer view (in Generator mode)");
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

fn toggle_layer(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut current_layer: ResMut<CurrentLayer>,
    textures: Option<Res<WorldMapTextures>>,
    mut query: Query<&mut Sprite, With<WorldMapSprite>>,
) {
    if !keyboard.just_pressed(KeyCode::Space) {
        return;
    }

    let Some(textures) = textures else {
        return;
    };

    // Cycle to next layer
    let next_layer = current_layer.next();
    *current_layer = next_layer;

    // Update sprite texture
    for mut sprite in &mut query {
        sprite.image = match next_layer {
            CurrentLayer::Biome => textures.biome.clone(),
            CurrentLayer::Temperature => textures.temperature.clone(),
            CurrentLayer::Continentalness => textures.continentalness.clone(),
        };
    }

    println!("Switched to {} view", next_layer.name());
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
    textures: Res<WorldMapTextures>,
    mut query: Query<&mut Sprite, With<WorldMapSprite>>,
    current_layer: Res<CurrentLayer>,
) {
    if !regen_request.pending {
        return;
    }
    regen_request.pending = false;

    println!("Regenerating world map with seed {}...", world_def.seed);

    // Generate new biome map
    let biome_map = BiomeMap::generate(world_def.seed, world_def.width, world_def.height);

    // Update textures
    let biome_image = create_image(world_def.width, world_def.height, biome_map.to_biome_image());
    let temp_image = create_image(world_def.width, world_def.height, biome_map.to_temperature_image());
    let cont_image = create_image(world_def.width, world_def.height, biome_map.to_continentalness_image());

    // Replace image assets
    if let Some(img) = images.get_mut(&textures.biome) {
        *img = biome_image;
    }
    if let Some(img) = images.get_mut(&textures.temperature) {
        *img = temp_image;
    }
    if let Some(img) = images.get_mut(&textures.continentalness) {
        *img = cont_image;
    }

    // Update sprite to current layer
    for mut sprite in &mut query {
        sprite.image = match *current_layer {
            CurrentLayer::Biome => textures.biome.clone(),
            CurrentLayer::Temperature => textures.temperature.clone(),
            CurrentLayer::Continentalness => textures.continentalness.clone(),
        };
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
        projection.scale = (projection.scale * zoom_factor).clamp(0.1, 10.0);
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
    let over_ui = contexts.ctx_mut().is_pointer_over_area();
    if mouse.pressed(MouseButton::Left) && !over_ui {
        for event in motion_events.read() {
            pan_delta -= event.delta;
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
/// Generates at most 1 tile per frame for smooth loading.
fn manage_meso_tiles(
    mut commands: Commands,
    view_level: Res<ViewLevel>,
    visible_range: Res<VisibleChunkRange>,
    mut loaded_tiles: ResMut<LoadedMesoTiles>,
    mut images: ResMut<Assets<Image>>,
    world_def: Res<WorldDefinition>,
    current_layer: Res<CurrentLayer>,
    tiles_query: Query<(Entity, &MesoTile)>,
) {
    let half_map_width = world_def.width as f32 / 2.0;
    let half_map_height = world_def.height as f32 / 2.0;

    if *view_level != ViewLevel::Meso {
        // Despawn all meso tiles when at macro level
        for (entity, _) in &tiles_query {
            commands.entity(entity).despawn();
        }
        // Clean up resources
        for (_, handle) in loaded_tiles.textures.drain() {
            images.remove(&handle);
        }
        loaded_tiles.tiles.clear();
        loaded_tiles.pending.clear();
        return;
    }

    // Collect currently needed tiles
    let mut needed_tiles: HashMap<(i32, i32), bool> = HashMap::new();
    for cy in visible_range.min_y..=visible_range.max_y {
        for cx in visible_range.min_x..=visible_range.max_x {
            needed_tiles.insert((cx, cy), true);
        }
    }

    // Despawn tiles that are no longer needed
    let mut to_remove = Vec::new();
    for (&coord, &entity) in &loaded_tiles.tiles {
        if !needed_tiles.contains_key(&coord) {
            commands.entity(entity).despawn();
            to_remove.push(coord);
        }
    }
    for coord in to_remove {
        loaded_tiles.tiles.remove(&coord);
        if let Some(handle) = loaded_tiles.textures.remove(&coord) {
            images.remove(&handle);
        }
    }

    // Remove stale pending tiles that are no longer needed
    loaded_tiles.pending.retain(|coord| needed_tiles.contains_key(coord));

    // Queue missing tiles (don't generate yet)
    for &coord in needed_tiles.keys() {
        if !loaded_tiles.tiles.contains_key(&coord)
            && !loaded_tiles.pending.contains(&coord)
        {
            loaded_tiles.pending.push(coord);
        }
    }

    // Generate only 1 tile this frame for smooth loading
    if let Some(coord) = loaded_tiles.pending.pop() {
        let (cx, cy) = coord;

        // Generate meso map for this chunk
        let world_x = cx as f64 * CHUNK_SIZE as f64;
        let world_y = cy as f64 * CHUNK_SIZE as f64;

        let meso_map = BiomeMap::generate_region(
            world_def.seed,
            world_x,
            world_y,
            CHUNK_SIZE as f64,
            MESO_MAP_SIZE,
            world_def.height as f64,
            1, // detail_level 1 for meso
        );

        // Create texture based on current layer
        let image_data = match *current_layer {
            CurrentLayer::Biome => meso_map.to_biome_image(),
            CurrentLayer::Temperature => meso_map.to_temperature_image(),
            CurrentLayer::Continentalness => meso_map.to_continentalness_image(),
        };

        let meso_image = create_image(MESO_MAP_SIZE, MESO_MAP_SIZE, image_data);
        let handle = images.add(meso_image);

        // Calculate sprite position (center of chunk in world coords)
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
        loaded_tiles.textures.insert(coord, handle);
    }
}

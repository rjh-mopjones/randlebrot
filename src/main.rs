use bevy::image::{ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy_egui::EguiContexts;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use rb_core::{AppMode, ModeTransitionEvent, handle_mode_shortcuts};
use rb_editor::RegenerationRequest;
use rb_noise::BiomeMap;
use rb_world::WorldDefinition;

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

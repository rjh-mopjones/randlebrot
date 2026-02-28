use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use rb_noise::BiomeMap;

const MAP_WIDTH: usize = 1024;
const MAP_HEIGHT: usize = 512;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Randlebrot - Tidally Locked World".into(),
                resolution: (MAP_WIDTH as f32, MAP_HEIGHT as f32).into(),
                ..default()
            }),
            ..default()
        }))
        .init_state::<AppMode>()
        .init_resource::<CurrentLayer>()
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
        .add_systems(Startup, setup_world_map)
        .add_systems(Update, toggle_layer)
        .run();
}

/// Application mode state.
#[derive(States, Default, Clone, Eq, PartialEq, Hash, Debug)]
pub enum AppMode {
    #[default]
    Editor,
    Play,
}

/// Current visualization layer.
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

fn setup_world_map(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    // Spawn 2D camera
    commands.spawn(Camera2d);

    println!("Generating world map {}x{}...", MAP_WIDTH, MAP_HEIGHT);

    // Generate the biome map
    let biome_map = BiomeMap::generate(42, MAP_WIDTH, MAP_HEIGHT);

    println!("World map generated. Creating textures...");

    // Create biome texture
    let biome_image = create_image(MAP_WIDTH, MAP_HEIGHT, biome_map.to_biome_image());
    let biome_handle = images.add(biome_image);

    // Create temperature texture
    let temp_image = create_image(MAP_WIDTH, MAP_HEIGHT, biome_map.to_temperature_image());
    let temperature_handle = images.add(temp_image);

    // Create continentalness texture
    let cont_image = create_image(MAP_WIDTH, MAP_HEIGHT, biome_map.to_continentalness_image());
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

    println!("World map ready. Press SPACE to toggle layers (Biome/Temperature/Continentalness)");
}

fn create_image(width: usize, height: usize, data: Vec<u8>) -> Image {
    Image::new(
        Extent3d {
            width: width as u32,
            height: height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        default(),
    )
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

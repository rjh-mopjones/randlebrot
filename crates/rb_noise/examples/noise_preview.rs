//! Noise preview example - visualizes continentalness and temperature noise.
//!
//! Run with: cargo run -p rb_noise --example noise_preview

use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use rb_core::DetailLevel;
use rb_noise::{ContinentalnessStrategy, TemperatureStrategy, WorldChunks};

const PREVIEW_WIDTH: u32 = 512;
const PREVIEW_HEIGHT: u32 = 512;
const WORLD_SCALE: f64 = 2.0; // World units per pixel

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Noise Preview".into(),
                resolution: (1024.0, 512.0).into(),
                ..default()
            }),
            ..default()
        }))
        .init_resource::<WorldChunks>()
        .init_resource::<CameraOffset>()
        .add_systems(Startup, setup)
        .add_systems(Update, (handle_input, update_noise_textures))
        .run();
}

#[derive(Resource, Default)]
struct CameraOffset {
    x: f64,
    y: f64,
    dirty: bool,
}

#[derive(Component)]
struct ContinentalnessTexture;

#[derive(Component)]
struct TemperatureTexture;

fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut camera_offset: ResMut<CameraOffset>,
) {
    // Camera
    commands.spawn(Camera2d);

    // Create continentalness texture (left side)
    let continentalness_image = create_noise_image();
    let continentalness_handle = images.add(continentalness_image);

    commands.spawn((
        Sprite {
            image: continentalness_handle,
            ..default()
        },
        Transform::from_xyz(-256.0, 0.0, 0.0),
        ContinentalnessTexture,
    ));

    // Create temperature texture (right side)
    let temperature_image = create_noise_image();
    let temperature_handle = images.add(temperature_image);

    commands.spawn((
        Sprite {
            image: temperature_handle,
            ..default()
        },
        Transform::from_xyz(256.0, 0.0, 0.0),
        TemperatureTexture,
    ));

    // Mark as dirty to generate initial textures
    camera_offset.dirty = true;
}

fn create_noise_image() -> Image {
    let size = Extent3d {
        width: PREVIEW_WIDTH,
        height: PREVIEW_HEIGHT,
        depth_or_array_layers: 1,
    };

    let data = vec![128u8; (PREVIEW_WIDTH * PREVIEW_HEIGHT * 4) as usize];

    Image::new(
        size,
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        default(),
    )
}

fn handle_input(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut camera_offset: ResMut<CameraOffset>,
) {
    let speed = 50.0;
    let mut moved = false;

    if keyboard.pressed(KeyCode::ArrowLeft) || keyboard.pressed(KeyCode::KeyA) {
        camera_offset.x -= speed;
        moved = true;
    }
    if keyboard.pressed(KeyCode::ArrowRight) || keyboard.pressed(KeyCode::KeyD) {
        camera_offset.x += speed;
        moved = true;
    }
    if keyboard.pressed(KeyCode::ArrowUp) || keyboard.pressed(KeyCode::KeyW) {
        camera_offset.y -= speed;
        moved = true;
    }
    if keyboard.pressed(KeyCode::ArrowDown) || keyboard.pressed(KeyCode::KeyS) {
        camera_offset.y += speed;
        moved = true;
    }

    if moved {
        camera_offset.dirty = true;
    }
}

fn update_noise_textures(
    mut camera_offset: ResMut<CameraOffset>,
    mut world_chunks: ResMut<WorldChunks>,
    mut images: ResMut<Assets<Image>>,
    continentalness_query: Query<&Sprite, With<ContinentalnessTexture>>,
    temperature_query: Query<&Sprite, With<TemperatureTexture>>,
) {
    if !camera_offset.dirty {
        return;
    }
    camera_offset.dirty = false;

    let detail_level = DetailLevel::Macro;
    let offset_x = camera_offset.x;
    let offset_y = camera_offset.y;

    // Update continentalness texture
    if let Ok(sprite) = continentalness_query.get_single() {
        if let Some(image) = images.get_mut(&sprite.image) {
            for y in 0..PREVIEW_HEIGHT {
                for x in 0..PREVIEW_WIDTH {
                    let world_x = offset_x + (x as f64) * WORLD_SCALE;
                    let world_y = offset_y + (y as f64) * WORLD_SCALE;

                    let value = world_chunks.sample_continentalness(world_x, world_y, detail_level);

                    // Map [-1, 1] to [0, 255] grayscale
                    let gray = ((value + 1.0) * 0.5 * 255.0).clamp(0.0, 255.0) as u8;

                    let idx = ((y * PREVIEW_WIDTH + x) * 4) as usize;
                    image.data[idx] = gray;     // R
                    image.data[idx + 1] = gray; // G
                    image.data[idx + 2] = gray; // B
                    image.data[idx + 3] = 255;  // A
                }
            }
        }
    }

    // Update temperature texture
    if let Ok(sprite) = temperature_query.get_single() {
        if let Some(image) = images.get_mut(&sprite.image) {
            for y in 0..PREVIEW_HEIGHT {
                for x in 0..PREVIEW_WIDTH {
                    let world_x = offset_x + (x as f64) * WORLD_SCALE;
                    let world_y = offset_y + (y as f64) * WORLD_SCALE;

                    let value = world_chunks.sample_temperature(world_x, world_y, detail_level);

                    // Map [-100, 100] to blue-to-red gradient
                    let normalized = ((value + 100.0) / 200.0).clamp(0.0, 1.0);

                    // Cold (blue) to hot (red) gradient
                    let r = (normalized * 255.0) as u8;
                    let b = ((1.0 - normalized) * 255.0) as u8;
                    let g = ((1.0 - (normalized - 0.5).abs() * 2.0) * 128.0) as u8;

                    let idx = ((y * PREVIEW_WIDTH + x) * 4) as usize;
                    image.data[idx] = r;       // R
                    image.data[idx + 1] = g;   // G
                    image.data[idx + 2] = b;   // B
                    image.data[idx + 3] = 255; // A
                }
            }
        }
    }

    println!(
        "Updated noise at offset ({:.0}, {:.0})",
        offset_x, offset_y
    );
}

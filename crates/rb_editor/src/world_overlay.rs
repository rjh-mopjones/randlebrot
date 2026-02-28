use bevy::prelude::*;
use rb_world::{CityTier, LandmarkKind, WorldDefinition};

/// Marker component for city sprites.
#[derive(Component)]
pub struct CityMarker {
    pub city_id: u32,
}

/// Marker component for landmark sprites.
#[derive(Component)]
pub struct LandmarkMarker {
    pub landmark_id: u32,
}

/// Marker component for region boundary shapes.
#[derive(Component)]
pub struct RegionBoundary {
    pub region_id: u32,
}

/// Resource for overlay visibility settings.
#[derive(Resource)]
pub struct OverlaySettings {
    pub show_cities: bool,
    pub show_landmarks: bool,
    pub show_regions: bool,
    pub show_chunk_grid: bool,
}

impl Default for OverlaySettings {
    fn default() -> Self {
        Self {
            show_cities: true,
            show_landmarks: true,
            show_regions: true,
            show_chunk_grid: false,
        }
    }
}

/// System to spawn city markers when entering World Map Editor mode.
pub fn spawn_overlays(
    mut commands: Commands,
    world_def: Res<WorldDefinition>,
    query: Query<Entity, With<CityMarker>>,
) {
    // Skip if overlays already exist
    if !query.is_empty() {
        return;
    }

    // Spawn city markers
    for city in &world_def.cities {
        let color = city_color(city.tier);
        let size = city_size(city.tier);

        // Convert world position to screen position
        // World is centered at origin, so offset by half width/height
        let x = city.position.x as f32 - (world_def.width as f32 / 2.0);
        let y = -(city.position.y as f32 - (world_def.height as f32 / 2.0)); // Flip Y

        commands.spawn((
            Sprite {
                color,
                custom_size: Some(Vec2::splat(size)),
                ..default()
            },
            Transform::from_xyz(x, y, 1.0),
            CityMarker { city_id: city.id },
        ));
    }

    // Spawn landmark markers
    for landmark in &world_def.landmarks {
        let color = landmark_color(landmark.kind);

        let x = landmark.position.x as f32 - (world_def.width as f32 / 2.0);
        let y = -(landmark.position.y as f32 - (world_def.height as f32 / 2.0));

        commands.spawn((
            Sprite {
                color,
                custom_size: Some(Vec2::splat(8.0)),
                ..default()
            },
            Transform::from_xyz(x, y, 1.0),
            LandmarkMarker { landmark_id: landmark.id },
        ));
    }
}

/// System to despawn overlays when leaving World Map Editor mode.
pub fn despawn_overlays(
    mut commands: Commands,
    city_query: Query<Entity, With<CityMarker>>,
    landmark_query: Query<Entity, With<LandmarkMarker>>,
    region_query: Query<Entity, With<RegionBoundary>>,
) {
    for entity in city_query.iter().chain(landmark_query.iter()).chain(region_query.iter()) {
        commands.entity(entity).despawn();
    }
}

/// System to update overlay positions when world definition changes.
pub fn update_overlays(
    world_def: Res<WorldDefinition>,
    mut city_query: Query<(&CityMarker, &mut Transform, &mut Sprite)>,
    mut landmark_query: Query<(&LandmarkMarker, &mut Transform, &mut Sprite), Without<CityMarker>>,
) {
    if !world_def.is_changed() {
        return;
    }

    // Update city positions
    for (marker, mut transform, mut sprite) in &mut city_query {
        if let Some(city) = world_def.cities.iter().find(|c| c.id == marker.city_id) {
            let x = city.position.x as f32 - (world_def.width as f32 / 2.0);
            let y = -(city.position.y as f32 - (world_def.height as f32 / 2.0));
            transform.translation.x = x;
            transform.translation.y = y;
            sprite.color = city_color(city.tier);
            sprite.custom_size = Some(Vec2::splat(city_size(city.tier)));
        }
    }

    // Update landmark positions
    for (marker, mut transform, mut sprite) in &mut landmark_query {
        if let Some(landmark) = world_def.landmarks.iter().find(|l| l.id == marker.landmark_id) {
            let x = landmark.position.x as f32 - (world_def.width as f32 / 2.0);
            let y = -(landmark.position.y as f32 - (world_def.height as f32 / 2.0));
            transform.translation.x = x;
            transform.translation.y = y;
            sprite.color = landmark_color(landmark.kind);
        }
    }
}

/// Get the display color for a city tier.
fn city_color(tier: CityTier) -> Color {
    match tier {
        CityTier::Capital => Color::srgb(1.0, 0.84, 0.0), // Gold
        CityTier::Town => Color::srgb(0.8, 0.8, 0.8),     // Silver
        CityTier::Village => Color::srgb(0.6, 0.4, 0.2),  // Brown
    }
}

/// Get the display size for a city tier.
fn city_size(tier: CityTier) -> f32 {
    match tier {
        CityTier::Capital => 16.0,
        CityTier::Town => 10.0,
        CityTier::Village => 6.0,
    }
}

/// Get the display color for a landmark kind.
fn landmark_color(kind: LandmarkKind) -> Color {
    match kind {
        LandmarkKind::Ruin => Color::srgb(0.5, 0.5, 0.5),
        LandmarkKind::Temple => Color::srgb(1.0, 1.0, 0.8),
        LandmarkKind::Tower => Color::srgb(0.6, 0.6, 0.8),
        LandmarkKind::Cave => Color::srgb(0.3, 0.3, 0.3),
        LandmarkKind::Bridge => Color::srgb(0.6, 0.4, 0.2),
        LandmarkKind::Monument => Color::srgb(0.9, 0.9, 0.9),
        LandmarkKind::Mine => Color::srgb(0.4, 0.3, 0.2),
        LandmarkKind::Port => Color::srgb(0.2, 0.5, 0.8),
        LandmarkKind::Other => Color::srgb(0.5, 0.5, 0.5),
    }
}

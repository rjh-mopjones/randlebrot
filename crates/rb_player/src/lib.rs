use bevy::prelude::*;
use rb_core::PlayableLevel;

/// Player marker component.
#[derive(Component)]
pub struct Player {
    pub speed: f32,
}

/// Marker for the camera that follows the player.
#[derive(Component)]
pub struct PlayerCamera;

/// Saved camera state to restore when exiting play mode.
#[derive(Resource)]
pub struct SavedCameraState {
    pub scale: f32,
    pub position: Vec3,
}

/// Player plugin for Randlebrot.
/// Handles player controller, camera, and 2D top-down interaction.
pub struct RbPlayerPlugin;

impl Plugin for RbPlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (
            spawn_player.run_if(resource_added::<PlayableLevel>),
            player_movement.run_if(resource_exists::<PlayableLevel>),
            camera_follow_player.run_if(resource_exists::<PlayableLevel>),
            despawn_player.run_if(resource_removed::<PlayableLevel>),
        ));
    }
}

/// Spawn player sprite and configure camera for play mode.
fn spawn_player(
    mut commands: Commands,
    level: Res<PlayableLevel>,
    mut camera_query: Query<(Entity, &mut Projection, &mut Transform), With<Camera2d>>,
) {
    // Spawn player at center of selected macro chunk in level space
    // Level chunk (0,0) is at the origin; player starts at center of chunk grid
    let player_x = 32.0; // center of chunk (0,0) in level tiles
    let player_y = -32.0; // Y-flipped for Bevy coords

    commands.spawn((
        Sprite {
            color: Color::srgb(0.2, 0.6, 1.0),
            custom_size: Some(Vec2::new(1.0, 1.5)),
            ..default()
        },
        Transform::from_xyz(player_x, player_y, 1.0),
        Player { speed: 150.0 },
    ));

    // Save camera state and set up for play mode
    if let Ok((entity, mut projection, mut transform)) = camera_query.single_mut() {
        let current_scale = match &*projection {
            Projection::Orthographic(o) => o.scale,
            _ => 1.0,
        };
        commands.insert_resource(SavedCameraState {
            scale: current_scale,
            position: transform.translation,
        });

        // Set camera for street-level view (~30 tiles visible on screen)
        if let Projection::Orthographic(ref mut o) = *projection {
            o.scale = 0.035;
        }
        transform.translation = Vec3::new(player_x, player_y, transform.translation.z);

        commands.entity(entity).insert(PlayerCamera);
    }

    println!(
        "Player spawned at level chunk ({}, {}) — world origin ({:.1}, {:.1})",
        level.chunk_coord.0, level.chunk_coord.1,
        level.origin.x, level.origin.y,
    );
}

/// Despawn player and restore camera when leaving play mode.
fn despawn_player(
    mut commands: Commands,
    player_query: Query<Entity, With<Player>>,
    mut camera_query: Query<(Entity, &mut Projection, &mut Transform), With<PlayerCamera>>,
    saved_state: Option<Res<SavedCameraState>>,
) {
    for entity in &player_query {
        commands.entity(entity).despawn();
    }

    if let Ok((entity, mut projection, mut transform)) = camera_query.single_mut() {
        if let Some(saved) = saved_state {
            if let Projection::Orthographic(ref mut o) = *projection {
                o.scale = saved.scale;
            }
            transform.translation = saved.position;
        }
        commands.entity(entity).remove::<PlayerCamera>();
    }

    commands.remove_resource::<SavedCameraState>();
    println!("Player despawned, camera restored");
}

/// WASD player movement.
fn player_movement(
    keyboard: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut query: Query<(&mut Transform, &Player)>,
) {
    let mut direction = Vec3::ZERO;
    if keyboard.pressed(KeyCode::KeyW) { direction.y += 1.0; }
    if keyboard.pressed(KeyCode::KeyS) { direction.y -= 1.0; }
    if keyboard.pressed(KeyCode::KeyA) { direction.x -= 1.0; }
    if keyboard.pressed(KeyCode::KeyD) { direction.x += 1.0; }

    if direction != Vec3::ZERO {
        direction = direction.normalize();
        for (mut transform, player) in &mut query {
            transform.translation += direction * player.speed * time.delta_secs();
        }
    }
}

/// Smooth camera follow on the player.
fn camera_follow_player(
    player_query: Query<&Transform, (With<Player>, Without<PlayerCamera>)>,
    mut camera_query: Query<&mut Transform, With<PlayerCamera>>,
    time: Res<Time>,
) {
    let Ok(player_transform) = player_query.single() else { return };
    let Ok(mut camera_transform) = camera_query.single_mut() else { return };

    let target = Vec3::new(
        player_transform.translation.x,
        player_transform.translation.y,
        camera_transform.translation.z,
    );
    let lerp_speed = 5.0 * time.delta_secs();
    camera_transform.translation = camera_transform.translation.lerp(target, lerp_speed.min(1.0));
}

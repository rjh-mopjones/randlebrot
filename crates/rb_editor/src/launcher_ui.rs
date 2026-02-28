use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use rb_core::AppMode;
use rb_world::SelectedChunk;

/// State for the level launcher.
#[derive(Resource, Default)]
pub struct LauncherState {
    /// Is gameplay active (vs paused/setup).
    pub is_playing: bool,
    /// Show debug overlays.
    pub show_debug: bool,
    /// Show collision boxes.
    pub show_collision: bool,
    /// Show FPS counter.
    pub show_fps: bool,
}

/// Marker component for the test player.
#[derive(Component)]
pub struct TestPlayer;

/// System to render the Level Launcher UI panel.
pub fn launcher_ui_system(
    mut contexts: EguiContexts,
    selected_chunk: Res<SelectedChunk>,
    mut state: ResMut<LauncherState>,
    current_mode: Res<State<AppMode>>,
    time: Res<Time>,
) {
    // Only show in Level Launcher mode
    if *current_mode.get() != AppMode::LevelLauncher {
        return;
    }

    egui::SidePanel::left("launcher_panel")
        .default_width(150.0)
        .show(contexts.ctx_mut(), |ui| {
            ui.heading("Level Launcher");
            ui.separator();

            // Chunk info
            if let Some((cx, cy)) = selected_chunk.coord {
                ui.label(format!("Testing chunk ({}, {})", cx, cy));
            } else {
                ui.label("No chunk selected");
                ui.add_space(8.0);
                ui.label("Select a chunk in");
                ui.label("Chunk Editor (F3)");
            }

            ui.add_space(16.0);
            ui.separator();

            // Play controls
            let play_text = if state.is_playing { "Stop" } else { "Play" };
            if ui.button(play_text).clicked() {
                state.is_playing = !state.is_playing;
                println!("{}", if state.is_playing { "Started playtest" } else { "Stopped playtest" });
            }

            ui.add_space(8.0);

            // Player controls (when playing)
            if state.is_playing {
                ui.label("Controls:");
                ui.label("  WASD - Move");
                ui.label("  ESC - Stop");
            }

            ui.add_space(16.0);
            ui.separator();

            // Debug options
            ui.label("Debug:");
            ui.checkbox(&mut state.show_debug, "Debug overlay");
            ui.checkbox(&mut state.show_collision, "Collision");
            ui.checkbox(&mut state.show_fps, "FPS counter");

            // FPS display
            if state.show_fps {
                ui.add_space(8.0);
                let fps = 1.0 / time.delta_secs();
                ui.label(format!("FPS: {:.1}", fps));
            }

            ui.add_space(16.0);
            ui.separator();

            // Navigation
            if ui.button("Edit Chunk (F3)").clicked() {
                println!("Use F3 to switch to Chunk Editor mode");
            }
        });
}

/// System to spawn test player when entering launcher mode.
pub fn spawn_test_player(
    mut commands: Commands,
    selected_chunk: Res<SelectedChunk>,
    state: Res<LauncherState>,
    query: Query<Entity, With<TestPlayer>>,
) {
    // Only spawn if no player exists and we're playing
    if !query.is_empty() || !state.is_playing {
        return;
    }

    // Spawn at center of selected chunk or origin
    let pos = if let Some((cx, cy)) = selected_chunk.coord {
        let chunk_size = 64.0;
        Vec3::new(
            (cx as f32 + 0.5) * chunk_size,
            (cy as f32 + 0.5) * chunk_size,
            2.0,
        )
    } else {
        Vec3::new(0.0, 0.0, 2.0)
    };

    commands.spawn((
        Sprite {
            color: Color::srgb(0.2, 0.6, 1.0),
            custom_size: Some(Vec2::new(16.0, 24.0)),
            ..default()
        },
        Transform::from_translation(pos),
        TestPlayer,
    ));

    println!("Spawned test player at {:?}", pos);
}

/// System to despawn test player when leaving launcher mode.
pub fn despawn_test_player(
    mut commands: Commands,
    query: Query<Entity, With<TestPlayer>>,
) {
    for entity in &query {
        commands.entity(entity).despawn();
    }
}

/// System to handle player movement during playtest.
pub fn player_movement_system(
    keyboard: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    state: Res<LauncherState>,
    mut query: Query<&mut Transform, With<TestPlayer>>,
    current_mode: Res<State<AppMode>>,
) {
    // Only move in launcher mode when playing
    if *current_mode.get() != AppMode::LevelLauncher || !state.is_playing {
        return;
    }

    let speed = 100.0;
    let mut direction = Vec3::ZERO;

    if keyboard.pressed(KeyCode::KeyW) {
        direction.y += 1.0;
    }
    if keyboard.pressed(KeyCode::KeyS) {
        direction.y -= 1.0;
    }
    if keyboard.pressed(KeyCode::KeyA) {
        direction.x -= 1.0;
    }
    if keyboard.pressed(KeyCode::KeyD) {
        direction.x += 1.0;
    }

    if direction != Vec3::ZERO {
        direction = direction.normalize();
        for mut transform in &mut query {
            transform.translation += direction * speed * time.delta_secs();
        }
    }
}

/// System to handle escape key to stop playtest.
pub fn escape_to_stop_system(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<LauncherState>,
    current_mode: Res<State<AppMode>>,
) {
    if *current_mode.get() != AppMode::LevelLauncher {
        return;
    }

    if keyboard.just_pressed(KeyCode::Escape) && state.is_playing {
        state.is_playing = false;
        println!("Stopped playtest");
    }
}

use bevy::prelude::*;
use rb_core::{PlayableLevel, SelectedChunk};
use rb_tilemap::{LevelChunk, LoadedChunks};

/// System to handle ESC key to exit play mode directly.
/// ESC removes PlayableLevel (stops micro generation) but keeps SelectedChunk
/// so the user can click again to re-enter play at the same chunk.
pub fn escape_to_stop_system(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    existing_level: Option<Res<PlayableLevel>>,
    level_chunks: Query<Entity, With<LevelChunk>>,
) {
    if existing_level.is_none() {
        return;
    }

    if keyboard.just_pressed(KeyCode::Escape) {
        commands.remove_resource::<PlayableLevel>();
        commands.remove_resource::<LoadedChunks>();

        for entity in &level_chunks {
            commands.entity(entity).despawn();
        }

        println!("Exited play mode");
    }
}

/// System to clean up when leaving LevelLauncher mode entirely.
pub fn cleanup_on_exit(
    mut commands: Commands,
    existing_level: Option<Res<PlayableLevel>>,
    existing_selected: Option<Res<SelectedChunk>>,
    level_chunks: Query<Entity, With<LevelChunk>>,
) {
    if existing_level.is_some() {
        commands.remove_resource::<PlayableLevel>();
        commands.remove_resource::<LoadedChunks>();
    }

    if existing_selected.is_some() {
        commands.remove_resource::<SelectedChunk>();
    }

    for entity in &level_chunks {
        commands.entity(entity).despawn();
    }
}

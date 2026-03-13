use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use rb_core::{PlayableLevel, SelectedChunk};
use rb_tilemap::{LevelChunk, LoadedChunks};

/// Signal to generate a mesomap for the selected chunk.
#[derive(Resource)]
pub struct GenerateMesoRequest;

/// Launcher UI panel — shows selected chunk info and Generate Mesomap button.
pub fn launcher_ui_system(
    mut commands: Commands,
    mut contexts: EguiContexts,
    selected: Option<Res<SelectedChunk>>,
    playing: Option<Res<PlayableLevel>>,
) {
    let ctx = contexts.ctx_mut();

    egui::SidePanel::left("launcher_panel")
        .default_width(220.0)
        .show(ctx, |ui| {
            ui.heading("Level Launcher");
            ui.separator();

            if let Some(selected) = &selected {
                let (cx, cy) = selected.chunk_coord;
                ui.label(format!("Selected chunk: ({cx}, {cy})"));
                ui.label(format!(
                    "World origin: ({:.0}, {:.0})",
                    selected.origin.x, selected.origin.y
                ));

                ui.separator();

                if playing.is_some() {
                    ui.label("Playing — press ESC to stop");
                } else if ui.button("Generate Mesomap").clicked() {
                    commands.insert_resource(GenerateMesoRequest);
                }
            } else {
                ui.label("No chunk selected.");
                ui.label("Click a tile on the world map,");
                ui.label("then press F4.");
            }
        });
}

/// System to handle ESC key to exit play mode directly.
/// ESC removes PlayableLevel (stops micro generation) but keeps SelectedChunk
/// so the user can click the button again.
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

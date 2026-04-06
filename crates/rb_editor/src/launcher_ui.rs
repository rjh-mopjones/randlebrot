use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use rb_core::{PlayableLevel, SelectedChunk, SelectedMesoTile, SelectedMicroTile};
use rb_tilemap::{LevelChunk, LoadedChunks};

/// Signal to generate meso tiles for the selected macro chunk.
#[derive(Resource)]
pub struct GenerateMesoRequest;

/// Signal to launch the level at the selected meso tile (starts micro generation).
#[derive(Resource)]
pub struct LaunchLevelRequest;

/// Signal to start playing at the selected micro tile.
#[derive(Resource)]
pub struct StartPlayRequest;

/// Signal requesting the current micro tile be saved as a level artifact.
///
/// Emitted by the launcher UI when the user confirms a level tag.
/// Consumed by a system in `main.rs` that has access to `MicroTileCache`.
#[derive(Resource)]
pub struct SaveLevelRequest {
    /// User-chosen tag for the level artifact.
    pub tag: String,
}

/// UI state for the "Save Level" dialog in the launcher panel.
#[derive(Resource, Default)]
pub struct SaveLevelUiState {
    /// Whether the save dialog is currently open.
    pub show_dialog: bool,
    /// Tag text being edited by the user.
    pub tag_input: String,
    /// Status message after save attempt (message, is_error).
    pub status: Option<(String, bool)>,
}

/// Launcher phase — tracks progression through the multi-step workflow.
#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug)]
pub enum LauncherPhase {
    /// Showing the selected macro chunk, waiting for "Generate Mesomap".
    MacroView,
    /// Generating 8×8 meso tiles asynchronously.
    GeneratingMeso,
    /// Displaying the 8×8 meso grid, user can select a meso tile.
    MesoView,
    /// Generating 32×32 micro tiles asynchronously.
    GeneratingMicro,
    /// Displaying the 32×32 micro grid, user can select a micro tile.
    MicroView,
    /// Playing at micro level.
    Playing,
}

/// Launcher UI panel — phase-aware side panel.
pub fn launcher_ui_system(
    mut commands: Commands,
    mut contexts: EguiContexts,
    selected_chunk: Option<Res<SelectedChunk>>,
    selected_meso: Option<Res<SelectedMesoTile>>,
    selected_micro: Option<Res<SelectedMicroTile>>,
    phase: Option<Res<LauncherPhase>>,
    playing: Option<Res<PlayableLevel>>,
    mut save_state: ResMut<SaveLevelUiState>,
) {
    let ctx = contexts.ctx_mut().unwrap();
    let phase = phase.map(|p| *p).unwrap_or(LauncherPhase::MacroView);

    egui::SidePanel::left("launcher_panel")
        .default_width(220.0)
        .show(ctx, |ui| {
            ui.heading("Level Launcher");
            ui.separator();

            match phase {
                LauncherPhase::MacroView => {
                    if let Some(ref chunk) = selected_chunk {
                        let (cx, cy) = chunk.chunk_coord;
                        ui.label(format!("Macro chunk: ({cx}, {cy})"));
                        ui.label(format!(
                            "World origin: ({:.0}, {:.0})",
                            chunk.origin.x, chunk.origin.y
                        ));
                        ui.separator();
                        if ui.button("Generate Mesomap").clicked() {
                            commands.insert_resource(GenerateMesoRequest);
                        }
                    } else {
                        ui.label("No chunk selected.");
                        ui.label("Click a tile on the world map,");
                        ui.label("then press F4.");
                    }
                }
                LauncherPhase::GeneratingMeso => {
                    ui.label("Generating meso tiles...");
                }
                LauncherPhase::MesoView => {
                    if let Some(ref chunk) = selected_chunk {
                        let (cx, cy) = chunk.chunk_coord;
                        ui.label(format!("Macro chunk: ({cx}, {cy})"));
                    }
                    ui.separator();
                    if let Some(ref meso) = selected_meso {
                        let (mx, my) = meso.meso_coord;
                        ui.label(format!("Meso tile: ({mx}, {my})"));
                        ui.label(format!(
                            "World: ({:.0}, {:.0})",
                            meso.origin.x, meso.origin.y
                        ));
                        ui.separator();
                        if ui.button("Generate Micromap").clicked() {
                            commands.insert_resource(LaunchLevelRequest);
                        }
                    } else {
                        ui.label("Click a meso tile to select it.");
                    }
                }
                LauncherPhase::GeneratingMicro => {
                    ui.label("Generating micro tiles...");
                }
                LauncherPhase::MicroView => {
                    if let Some(ref meso) = selected_meso {
                        let (mx, my) = meso.meso_coord;
                        ui.label(format!("Meso tile: ({mx}, {my})"));
                    }
                    ui.separator();
                    if let Some(ref micro) = selected_micro {
                        let (ux, uy) = micro.micro_coord;
                        ui.label(format!("Micro tile: ({ux}, {uy})"));
                        ui.label(format!(
                            "World: ({:.2}, {:.2})",
                            micro.origin.x, micro.origin.y
                        ));
                        ui.separator();
                        if ui.button("Play").clicked() {
                            commands.insert_resource(StartPlayRequest);
                        }
                    } else {
                        ui.label("Click a micro tile to select it.");
                    }

                    // Save Level button + dialog — visible when a micro tile is selected.
                    if selected_micro.is_some() {
                        save_level_ui(ui, &mut commands, &mut save_state);
                    }
                }
                LauncherPhase::Playing => {
                    if playing.is_some() {
                        ui.label("Playing — press ESC to stop");
                    }

                    // Save Level button + dialog — visible during play.
                    if selected_micro.is_some() {
                        save_level_ui(ui, &mut commands, &mut save_state);
                    }
                }
            }
        });
}

/// Shared "Save Level" UI rendered in both MicroView and Playing phases.
fn save_level_ui(
    ui: &mut egui::Ui,
    commands: &mut Commands,
    save_state: &mut SaveLevelUiState,
) {
    ui.separator();

    // Show status message from previous save attempt.
    if let Some((ref msg, is_error)) = save_state.status {
        let color = if is_error {
            egui::Color32::RED
        } else {
            egui::Color32::GREEN
        };
        ui.colored_label(color, msg.as_str());
        ui.add_space(4.0);
    }

    if !save_state.show_dialog {
        if ui.button("Save Level").clicked() {
            save_state.show_dialog = true;
            save_state.status = None;
        }
    } else {
        ui.label("Level tag:");
        ui.text_edit_singleline(&mut save_state.tag_input);

        let tag_valid = !save_state.tag_input.is_empty()
            && save_state.tag_input.chars().all(|c| {
                c.is_ascii_alphanumeric() || c == '-' || c == '_'
            });

        ui.horizontal(|ui| {
            if ui.add_enabled(tag_valid, egui::Button::new("Save")).clicked() {
                commands.insert_resource(SaveLevelRequest {
                    tag: save_state.tag_input.clone(),
                });
                save_state.show_dialog = false;
            }
            if ui.button("Cancel").clicked() {
                save_state.show_dialog = false;
            }
        });

        if !tag_valid && !save_state.tag_input.is_empty() {
            ui.colored_label(
                egui::Color32::YELLOW,
                "Tag: alphanumeric, hyphens, underscores only",
            );
        }
    }
}

/// ESC exits play mode back to meso view (not all the way out).
pub fn escape_to_stop_system(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    existing_level: Option<Res<PlayableLevel>>,
    phase: Option<Res<LauncherPhase>>,
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

        // Go back to micro view if we were playing
        if phase.map_or(false, |p| *p == LauncherPhase::Playing) {
            commands.insert_resource(LauncherPhase::MicroView);
        }

        println!("Exited play mode");
    }
}

/// Clean up when leaving LevelLauncher mode entirely.
pub fn cleanup_on_exit(
    mut commands: Commands,
    existing_level: Option<Res<PlayableLevel>>,
    existing_selected: Option<Res<SelectedChunk>>,
    existing_meso: Option<Res<SelectedMesoTile>>,
    existing_micro: Option<Res<SelectedMicroTile>>,
    level_chunks: Query<Entity, With<LevelChunk>>,
) {
    if existing_level.is_some() {
        commands.remove_resource::<PlayableLevel>();
        commands.remove_resource::<LoadedChunks>();
    }
    if existing_selected.is_some() {
        commands.remove_resource::<SelectedChunk>();
    }
    if existing_meso.is_some() {
        commands.remove_resource::<SelectedMesoTile>();
    }
    if existing_micro.is_some() {
        commands.remove_resource::<SelectedMicroTile>();
    }
    commands.remove_resource::<LauncherPhase>();
    commands.remove_resource::<GenerateMesoRequest>();
    commands.remove_resource::<LaunchLevelRequest>();
    commands.remove_resource::<StartPlayRequest>();
    commands.remove_resource::<SaveLevelRequest>();

    for entity in &level_chunks {
        commands.entity(entity).despawn();
    }
}

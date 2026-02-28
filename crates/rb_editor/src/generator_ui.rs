use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use rb_core::AppMode;
use rb_persistence::{list_worlds, load_world, save_world, world_path};
use rb_world::WorldDefinition;

/// Resource for tracking UI state in the generator.
#[derive(Resource, Default)]
pub struct GeneratorUiState {
    /// Seed input as string for editing.
    pub seed_text: String,
    /// Whether the seed text has been initialized.
    pub initialized: bool,
    /// Show load dialog.
    pub show_load_dialog: bool,
    /// Available world files for loading.
    pub available_worlds: Vec<std::path::PathBuf>,
    /// Status message to display.
    pub status_message: Option<(String, f64)>,
}

/// Resource for signaling world regeneration is needed.
#[derive(Resource, Default)]
pub struct RegenerationRequest {
    pub pending: bool,
}

/// System to render the World Generator UI panel.
pub fn generator_ui_system(
    mut contexts: EguiContexts,
    mut world_def: ResMut<WorldDefinition>,
    mut ui_state: ResMut<GeneratorUiState>,
    mut regen_request: ResMut<RegenerationRequest>,
    current_mode: Res<State<AppMode>>,
    mut next_mode: ResMut<NextState<AppMode>>,
) {
    // Initialize seed text from world definition
    if !ui_state.initialized {
        ui_state.seed_text = world_def.seed.to_string();
        ui_state.initialized = true;
    }

    // Top menu bar (visible in all modes)
    egui::TopBottomPanel::top("mode_bar").show(contexts.ctx_mut(), |ui| {
        ui.horizontal(|ui| {
            for mode in AppMode::all() {
                let is_selected = current_mode.get() == mode;
                let text = format!("{} ({})", mode.name(), format_keycode(mode.shortcut()));

                if ui.selectable_label(is_selected, text).clicked() {
                    next_mode.set(mode.clone());
                }
            }
        });
    });

    // Left panel only in WorldGenerator mode
    if *current_mode.get() != AppMode::WorldGenerator {
        return;
    }

    egui::SidePanel::left("generator_panel")
        .default_width(180.0)
        .show(contexts.ctx_mut(), |ui| {
            ui.heading("World Generator");
            ui.separator();

            // World name
            ui.label("World Name:");
            ui.text_edit_singleline(&mut world_def.name);
            ui.add_space(8.0);

            // Seed
            ui.label("Seed:");
            ui.horizontal(|ui| {
                let response = ui.text_edit_singleline(&mut ui_state.seed_text);
                if response.lost_focus() {
                    if let Ok(new_seed) = ui_state.seed_text.parse::<u32>() {
                        if new_seed != world_def.seed {
                            world_def.seed = new_seed;
                            regen_request.pending = true;
                        }
                    } else {
                        // Reset to current seed on invalid input
                        ui_state.seed_text = world_def.seed.to_string();
                    }
                }
                if ui.button("🎲").on_hover_text("Random seed").clicked() {
                    world_def.seed = rand_seed();
                    ui_state.seed_text = world_def.seed.to_string();
                    regen_request.pending = true;
                }
            });
            ui.add_space(8.0);

            // Regenerate button
            if ui.button("Regenerate Map").clicked() {
                regen_request.pending = true;
            }
            ui.add_space(16.0);

            // Noise Parameters
            ui.collapsing("Noise Parameters", |ui| {
                let params = &mut world_def.noise_params;

                ui.label("Continentalness:");
                let mut cont_octaves = params.continentalness_octaves as i32;
                if ui.add(egui::Slider::new(&mut cont_octaves, 1..=24).text("Octaves")).changed() {
                    params.continentalness_octaves = cont_octaves as u32;
                    regen_request.pending = true;
                }

                if ui.add(egui::Slider::new(&mut params.continentalness_persistence, 0.1..=0.9).text("Persistence")).changed() {
                    regen_request.pending = true;
                }

                if ui.add(egui::Slider::new(&mut params.continentalness_lacunarity, 1.5..=3.0).text("Lacunarity")).changed() {
                    regen_request.pending = true;
                }

                ui.add_space(8.0);
                ui.label("Temperature:");
                let mut temp_octaves = params.temperature_octaves as i32;
                if ui.add(egui::Slider::new(&mut temp_octaves, 1..=16).text("Octaves")).changed() {
                    params.temperature_octaves = temp_octaves as u32;
                    regen_request.pending = true;
                }

                if ui.add(egui::Slider::new(&mut params.temperature_persistence, 0.1..=0.9).text("Persistence")).changed() {
                    regen_request.pending = true;
                }
            });
            ui.add_space(8.0);

            // Sea level
            ui.collapsing("Climate", |ui| {
                if ui.add(egui::Slider::new(&mut world_def.sea_level, -0.5..=0.5).text("Sea Level")).changed() {
                    regen_request.pending = true;
                }
            });
            ui.add_space(16.0);

            ui.separator();

            // View layer selection
            ui.label("View Layer:");
            ui.label("Press SPACE to cycle");
            ui.add_space(16.0);

            ui.separator();

            // Save/Load buttons
            if ui.button("Save World").clicked() {
                let path = world_path(&world_def.name);
                match save_world(&path, &world_def) {
                    Ok(()) => {
                        ui_state.status_message = Some((format!("Saved to {}", path.display()), 3.0));
                        println!("Saved world to {}", path.display());
                    }
                    Err(e) => {
                        ui_state.status_message = Some((format!("Save failed: {}", e), 5.0));
                        eprintln!("Failed to save world: {}", e);
                    }
                }
            }

            if ui.button("Load World...").clicked() {
                ui_state.show_load_dialog = true;
                ui_state.available_worlds = list_worlds().unwrap_or_default();
            }

            // Status message
            if let Some((msg, _)) = &ui_state.status_message {
                ui.add_space(8.0);
                ui.label(msg);
            }
        });

    // Load dialog window
    if ui_state.show_load_dialog {
        let mut close_dialog = false;
        let mut load_path: Option<std::path::PathBuf> = None;

        egui::Window::new("Load World")
            .collapsible(false)
            .resizable(true)
            .show(contexts.ctx_mut(), |ui| {
                ui.label("Select a world to load:");
                ui.separator();

                egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                    for path in &ui_state.available_worlds {
                        let name = path.file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("Unknown");

                        if ui.selectable_label(false, name).clicked() {
                            load_path = Some(path.clone());
                            close_dialog = true;
                        }
                    }

                    if ui_state.available_worlds.is_empty() {
                        ui.label("No saved worlds found.");
                    }
                });

                ui.separator();
                if ui.button("Cancel").clicked() {
                    close_dialog = true;
                }
            });

        if close_dialog {
            ui_state.show_load_dialog = false;
        }

        if let Some(path) = load_path {
            match load_world(&path) {
                Ok(loaded) => {
                    *world_def = loaded;
                    ui_state.seed_text = world_def.seed.to_string();
                    regen_request.pending = true;
                    ui_state.status_message = Some((format!("Loaded {}", path.display()), 3.0));
                    println!("Loaded world from {}", path.display());
                }
                Err(e) => {
                    ui_state.status_message = Some((format!("Load failed: {}", e), 5.0));
                    eprintln!("Failed to load world: {}", e);
                }
            }
        }
    }
}

/// Format a KeyCode for display.
fn format_keycode(key: KeyCode) -> String {
    match key {
        KeyCode::F1 => "F1".to_string(),
        KeyCode::F2 => "F2".to_string(),
        KeyCode::F3 => "F3".to_string(),
        KeyCode::F4 => "F4".to_string(),
        _ => format!("{:?}", key),
    }
}

/// Generate a random seed.
fn rand_seed() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    (duration.as_nanos() & 0xFFFFFFFF) as u32
}

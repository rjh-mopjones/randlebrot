use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use rb_artifacts::LayerManifest;
use rb_core::AppMode;
use rb_noise::{NoiseBackend, NoiseLayer};
use rb_persistence::{list_worlds, load_world, save_world, world_path};
use rb_world::WorldDefinition;

/// Current visualization layer for World Generator mode.
#[derive(Resource)]
pub struct CurrentLayer(pub NoiseLayer);

impl Default for CurrentLayer {
    fn default() -> Self {
        Self(NoiseLayer::Biome)
    }
}

/// Resource for tracking UI state in the generator.
#[derive(Resource)]
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
    /// Current layer for display (synced from CurrentLayer resource).
    pub current_layer: Option<NoiseLayer>,
    /// Layer change requested by UI (read by main.rs to update textures).
    pub layer_changed: Option<NoiseLayer>,
    /// Whether to use GPU for noise generation (defaults to true).
    pub use_gpu: bool,
    /// Whether the Open Artifact dialog is shown.
    pub show_open_artifact_dialog: bool,
    /// Cached list of available layer artifacts (fetched on dialog open).
    pub available_artifacts: Vec<(String, LayerManifest)>,
    /// Error message from listing artifacts.
    pub artifact_list_error: Option<String>,
    /// Whether the Save As Artifact dialog is shown.
    pub show_save_as_dialog: bool,
    /// Tag input for Save As dialog.
    pub save_as_tag_input: String,
    /// Error message for Save As dialog.
    pub save_as_error: Option<String>,
    /// Whether a Save As operation is in progress.
    pub save_as_in_progress: bool,
    /// Whether to confirm overwriting an existing artifact.
    pub save_as_confirm_overwrite: bool,
    /// Whether a generated world is available (AppPhase::Ready). Set by main.rs.
    pub world_ready: bool,
}

impl Default for GeneratorUiState {
    fn default() -> Self {
        Self {
            seed_text: String::new(),
            initialized: false,
            show_load_dialog: false,
            available_worlds: Vec::new(),
            status_message: None,
            current_layer: None,
            layer_changed: None,
            use_gpu: true,
            show_open_artifact_dialog: false,
            available_artifacts: Vec::new(),
            artifact_list_error: None,
            show_save_as_dialog: false,
            save_as_tag_input: String::new(),
            save_as_error: None,
            save_as_in_progress: false,
            save_as_confirm_overwrite: false,
            world_ready: false,
        }
    }
}

/// Signal resource: user selected an artifact to load from the Open dialog.
/// main.rs consumes this and transitions to LoadingArtifact phase.
#[derive(Resource)]
pub struct OpenArtifactRequest {
    /// Tag of the artifact to load.
    pub tag: String,
}

/// Signal resource: user confirmed Save As with a tag name.
/// main.rs consumes this and performs the save.
#[derive(Resource)]
pub struct SaveAsArtifactRequest {
    /// Tag to save under.
    pub tag: String,
}

impl GeneratorUiState {
    /// Get the noise backend based on current settings.
    pub fn backend(&self) -> NoiseBackend {
        if self.use_gpu {
            NoiseBackend::Gpu
        } else {
            NoiseBackend::Cpu
        }
    }
}

/// Resource for signaling world regeneration is needed.
#[derive(Resource, Default)]
pub struct RegenerationRequest {
    pub pending: bool,
}

/// System to render the top mode bar (runs in all modes).
pub fn mode_bar_system(
    mut contexts: EguiContexts,
    current_mode: Res<State<AppMode>>,
    mut next_mode: ResMut<NextState<AppMode>>,
) {
    egui::TopBottomPanel::top("mode_bar").show(contexts.ctx_mut().unwrap(), |ui| {
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
}

/// System to render the World Generator terrain panel and load dialog.
pub fn terrain_panel_system(
    mut contexts: EguiContexts,
    mut world_def: ResMut<WorldDefinition>,
    mut ui_state: ResMut<GeneratorUiState>,
    mut regen_request: ResMut<RegenerationRequest>,
    mut commands: Commands,
) {
    // Initialize seed text from world definition
    if !ui_state.initialized {
        ui_state.seed_text = world_def.seed.to_string();
        ui_state.initialized = true;
    }

    egui::SidePanel::left("generator_panel")
        .default_width(180.0)
        .show(contexts.ctx_mut().unwrap(), |ui| {
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
            ui.add_space(8.0);

            // GPU acceleration toggle
            let gpu_available = NoiseBackend::gpu_available();
            ui.horizontal(|ui| {
                let checkbox = ui.checkbox(&mut ui_state.use_gpu, "GPU Acceleration");
                if !gpu_available {
                    ui_state.use_gpu = false;
                    checkbox.on_hover_text("GPU not available, using CPU");
                } else {
                    checkbox.on_hover_text("Use GPU compute shaders for faster noise generation");
                }
            });
            if gpu_available && ui_state.use_gpu {
                ui.label(egui::RichText::new("GPU enabled").small().color(egui::Color32::GREEN));
            } else if !gpu_available {
                ui.label(egui::RichText::new("GPU unavailable").small().color(egui::Color32::GRAY));
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

            ui.add_space(16.0);
            ui.separator();

            // Artifact Open/Save As buttons
            ui.heading("Artifacts");
            ui.add_space(4.0);

            if ui.button("Open Artifact...").clicked() {
                ui_state.show_open_artifact_dialog = true;
                ui_state.artifact_list_error = None;
                // Fetch the artifact list from disk.
                match rb_artifacts::ArtifactStore::new() {
                    Ok(store) => match store.list_layers() {
                        Ok(list) => ui_state.available_artifacts = list,
                        Err(e) => {
                            ui_state.artifact_list_error =
                                Some(format!("Failed to list artifacts: {e}"));
                            ui_state.available_artifacts = Vec::new();
                        }
                    },
                    Err(e) => {
                        ui_state.artifact_list_error =
                            Some(format!("Failed to open artifact store: {e}"));
                        ui_state.available_artifacts = Vec::new();
                    }
                }
            }

            let save_as_enabled = ui_state.world_ready;
            ui.add_enabled_ui(save_as_enabled, |ui| {
                if ui
                    .button("Save As Artifact...")
                    .on_disabled_hover_text("Generate a world first")
                    .clicked()
                {
                    ui_state.show_save_as_dialog = true;
                    ui_state.save_as_error = None;
                    ui_state.save_as_in_progress = false;
                    ui_state.save_as_confirm_overwrite = false;
                    // Pre-fill with sanitised world name.
                    ui_state.save_as_tag_input = sanitize_tag_for_ui(&world_def.name);
                }
            });

            // Status message
            if let Some((msg, _)) = &ui_state.status_message {
                ui.add_space(8.0);
                ui.label(msg);
            }

            ui.add_space(16.0);
            ui.separator();

            // View layer selection (only shown if CurrentLayer exists)
            if let Some(ref mut current_layer) = ui_state.current_layer {
                ui.heading("View Layer");

                let current = *current_layer;
                egui::ComboBox::from_label("Layer")
                    .selected_text(current.name())
                    .show_ui(ui, |ui| {
                        // Biome (default) at top
                        if ui.selectable_label(current == NoiseLayer::Biome, NoiseLayer::Biome.name()).clicked() {
                            ui_state.layer_changed = Some(NoiseLayer::Biome);
                        }

                        // Group layers by category
                        let categories = ["Base", "Terrain", "Climate", "Hydrology", "Ecology"];
                        for &cat in &categories {
                            ui.separator();
                            ui.label(cat);
                            for &layer in NoiseLayer::all() {
                                if layer == NoiseLayer::Biome { continue; }
                                if layer.category() == cat {
                                    if ui.selectable_label(current == layer, layer.name()).clicked() {
                                        ui_state.layer_changed = Some(layer);
                                    }
                                }
                            }
                        }
                    });
            }
        });

    // Load dialog window
    if ui_state.show_load_dialog {
        let mut close_dialog = false;
        let mut load_path: Option<std::path::PathBuf> = None;

        egui::Window::new("Load World")
            .collapsible(false)
            .resizable(true)
            .show(contexts.ctx_mut().unwrap(), |ui| {
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

    // Open Artifact dialog window
    if ui_state.show_open_artifact_dialog {
        let mut close_dialog = false;
        let mut selected_tag: Option<String> = None;

        egui::Window::new("Open Layer Artifact")
            .collapsible(false)
            .resizable(true)
            .default_size([420.0, 300.0])
            .show(contexts.ctx_mut().unwrap(), |ui| {
                ui.label("Select a layer artifact to load:");
                ui.add_space(4.0);

                if let Some(ref err) = ui_state.artifact_list_error {
                    ui.label(
                        egui::RichText::new(err)
                            .color(egui::Color32::from_rgb(255, 100, 100))
                            .size(12.0),
                    );
                    ui.add_space(8.0);
                }

                ui.separator();

                egui::ScrollArea::vertical()
                    .max_height(220.0)
                    .show(ui, |ui| {
                        if ui_state.available_artifacts.is_empty()
                            && ui_state.artifact_list_error.is_none()
                        {
                            ui.label("No layer artifacts found.");
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(
                                    "Generate a world via CLI or editor to create one.",
                                )
                                .size(11.0)
                                .color(egui::Color32::GRAY),
                            );
                        }

                        for (tag, manifest) in &ui_state.available_artifacts {
                            ui.horizontal(|ui| {
                                let label = format!(
                                    "{tag}  (seed: {}, {}x{}, {})",
                                    manifest.seed,
                                    manifest.world_width,
                                    manifest.world_height,
                                    &manifest.created[..10.min(manifest.created.len())],
                                );
                                if ui.selectable_label(false, label).clicked() {
                                    selected_tag = Some(tag.clone());
                                    close_dialog = true;
                                }
                            });
                        }
                    });

                ui.separator();
                if ui.button("Cancel").clicked() {
                    close_dialog = true;
                }
            });

        if close_dialog {
            ui_state.show_open_artifact_dialog = false;
        }

        if let Some(tag) = selected_tag {
            commands.insert_resource(OpenArtifactRequest { tag });
            ui_state.show_open_artifact_dialog = false;
        }
    }

    // Save As Artifact dialog window
    if ui_state.show_save_as_dialog {
        let mut close_dialog = false;
        let mut do_save = false;

        egui::Window::new("Save As Layer Artifact")
            .collapsible(false)
            .resizable(false)
            .default_size([380.0, 160.0])
            .show(contexts.ctx_mut().unwrap(), |ui| {
                ui.vertical(|ui| {
                    ui.label("Save the current world as a layer artifact.");
                    ui.add_space(8.0);

                    ui.horizontal(|ui| {
                        ui.label("Tag:");
                        ui.text_edit_singleline(&mut ui_state.save_as_tag_input);
                    });
                    ui.add_space(4.0);

                    ui.label(
                        egui::RichText::new(
                            "Letters, numbers, hyphens, and underscores only.",
                        )
                        .size(11.0)
                        .color(egui::Color32::GRAY),
                    );

                    if let Some(ref err) = ui_state.save_as_error {
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new(err)
                                .color(egui::Color32::from_rgb(255, 100, 100))
                                .size(12.0),
                        );
                    }

                    if ui_state.save_as_confirm_overwrite {
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new(format!(
                                "Artifact '{}' already exists. Overwrite?",
                                ui_state.save_as_tag_input.trim()
                            ))
                            .color(egui::Color32::from_rgb(255, 200, 80))
                            .size(12.0),
                        );
                        ui.horizontal(|ui| {
                            if ui.button("Overwrite").clicked() {
                                do_save = true;
                                ui_state.save_as_confirm_overwrite = false;
                            }
                            if ui.button("Cancel").clicked() {
                                ui_state.save_as_confirm_overwrite = false;
                            }
                        });
                    } else {
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            let can_save = !ui_state.save_as_in_progress;
                            ui.add_enabled_ui(can_save, |ui| {
                                if ui.button("Save").clicked() {
                                    let tag = ui_state.save_as_tag_input.trim().to_string();
                                    // Validate tag
                                    if tag.is_empty() {
                                        ui_state.save_as_error =
                                            Some("Tag must not be empty.".to_string());
                                    } else if !tag.chars().all(|c| {
                                        c.is_ascii_alphanumeric() || c == '-' || c == '_'
                                    }) {
                                        ui_state.save_as_error = Some(
                                            "Tag must contain only letters, numbers, hyphens, and underscores.".to_string(),
                                        );
                                    } else {
                                        // Check if it already exists
                                        let exists = rb_artifacts::ArtifactStore::new()
                                            .map(|s| {
                                                s.exists(rb_artifacts::ArtifactKind::Layers, &tag)
                                            })
                                            .unwrap_or(false);
                                        if exists {
                                            ui_state.save_as_confirm_overwrite = true;
                                            ui_state.save_as_error = None;
                                        } else {
                                            do_save = true;
                                        }
                                    }
                                }
                            });
                            if ui.button("Cancel").clicked() {
                                close_dialog = true;
                            }
                        });
                    }

                    if ui_state.save_as_in_progress {
                        ui.add_space(4.0);
                        ui.label("Saving...");
                    }
                });
            });

        if close_dialog {
            ui_state.show_save_as_dialog = false;
            ui_state.save_as_error = None;
            ui_state.save_as_confirm_overwrite = false;
        }

        if do_save {
            let tag = ui_state.save_as_tag_input.trim().to_string();
            ui_state.save_as_in_progress = true;
            ui_state.save_as_error = None;
            commands.insert_resource(SaveAsArtifactRequest { tag });
        }
    }
}

/// Sanitise a string into a valid artifact tag for the UI.
fn sanitize_tag_for_ui(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else if c == ' ' {
                '-'
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "world".to_string()
    } else {
        sanitized
    }
}

/// Format a KeyCode for display.
pub(crate) fn format_keycode(key: KeyCode) -> String {
    match key {
        KeyCode::F1 => "F1".to_string(),
        KeyCode::F2 => "F2".to_string(),
        KeyCode::F3 => "F3".to_string(),
        KeyCode::F4 => "F4".to_string(),
        _ => format!("{:?}", key),
    }
}

/// Generate a random seed.
pub(crate) fn rand_seed() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    (duration.as_nanos() & 0xFFFFFFFF) as u32
}

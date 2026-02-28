use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::{egui, EguiContexts};
use rb_core::AppMode;
use rb_world::{City, CityTier, Landmark, LandmarkKind, Point2D, WorldDefinition, WorldIdGenerator};

/// Currently selected editor tool.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum EditorTool {
    #[default]
    Select,
    PlaceCity,
    PlaceLandmark,
    DrawRegion,
}

/// Currently selected object in the editor.
#[derive(Resource, Default)]
pub struct EditorSelection {
    pub city_id: Option<u32>,
    pub landmark_id: Option<u32>,
    pub region_id: Option<u32>,
}

/// State for city placement.
#[derive(Resource, Default)]
pub struct CityPlacementState {
    pub name: String,
    pub tier: CityTier,
}

/// State for landmark placement.
#[derive(Resource, Default)]
pub struct LandmarkPlacementState {
    pub name: String,
    pub kind: LandmarkKind,
}

/// System to render the World Map Editor UI panel.
pub fn map_editor_ui_system(
    mut contexts: EguiContexts,
    mut world_def: ResMut<WorldDefinition>,
    mut current_tool: ResMut<EditorTool>,
    mut selection: ResMut<EditorSelection>,
    mut city_state: ResMut<CityPlacementState>,
    mut landmark_state: ResMut<LandmarkPlacementState>,
    current_mode: Res<State<AppMode>>,
) {
    // Only show in World Map Editor mode
    if *current_mode.get() != AppMode::WorldMapEditor {
        return;
    }

    egui::SidePanel::left("map_editor_panel")
        .default_width(180.0)
        .show(contexts.ctx_mut(), |ui| {
            ui.heading("Map Editor");
            ui.separator();

            // Tools
            ui.label("Tools:");
            ui.horizontal_wrapped(|ui| {
                if ui.selectable_label(*current_tool == EditorTool::Select, "Select").clicked() {
                    *current_tool = EditorTool::Select;
                }
                if ui.selectable_label(*current_tool == EditorTool::PlaceCity, "City").clicked() {
                    *current_tool = EditorTool::PlaceCity;
                }
                if ui.selectable_label(*current_tool == EditorTool::PlaceLandmark, "Landmark").clicked() {
                    *current_tool = EditorTool::PlaceLandmark;
                }
                if ui.selectable_label(*current_tool == EditorTool::DrawRegion, "Region").clicked() {
                    *current_tool = EditorTool::DrawRegion;
                }
            });
            ui.add_space(8.0);

            // Tool-specific options
            match *current_tool {
                EditorTool::PlaceCity => {
                    ui.separator();
                    ui.label("New City:");
                    ui.text_edit_singleline(&mut city_state.name);

                    ui.label("Tier:");
                    egui::ComboBox::from_id_salt("city_tier")
                        .selected_text(city_state.tier.name())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut city_state.tier, CityTier::Capital, "Capital");
                            ui.selectable_value(&mut city_state.tier, CityTier::Town, "Town");
                            ui.selectable_value(&mut city_state.tier, CityTier::Village, "Village");
                        });

                    ui.add_space(4.0);
                    ui.label("Click on map to place");
                }
                EditorTool::PlaceLandmark => {
                    ui.separator();
                    ui.label("New Landmark:");
                    ui.text_edit_singleline(&mut landmark_state.name);

                    ui.label("Type:");
                    egui::ComboBox::from_id_salt("landmark_kind")
                        .selected_text(landmark_state.kind.name())
                        .show_ui(ui, |ui| {
                            for kind in LandmarkKind::all() {
                                ui.selectable_value(&mut landmark_state.kind, *kind, kind.name());
                            }
                        });

                    ui.add_space(4.0);
                    ui.label("Click on map to place");
                }
                EditorTool::DrawRegion => {
                    ui.separator();
                    ui.label("Region drawing:");
                    ui.label("Click to add vertices");
                    ui.label("Double-click to close");
                    ui.add_space(4.0);
                    ui.label("(Not yet implemented)");
                }
                EditorTool::Select => {
                    // Show selected object properties
                    if let Some(city_id) = selection.city_id {
                        if let Some(city) = world_def.cities.iter_mut().find(|c| c.id == city_id) {
                            ui.separator();
                            ui.label("Selected City:");
                            ui.text_edit_singleline(&mut city.name);

                            egui::ComboBox::from_id_salt("edit_city_tier")
                                .selected_text(city.tier.name())
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut city.tier, CityTier::Capital, "Capital");
                                    ui.selectable_value(&mut city.tier, CityTier::Town, "Town");
                                    ui.selectable_value(&mut city.tier, CityTier::Village, "Village");
                                });

                            ui.label(format!("Position: ({:.0}, {:.0})", city.position.x, city.position.y));

                            if ui.button("Delete").clicked() {
                                let id = city_id;
                                world_def.cities.retain(|c| c.id != id);
                                selection.city_id = None;
                            }
                        }
                    } else if let Some(landmark_id) = selection.landmark_id {
                        if let Some(landmark) = world_def.landmarks.iter_mut().find(|l| l.id == landmark_id) {
                            ui.separator();
                            ui.label("Selected Landmark:");
                            ui.text_edit_singleline(&mut landmark.name);

                            egui::ComboBox::from_id_salt("edit_landmark_kind")
                                .selected_text(landmark.kind.name())
                                .show_ui(ui, |ui| {
                                    for kind in LandmarkKind::all() {
                                        ui.selectable_value(&mut landmark.kind, *kind, kind.name());
                                    }
                                });

                            ui.label(format!("Position: ({:.0}, {:.0})", landmark.position.x, landmark.position.y));

                            if ui.button("Delete").clicked() {
                                let id = landmark_id;
                                world_def.landmarks.retain(|l| l.id != id);
                                selection.landmark_id = None;
                            }
                        }
                    } else {
                        ui.label("Click to select");
                    }
                }
            }

            ui.add_space(16.0);
            ui.separator();

            // Object lists
            ui.collapsing(format!("Cities ({})", world_def.cities.len()), |ui| {
                for city in &world_def.cities {
                    let selected = selection.city_id == Some(city.id);
                    let label = format!("{} ({})", city.name, city.tier.name());
                    if ui.selectable_label(selected, label).clicked() {
                        selection.city_id = Some(city.id);
                        selection.landmark_id = None;
                        selection.region_id = None;
                        *current_tool = EditorTool::Select;
                    }
                }
            });

            ui.collapsing(format!("Landmarks ({})", world_def.landmarks.len()), |ui| {
                for landmark in &world_def.landmarks {
                    let selected = selection.landmark_id == Some(landmark.id);
                    let label = format!("{} ({})", landmark.name, landmark.kind.name());
                    if ui.selectable_label(selected, label).clicked() {
                        selection.landmark_id = Some(landmark.id);
                        selection.city_id = None;
                        selection.region_id = None;
                        *current_tool = EditorTool::Select;
                    }
                }
            });

            ui.collapsing(format!("Regions ({})", world_def.regions.len()), |ui| {
                for region in &world_def.regions {
                    let selected = selection.region_id == Some(region.id);
                    if ui.selectable_label(selected, &region.name).clicked() {
                        selection.region_id = Some(region.id);
                        selection.city_id = None;
                        selection.landmark_id = None;
                        *current_tool = EditorTool::Select;
                    }
                }
            });
        });
}

/// System to handle mouse clicks for placing objects.
pub fn map_editor_click_system(
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_query: Query<(&Camera, &GlobalTransform)>,
    current_mode: Res<State<AppMode>>,
    current_tool: Res<EditorTool>,
    mut world_def: ResMut<WorldDefinition>,
    mut id_gen: ResMut<WorldIdGenerator>,
    city_state: Res<CityPlacementState>,
    landmark_state: Res<LandmarkPlacementState>,
    mut contexts: EguiContexts,
) {
    // Only process in World Map Editor mode
    if *current_mode.get() != AppMode::WorldMapEditor {
        return;
    }

    // Skip if clicking on egui
    if contexts.ctx_mut().is_pointer_over_area() {
        return;
    }

    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }

    // Get cursor position
    let Ok(window) = windows.get_single() else { return };
    let Some(cursor_pos) = window.cursor_position() else { return };
    let Ok((camera, camera_transform)) = camera_query.get_single() else { return };

    // Convert screen to world coordinates
    let Ok(world_pos) = camera.viewport_to_world_2d(camera_transform, cursor_pos) else { return };

    // Convert to map coordinates (map is centered at origin)
    let map_x = world_pos.x + (world_def.width as f32 / 2.0);
    let map_y = (world_def.height as f32 / 2.0) - world_pos.y; // Flip Y

    // Check bounds
    if map_x < 0.0 || map_x >= world_def.width as f32 || map_y < 0.0 || map_y >= world_def.height as f32 {
        return;
    }

    let position = Point2D::new(map_x as f64, map_y as f64);

    match *current_tool {
        EditorTool::PlaceCity => {
            let name = if city_state.name.is_empty() {
                format!("City {}", world_def.cities.len() + 1)
            } else {
                city_state.name.clone()
            };

            let city = City::new(id_gen.next_city_id(), name, position, city_state.tier);
            world_def.cities.push(city);
            println!("Placed city at ({:.0}, {:.0})", map_x, map_y);
        }
        EditorTool::PlaceLandmark => {
            let name = if landmark_state.name.is_empty() {
                format!("Landmark {}", world_def.landmarks.len() + 1)
            } else {
                landmark_state.name.clone()
            };

            let landmark = Landmark::new(id_gen.next_landmark_id(), name, position, landmark_state.kind);
            world_def.landmarks.push(landmark);
            println!("Placed landmark at ({:.0}, {:.0})", map_x, map_y);
        }
        _ => {}
    }
}

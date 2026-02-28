use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::{egui, EguiContexts};
use rb_core::AppMode;
use rb_world::{SelectedChunk, WorldDefinition};

// WorldDefinition is used in chunk_selection_system

/// Chunk editor tool.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChunkTool {
    #[default]
    Select,
    PaintTile,
    PlaceBuilding,
    PlaceNpc,
}

/// State for the chunk editor.
#[derive(Resource, Default)]
pub struct ChunkEditorState {
    /// Currently selected tile type for painting.
    pub selected_tile: usize,
    /// Show tile grid overlay.
    pub show_grid: bool,
}

/// System to render the Chunk Editor UI panel.
pub fn chunk_editor_ui_system(
    mut contexts: EguiContexts,
    selected_chunk: Res<SelectedChunk>,
    mut current_tool: ResMut<ChunkTool>,
    mut state: ResMut<ChunkEditorState>,
    current_mode: Res<State<AppMode>>,
) {
    // Only show in Chunk Editor mode
    if *current_mode.get() != AppMode::ChunkEditor {
        return;
    }

    egui::SidePanel::left("chunk_editor_panel")
        .default_width(180.0)
        .show(contexts.ctx_mut(), |ui| {
            ui.heading("Chunk Editor");
            ui.separator();

            // Show selected chunk info
            if let Some((cx, cy)) = selected_chunk.coord {
                ui.label(format!("Chunk: ({}, {})", cx, cy));

                // Calculate world position of chunk
                let chunk_size = 64; // tiles per chunk
                let world_x = cx * chunk_size;
                let world_y = cy * chunk_size;
                ui.label(format!("World pos: ({}, {})", world_x, world_y));
            } else {
                ui.label("No chunk selected");
                ui.add_space(8.0);
                ui.label("Click on world map");
                ui.label("to select a chunk");
                ui.add_space(8.0);
                ui.label("Or press F2 for");
                ui.label("Map Editor mode");
            }

            ui.add_space(16.0);
            ui.separator();

            // Tools
            ui.label("Tools:");
            ui.horizontal_wrapped(|ui| {
                if ui.selectable_label(*current_tool == ChunkTool::Select, "Select").clicked() {
                    *current_tool = ChunkTool::Select;
                }
                if ui.selectable_label(*current_tool == ChunkTool::PaintTile, "Tile").clicked() {
                    *current_tool = ChunkTool::PaintTile;
                }
                if ui.selectable_label(*current_tool == ChunkTool::PlaceBuilding, "Build").clicked() {
                    *current_tool = ChunkTool::PlaceBuilding;
                }
                if ui.selectable_label(*current_tool == ChunkTool::PlaceNpc, "NPC").clicked() {
                    *current_tool = ChunkTool::PlaceNpc;
                }
            });

            ui.add_space(8.0);

            // Tool-specific UI
            match *current_tool {
                ChunkTool::PaintTile => {
                    ui.separator();
                    ui.label("Tile Palette:");
                    ui.add_space(4.0);

                    // Simple tile palette
                    let tiles = ["Grass", "Stone", "Sand", "Water", "Road"];
                    for (i, name) in tiles.iter().enumerate() {
                        if ui.selectable_label(state.selected_tile == i, *name).clicked() {
                            state.selected_tile = i;
                        }
                    }

                    ui.add_space(8.0);
                    ui.label("(Painting not yet implemented)");
                }
                ChunkTool::PlaceBuilding => {
                    ui.separator();
                    ui.label("Building Types:");
                    ui.label("• House");
                    ui.label("• Shop");
                    ui.label("• Tavern");
                    ui.label("• Tower");
                    ui.add_space(8.0);
                    ui.label("(Not yet implemented)");
                }
                ChunkTool::PlaceNpc => {
                    ui.separator();
                    ui.label("NPC Spawn:");
                    ui.label("(Not yet implemented)");
                }
                ChunkTool::Select => {}
            }

            ui.add_space(16.0);
            ui.separator();

            // View options
            ui.checkbox(&mut state.show_grid, "Show grid");

            ui.add_space(16.0);
            ui.separator();

            // Quick actions
            if ui.button("Back to Map (F2)").clicked() {
                // Note: This just prints - actual mode switch is handled by F-key system
                println!("Use F2 to switch to Map Editor mode");
            }

            if ui.button("Test Play (F4)").clicked() {
                println!("Use F4 to switch to Level Launcher mode");
            }
        });
}

/// System to handle chunk selection clicks (in Map Editor or Chunk Editor mode).
pub fn chunk_selection_system(
    mouse: Res<ButtonInput<MouseButton>>,
    keyboard: Res<ButtonInput<KeyCode>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_query: Query<(&Camera, &GlobalTransform)>,
    world_def: Res<WorldDefinition>,
    mut selected_chunk: ResMut<SelectedChunk>,
    current_mode: Res<State<AppMode>>,
    mut contexts: EguiContexts,
) {
    // Only process in Map Editor or Chunk Editor mode with Ctrl held
    let in_map_editor = *current_mode.get() == AppMode::WorldMapEditor;
    let in_chunk_editor = *current_mode.get() == AppMode::ChunkEditor;

    if !in_map_editor && !in_chunk_editor {
        return;
    }

    // Skip if clicking on egui
    if contexts.ctx_mut().is_pointer_over_area() {
        return;
    }

    // Require Ctrl+Click for chunk selection in Map Editor, regular click in Chunk Editor
    let should_select = if in_map_editor {
        mouse.just_pressed(MouseButton::Left) && keyboard.pressed(KeyCode::ControlLeft)
    } else {
        mouse.just_pressed(MouseButton::Right) // Right-click to select in chunk editor
    };

    if !should_select {
        return;
    }

    // Get cursor position
    let Ok(window) = windows.get_single() else { return };
    let Some(cursor_pos) = window.cursor_position() else { return };
    let Ok((camera, camera_transform)) = camera_query.get_single() else { return };

    // Convert screen to world coordinates
    let Ok(world_pos) = camera.viewport_to_world_2d(camera_transform, cursor_pos) else { return };

    // Convert to map coordinates
    let map_x = world_pos.x + (world_def.width as f32 / 2.0);
    let map_y = (world_def.height as f32 / 2.0) - world_pos.y;

    // Check bounds
    if map_x < 0.0 || map_x >= world_def.width as f32 || map_y < 0.0 || map_y >= world_def.height as f32 {
        return;
    }

    // Convert to chunk coordinates (assuming each pixel = 1 tile, 64 tiles per chunk)
    let chunk_size = 64;
    let chunk_x = (map_x as i32) / chunk_size;
    let chunk_y = (map_y as i32) / chunk_size;

    selected_chunk.coord = Some((chunk_x, chunk_y));
    println!("Selected chunk ({}, {})", chunk_x, chunk_y);
}

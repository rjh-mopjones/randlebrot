use bevy::prelude::*;
use bevy_egui::EguiPlugin;
use rb_core::AppMode;

pub mod chunk_editor_ui;
pub mod generator_ui;
pub mod launcher_ui;
pub mod map_editor_ui;
pub mod world_overlay;

pub use chunk_editor_ui::{ChunkEditorState, ChunkTool};
pub use generator_ui::{CurrentLayer, GeneratorUiState, RegenerationRequest};
pub use launcher_ui::LauncherState;
pub use map_editor_ui::{CityPlacementState, EditorSelection, EditorTool, LandmarkPlacementState};
pub use world_overlay::OverlaySettings;

/// Editor plugin for Randlebrot.
/// Provides egui-based authoring tools and debug overlays.
pub struct RbEditorPlugin;

impl Plugin for RbEditorPlugin {
    fn build(&self, app: &mut App) {
        // Only add EguiPlugin if not already added
        if !app.is_plugin_added::<EguiPlugin>() {
            app.add_plugins(EguiPlugin);
        }

        app
            // Generator resources
            .init_resource::<GeneratorUiState>()
            .init_resource::<RegenerationRequest>()
            // Map editor resources
            .init_resource::<EditorTool>()
            .init_resource::<EditorSelection>()
            .init_resource::<CityPlacementState>()
            .init_resource::<LandmarkPlacementState>()
            .init_resource::<OverlaySettings>()
            // Chunk editor resources
            .init_resource::<ChunkTool>()
            .init_resource::<ChunkEditorState>()
            // Launcher resources
            .init_resource::<LauncherState>()
            // Generator UI (runs in all modes for the top bar)
            .add_systems(Update, generator_ui::generator_ui_system)
            // Map editor systems
            .add_systems(Update, (
                map_editor_ui::map_editor_ui_system,
                map_editor_ui::map_editor_click_system,
            ).run_if(in_state(AppMode::WorldMapEditor)))
            // Overlay systems
            .add_systems(OnEnter(AppMode::WorldMapEditor), world_overlay::spawn_overlays)
            .add_systems(OnExit(AppMode::WorldMapEditor), world_overlay::despawn_overlays)
            .add_systems(Update, world_overlay::update_overlays.run_if(in_state(AppMode::WorldMapEditor)))
            // Chunk editor systems
            .add_systems(Update, (
                chunk_editor_ui::chunk_editor_ui_system,
                chunk_editor_ui::chunk_selection_system,
            ).run_if(in_state(AppMode::ChunkEditor)))
            // Also allow chunk selection in map editor mode (Ctrl+Click)
            .add_systems(Update, chunk_editor_ui::chunk_selection_system.run_if(in_state(AppMode::WorldMapEditor)))
            // Launcher systems
            .add_systems(Update, (
                launcher_ui::launcher_ui_system,
                launcher_ui::spawn_test_player,
                launcher_ui::player_movement_system,
                launcher_ui::escape_to_stop_system,
            ).run_if(in_state(AppMode::LevelLauncher)))
            .add_systems(OnExit(AppMode::LevelLauncher), launcher_ui::despawn_test_player);
    }
}

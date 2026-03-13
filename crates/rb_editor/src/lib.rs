use bevy::prelude::*;
use bevy_egui::EguiPlugin;
use rb_core::AppMode;

pub mod generator_ui;
pub mod launcher_ui;

pub use generator_ui::{CurrentLayer, GeneratorUiState, RegenerationRequest};
pub use launcher_ui::{GenerateMesoRequest, LaunchLevelRequest, LauncherPhase};

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
            // Generator UI (runs in all modes for the top bar)
            .add_systems(Update, generator_ui::generator_ui_system)
            // Launcher systems
            .add_systems(Update, (
                launcher_ui::launcher_ui_system,
                launcher_ui::escape_to_stop_system,
            ).run_if(in_state(AppMode::LevelLauncher)))
            .add_systems(OnExit(AppMode::LevelLauncher), launcher_ui::cleanup_on_exit);
    }
}

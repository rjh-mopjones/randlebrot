use bevy::prelude::*;
use bevy_egui::EguiPlugin;
use rb_core::AppMode;

pub mod civ_ui;
pub mod generator_ui;
pub mod launcher_ui;
pub mod scene_ui;

pub use generator_ui::{CurrentLayer, GeneratorUiState, OpenArtifactRequest, RegenerationRequest, SaveAsArtifactRequest};
pub use launcher_ui::{GenerateMesoRequest, LaunchLevelRequest, LauncherPhase, SaveLevelRequest, SaveLevelUiState, StartPlayRequest};
pub use civ_ui::{CivUiState, CurrentLifeGenLayer, LifeGenLayer, OverlayState, RegenerateLifeGenRequest};
pub use scene_ui::{CurrentSceneLayer, SceneLayer};

/// Editor plugin for Randlebrot.
/// Provides egui-based authoring tools and debug overlays.
pub struct RbEditorPlugin;

impl Plugin for RbEditorPlugin {
    fn build(&self, app: &mut App) {
        // Only add EguiPlugin if not already added
        if !app.is_plugin_added::<EguiPlugin>() {
            app.add_plugins(EguiPlugin {
                enable_multipass_for_primary_context: false,
                ..Default::default()
            });
        }

        app
            // Generator resources
            .init_resource::<GeneratorUiState>()
            .init_resource::<RegenerationRequest>()
            // Civ generator resources
            .init_resource::<CivUiState>()
            .init_resource::<CurrentLifeGenLayer>()
            .init_resource::<OverlayState>()
            .init_resource::<RegenerateLifeGenRequest>()
            // Launcher save-level UI state
            .init_resource::<SaveLevelUiState>()
            // Scene inspector resources
            .init_resource::<CurrentSceneLayer>()
            // Mode bar (runs in all modes) — must run before side panels
            // so egui allocates the TopBottomPanel before SidePanels.
            .add_systems(Update, generator_ui::mode_bar_system)
            // Terrain panel (runs only in WorldGenerator mode)
            .add_systems(Update, generator_ui::terrain_panel_system
                .run_if(in_state(AppMode::WorldGenerator))
                .after(generator_ui::mode_bar_system))
            // Civ panel (runs only in CivGenerator mode)
            .add_systems(Update, civ_ui::civ_panel_system
                .run_if(in_state(AppMode::CivGenerator))
                .after(generator_ui::mode_bar_system))
            // Scene panel (runs only in SceneInspector mode)
            .add_systems(Update, scene_ui::scene_panel_system
                .run_if(in_state(AppMode::SceneInspector))
                .after(generator_ui::mode_bar_system))
            // Launcher systems
            .add_systems(Update, (
                launcher_ui::launcher_ui_system,
                launcher_ui::escape_to_stop_system,
            ).run_if(in_state(AppMode::LevelLauncher))
             .after(generator_ui::mode_bar_system))
            .add_systems(OnExit(AppMode::LevelLauncher), launcher_ui::cleanup_on_exit);
    }
}

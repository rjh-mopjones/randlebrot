use bevy::prelude::*;
use bevy_egui::EguiPlugin;

/// Editor plugin for Randlebrot.
/// Provides egui-based authoring tools and debug overlays.
pub struct RbEditorPlugin;

impl Plugin for RbEditorPlugin {
    fn build(&self, app: &mut App) {
        // Only add EguiPlugin if not already added
        if !app.is_plugin_added::<EguiPlugin>() {
            app.add_plugins(EguiPlugin);
        }
        // Editor UI systems will be added in future iterations.
    }
}

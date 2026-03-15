use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

/// Layers available in the Scene Inspector view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SceneLayer {
    Ground,
    Structures,
    Roads,
    Districts,
    Detail,
    Composite,
}

impl SceneLayer {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Ground => "Ground",
            Self::Structures => "Structures",
            Self::Roads => "Roads",
            Self::Districts => "Districts",
            Self::Detail => "Detail",
            Self::Composite => "Composite",
        }
    }

    pub fn all() -> &'static [SceneLayer] {
        &[
            Self::Ground,
            Self::Structures,
            Self::Roads,
            Self::Districts,
            Self::Detail,
            Self::Composite,
        ]
    }
}

/// Current scene layer for Scene Inspector mode.
#[derive(Resource)]
pub struct CurrentSceneLayer(pub SceneLayer);

impl Default for CurrentSceneLayer {
    fn default() -> Self {
        Self(SceneLayer::Composite)
    }
}

/// System to render the Scene Inspector UI panel.
pub fn scene_panel_system(
    mut contexts: EguiContexts,
) {
    egui::SidePanel::left("scene_panel")
        .default_width(180.0)
        .show(contexts.ctx_mut(), |ui| {
            ui.heading("Scene Inspector");
            ui.separator();

            ui.label("Zoom to micro level to inspect scenes");
            ui.add_space(16.0);

            // SceneLayer picker (disabled stub)
            ui.add_enabled_ui(false, |ui| {
                let mut current = SceneLayer::Composite;
                egui::ComboBox::from_label("Layer")
                    .selected_text(current.name())
                    .show_ui(ui, |ui| {
                        for layer in SceneLayer::all() {
                            ui.selectable_value(&mut current, *layer, layer.name());
                        }
                    });
            });

            ui.add_space(16.0);
            ui.label(
                egui::RichText::new("Scene generation not yet implemented")
                    .small()
                    .color(egui::Color32::GRAY),
            );
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_layers_have_unique_names() {
        let names: Vec<_> = SceneLayer::all().iter().map(|l| l.name()).collect();
        let unique: std::collections::HashSet<_> = names.iter().collect();
        assert_eq!(names.len(), unique.len());
    }
}

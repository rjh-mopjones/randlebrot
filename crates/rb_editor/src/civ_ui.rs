use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use rb_world::{LifeGenData, WorldDefinition};

use crate::generator_ui::rand_seed;

/// Layers available in the Civilization Generator view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LifeGenLayer {
    Habitability,
    NavigationCost,
    ResourceDesirability,
    Provinces,
    Factions,
    PoliticalState,
    Prosperity,
    Settlements,
}

impl LifeGenLayer {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Habitability => "Habitability",
            Self::NavigationCost => "Navigation Cost",
            Self::ResourceDesirability => "Resource Desirability",
            Self::Provinces => "Provinces",
            Self::Factions => "Factions",
            Self::PoliticalState => "Political State",
            Self::Prosperity => "Prosperity",
            Self::Settlements => "Settlements",
        }
    }

    pub fn category(&self) -> &'static str {
        match self {
            Self::Habitability | Self::NavigationCost | Self::ResourceDesirability => "Analysis",
            Self::Provinces | Self::Factions | Self::PoliticalState => "Political",
            Self::Prosperity => "Economic",
            Self::Settlements => "Settlements",
        }
    }

    pub fn all() -> &'static [LifeGenLayer] {
        &[
            Self::Habitability,
            Self::NavigationCost,
            Self::ResourceDesirability,
            Self::Provinces,
            Self::Factions,
            Self::PoliticalState,
            Self::Prosperity,
            Self::Settlements,
        ]
    }
}

/// Current visualization layer for the Civilization Generator view.
#[derive(Resource)]
pub struct CurrentLifeGenLayer(pub LifeGenLayer);

impl Default for CurrentLifeGenLayer {
    fn default() -> Self {
        Self(LifeGenLayer::Provinces)
    }
}

/// Overlay visibility toggles for the civilization map.
#[derive(Resource)]
pub struct OverlayState {
    pub province_borders: bool,
    pub faction_borders: bool,
    pub settlement_icons: bool,
    pub road_network: bool,
    pub trade_routes: bool,
}

impl Default for OverlayState {
    fn default() -> Self {
        Self {
            province_borders: true,
            faction_borders: false,
            settlement_icons: true,
            road_network: false,
            trade_routes: false,
        }
    }
}

/// UI state for the civilization generator panel.
#[derive(Resource, Default)]
pub struct CivUiState {
    pub civ_seed_text: String,
    pub initialized: bool,
}

/// Signal resource requesting lifegen regeneration.
#[derive(Resource, Default)]
pub struct RegenerateLifeGenRequest {
    pub pending: bool,
}

/// System that renders the Civilization Generator side panel (F2 mode).
pub fn civ_panel_system(
    mut contexts: EguiContexts,
    mut world_def: ResMut<WorldDefinition>,
    mut ui_state: ResMut<CivUiState>,
    mut current_layer: ResMut<CurrentLifeGenLayer>,
    mut overlay: ResMut<OverlayState>,
    mut regen_request: ResMut<RegenerateLifeGenRequest>,
    lifegen_data: Option<Res<LifeGenData>>,
) {
    // Initialize civ seed text from world definition
    if !ui_state.initialized {
        ui_state.civ_seed_text = world_def.civ_seed.to_string();
        ui_state.initialized = true;
    }

    egui::SidePanel::left("civ_panel")
        .default_width(180.0)
        .show(contexts.ctx_mut(), |ui| {
            ui.heading("Civilization Generator");
            ui.separator();

            // Civ seed
            ui.label("Civ Seed:");
            ui.horizontal(|ui| {
                let response = ui.text_edit_singleline(&mut ui_state.civ_seed_text);
                if response.lost_focus() {
                    if let Ok(new_seed) = ui_state.civ_seed_text.parse::<u32>() {
                        if new_seed != world_def.civ_seed {
                            world_def.civ_seed = new_seed;
                        }
                    } else {
                        // Reset to current seed on invalid input
                        ui_state.civ_seed_text = world_def.civ_seed.to_string();
                    }
                }
                if ui.button("🎲").on_hover_text("Random seed").clicked() {
                    world_def.civ_seed = rand_seed();
                    ui_state.civ_seed_text = world_def.civ_seed.to_string();
                }
            });

            // Regenerate button
            if ui.button("Regenerate LifeGen").clicked() {
                regen_request.pending = true;
            }

            ui.separator();
            ui.add_space(8.0);

            // View layer selection
            ui.heading("View Layer");
            let selected = current_layer.0;
            egui::ComboBox::from_label("Layer")
                .selected_text(selected.name())
                .show_ui(ui, |ui| {
                    let categories = ["Analysis", "Political", "Economic", "Settlements"];
                    for &cat in &categories {
                        ui.separator();
                        ui.label(cat);
                        for &layer in LifeGenLayer::all() {
                            if layer.category() == cat {
                                if ui
                                    .selectable_label(selected == layer, layer.name())
                                    .clicked()
                                {
                                    current_layer.0 = layer;
                                }
                            }
                        }
                    }
                });

            ui.separator();
            ui.add_space(8.0);

            // Overlays
            ui.heading("Overlays");
            ui.checkbox(&mut overlay.province_borders, "Province Borders");
            ui.checkbox(&mut overlay.faction_borders, "Faction Borders");
            ui.checkbox(&mut overlay.settlement_icons, "Settlement Icons");
            ui.checkbox(&mut overlay.road_network, "Road Network");
            ui.checkbox(&mut overlay.trade_routes, "Trade Routes");

            ui.separator();
            ui.add_space(8.0);

            // Info section
            ui.heading("Info");
            if let Some(data) = &lifegen_data {
                ui.label(format!("Provinces: {}", data.provinces.len()));
                ui.label(format!("Factions: {}", data.factions.len()));
                ui.label(format!("Settlements: {}", data.settlement_seeds.len()));
            } else {
                ui.label("No civilisation data yet.");
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn lifegen_layers_have_unique_names() {
        let names: Vec<&str> = LifeGenLayer::all().iter().map(|l| l.name()).collect();
        let unique: HashSet<&str> = names.iter().copied().collect();
        assert_eq!(
            names.len(),
            unique.len(),
            "Duplicate layer names found: {:?}",
            names
        );
    }

    #[test]
    fn overlay_state_defaults() {
        let state = OverlayState::default();
        assert!(state.province_borders);
        assert!(!state.faction_borders);
        assert!(state.settlement_icons);
        assert!(!state.road_network);
        assert!(!state.trade_routes);
    }
}

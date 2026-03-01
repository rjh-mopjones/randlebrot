use bevy::prelude::*;

pub mod civilization;
pub mod culture;
pub mod definition;
pub mod faction;
pub mod roads;
pub mod settlement_placement;
pub mod territory;

pub use civilization::{CivilizationConfig, CivilizationGenerator, CivilizationResult};
pub use culture::{BiomePreferences, Culture, CultureTraits, CultureType};
pub use definition::{
    City, CityTier, Landmark, LandmarkKind, NoiseParams, Point2D, Polygon, Region,
    SelectedChunk, WorldDefinition, WorldIdGenerator,
};
pub use faction::{Faction, FactionDisposition};
pub use roads::{Road, RoadType, TradeGood, TradeRoute};
pub use territory::TerritoryMap;

/// World plugin for Randlebrot.
/// Manages world definition, plates, coastlines, and climate baking.
pub struct RbWorldPlugin;

impl Plugin for RbWorldPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WorldDefinition>()
            .init_resource::<SelectedChunk>()
            .init_resource::<WorldIdGenerator>();
    }
}

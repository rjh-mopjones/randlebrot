use bevy::prelude::*;

pub mod definition;

pub use definition::{
    City, CityTier, Landmark, LandmarkKind, NoiseParams, Point2D, Polygon, Region,
    SelectedChunk, WorldDefinition, WorldIdGenerator,
};

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

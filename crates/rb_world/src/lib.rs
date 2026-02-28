use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// World definition resource containing global parameters.
#[derive(Resource, Debug, Clone, Serialize, Deserialize)]
pub struct WorldDefinition {
    /// World seed for procedural generation.
    pub seed: u32,
    /// X position of the terminator line (twilight zone center).
    pub terminator_x: f64,
    /// Width of the habitable twilight zone.
    pub twilight_width: f64,
}

impl Default for WorldDefinition {
    fn default() -> Self {
        Self {
            seed: 42,
            terminator_x: 0.0,
            twilight_width: 1000.0,
        }
    }
}

/// World plugin for Randlebrot.
/// Manages world definition, plates, coastlines, and climate baking.
pub struct RbWorldPlugin;

impl Plugin for RbWorldPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WorldDefinition>();
    }
}

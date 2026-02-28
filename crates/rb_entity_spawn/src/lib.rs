use bevy::prelude::*;

/// Entity spawn plugin for Randlebrot.
/// Handles building, NPC, and clutter spawning from chunk parameters.
pub struct RbEntitySpawnPlugin;

impl Plugin for RbEntitySpawnPlugin {
    fn build(&self, _app: &mut App) {
        // Entity spawning systems will be added in future iterations.
    }
}

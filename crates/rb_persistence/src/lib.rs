use bevy::prelude::*;

/// Persistence plugin for Randlebrot.
/// Handles delta storage and save/load functionality using RON format.
pub struct RbPersistencePlugin;

impl Plugin for RbPersistencePlugin {
    fn build(&self, _app: &mut App) {
        // Persistence systems will be added in future iterations.
    }
}

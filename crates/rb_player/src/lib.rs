use bevy::prelude::*;

/// Player marker component.
#[derive(Component)]
pub struct Player;

/// Player plugin for Randlebrot.
/// Handles player controller, camera, and 2D top-down interaction.
pub struct RbPlayerPlugin;

impl Plugin for RbPlayerPlugin {
    fn build(&self, _app: &mut App) {
        // Player systems will be added in future iterations.
    }
}

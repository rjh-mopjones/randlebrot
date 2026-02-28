use bevy::prelude::*;

pub mod world_io;

pub use world_io::{
    ensure_worlds_dir, list_worlds, load_world, save_world, world_filename, world_path,
    WorldIoError, WORLDS_DIR,
};

/// Persistence plugin for Randlebrot.
/// Handles delta storage and save/load functionality using RON format.
pub struct RbPersistencePlugin;

impl Plugin for RbPersistencePlugin {
    fn build(&self, _app: &mut App) {
        // Ensure worlds directory exists on startup
        if let Err(e) = ensure_worlds_dir() {
            eprintln!("Warning: Could not create worlds directory: {}", e);
        }
    }
}

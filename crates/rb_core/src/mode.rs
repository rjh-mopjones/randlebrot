use bevy::prelude::*;

/// Application mode state for the Randlebrot editor.
///
/// The editor operates in one of four modes, each providing
/// different tools and views for world creation and testing.
#[derive(States, Default, Clone, Eq, PartialEq, Hash, Debug)]
pub enum AppMode {
    /// Generate procedural world maps from noise parameters.
    /// Primary view: 1024×512 MacroMap
    /// Tools: Seed input, noise params, layer toggle, save/load
    #[default]
    WorldGenerator,

    /// Author global world features on top of generated terrain.
    /// Primary view: World map with overlays
    /// Tools: Region drawing, city placement, landmark placement
    WorldMapEditor,

    /// Edit individual chunks at street level.
    /// Primary view: 512×512 MicroMap
    /// Tools: Tile painting, building placement, NPC spawns
    ChunkEditor,

    /// Test gameplay in the current chunk.
    /// Primary view: Playable chunk with player
    /// Tools: Play/stop, teleport, debug overlays
    LevelLauncher,
}

impl AppMode {
    /// Get the display name for UI.
    pub fn name(&self) -> &'static str {
        match self {
            Self::WorldGenerator => "Generator",
            Self::WorldMapEditor => "Map Editor",
            Self::ChunkEditor => "Chunk Editor",
            Self::LevelLauncher => "Launcher",
        }
    }

    /// Get the keyboard shortcut for this mode.
    pub fn shortcut(&self) -> KeyCode {
        match self {
            Self::WorldGenerator => KeyCode::F1,
            Self::WorldMapEditor => KeyCode::F2,
            Self::ChunkEditor => KeyCode::F3,
            Self::LevelLauncher => KeyCode::F4,
        }
    }

    /// Get all modes in order.
    pub fn all() -> &'static [AppMode] {
        &[
            Self::WorldGenerator,
            Self::WorldMapEditor,
            Self::ChunkEditor,
            Self::LevelLauncher,
        ]
    }
}

/// Event fired when transitioning between modes.
#[derive(Event, Clone, Debug)]
pub struct ModeTransitionEvent {
    pub from: AppMode,
    pub to: AppMode,
}

/// System that handles F1-F4 key presses to switch modes.
pub fn handle_mode_shortcuts(
    keyboard: Res<ButtonInput<KeyCode>>,
    current_mode: Res<State<AppMode>>,
    mut next_mode: ResMut<NextState<AppMode>>,
    mut events: EventWriter<ModeTransitionEvent>,
) {
    for mode in AppMode::all() {
        if keyboard.just_pressed(mode.shortcut()) && current_mode.get() != mode {
            events.send(ModeTransitionEvent {
                from: current_mode.get().clone(),
                to: mode.clone(),
            });
            next_mode.set(mode.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_shortcuts_are_unique() {
        let shortcuts: Vec<_> = AppMode::all().iter().map(|m| m.shortcut()).collect();
        let unique: std::collections::HashSet<_> = shortcuts.iter().collect();
        assert_eq!(shortcuts.len(), unique.len());
    }

    #[test]
    fn all_modes_have_names() {
        for mode in AppMode::all() {
            assert!(!mode.name().is_empty());
        }
    }
}

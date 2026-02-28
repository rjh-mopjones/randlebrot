use std::fs;
use std::path::Path;
use rb_world::WorldDefinition;

/// Default directory for world saves.
pub const WORLDS_DIR: &str = "assets/worlds";

/// Error type for world I/O operations.
#[derive(Debug)]
pub enum WorldIoError {
    Io(std::io::Error),
    Ron(ron::Error),
    RonSpanned(ron::error::SpannedError),
}

impl From<std::io::Error> for WorldIoError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<ron::Error> for WorldIoError {
    fn from(err: ron::Error) -> Self {
        Self::Ron(err)
    }
}

impl From<ron::error::SpannedError> for WorldIoError {
    fn from(err: ron::error::SpannedError) -> Self {
        Self::RonSpanned(err)
    }
}

impl std::fmt::Display for WorldIoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "IO error: {}", e),
            Self::Ron(e) => write!(f, "RON serialization error: {}", e),
            Self::RonSpanned(e) => write!(f, "RON parse error: {}", e),
        }
    }
}

impl std::error::Error for WorldIoError {}

/// Save a world definition to a RON file.
///
/// # Arguments
/// * `path` - File path to save to
/// * `world` - World definition to save
pub fn save_world(path: &Path, world: &WorldDefinition) -> Result<(), WorldIoError> {
    let pretty_config = ron::ser::PrettyConfig::new()
        .depth_limit(4)
        .separate_tuple_members(true)
        .enumerate_arrays(true);

    let ron_string = ron::ser::to_string_pretty(world, pretty_config)?;
    fs::write(path, ron_string)?;
    Ok(())
}

/// Load a world definition from a RON file.
///
/// # Arguments
/// * `path` - File path to load from
pub fn load_world(path: &Path) -> Result<WorldDefinition, WorldIoError> {
    let contents = fs::read_to_string(path)?;
    let world: WorldDefinition = ron::from_str(&contents)?;
    Ok(world)
}

/// Ensure the worlds directory exists.
pub fn ensure_worlds_dir() -> Result<(), std::io::Error> {
    fs::create_dir_all(WORLDS_DIR)
}

/// List all world files in the worlds directory.
pub fn list_worlds() -> Result<Vec<std::path::PathBuf>, std::io::Error> {
    let dir = Path::new(WORLDS_DIR);
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut worlds = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("ron") {
            worlds.push(path);
        }
    }

    worlds.sort();
    Ok(worlds)
}

/// Generate a filename from a world name.
pub fn world_filename(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    format!("{}.ron", sanitized.to_lowercase())
}

/// Get the full path for a world file.
pub fn world_path(name: &str) -> std::path::PathBuf {
    Path::new(WORLDS_DIR).join(world_filename(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn save_and_load_world() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test_world.ron");

        let world = WorldDefinition::default();
        save_world(&path, &world).unwrap();

        let loaded = load_world(&path).unwrap();
        assert_eq!(loaded.name, world.name);
        assert_eq!(loaded.seed, world.seed);
    }

    #[test]
    fn world_filename_sanitizes() {
        assert_eq!(world_filename("My World"), "my_world.ron");
        assert_eq!(world_filename("Test-123"), "test-123.ron");
        assert_eq!(world_filename("Hello World!"), "hello_world_.ron");
    }
}

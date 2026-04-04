use serde::{Deserialize, Serialize};

/// Biome/tile types for world map generation.
/// Uses multi-axis climate classification for realistic biome placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum TileType {
    // Water types
    #[default]
    Sea,
    ShallowSea,        // Light cyan - near-shore shallow water
    ContinentalShelf,  // Medium cyan - continental shelf
    DeepOcean,         // Dark navy - abyssal plain
    OceanTrench, // Deep navy - plate boundary depths
    OceanRidge,  // Warm brown - mid-ocean divergent ridge
    River,       // Flowing water

    // Coastal
    Beach,
    Mangrove,    // Dark olive - tropical coastal wetland
    CoralReef,   // Coral pink - warm shallow water
    RockyCoast,  // Slate gray - rocky shoreline
    SeaCliff,    // Light slate - coastal cliff

    // Frozen biomes (dark side of tidally locked planet)
    White,      // Frozen ocean/ice
    Glacier,    // Ice blue - glacial ice on dark side
    Snow,       // Snow-covered land
    IceSheet,   // Vast flat ice plains
    FrozenBog,  // Frozen wetland
    Tundra,     // Grayish green - permafrost
    Taiga,      // Dark teal - cold coniferous forest
    AlpineMeadow, // High altitude meadow in cold zones

    // Temperate biomes (terminator zone)
    Plains,
    Meadow,              // Lush lowland grassland
    Forest,
    DeciduousForest,     // Temperate broadleaf forest
    TemperateRainforest, // Wet temperate forest
    Woodland,            // Open canopy forest
    Scrubland,           // Dry shrubby terrain
    Marsh,               // Olive drab - wetland
    Steppe,              // Pale yellow-green - dry grassland
    Mountain,
    Plateau,

    // Warm/subtropical biomes
    SubtropicalForest, // Warm humid forest
    DryWoodland,       // Warm open woodland
    Thornland,         // Thorny scrub
    HighlandSavanna,   // Warm highland grassland
    CloudForest,       // Humid highland forest

    // Hot biomes (sun side)
    Savanna,   // Khaki - hot grassland
    Jungle,    // Very dark green - hot humid forest
    Desert,
    Sahara,
    Erg,       // Sand sea - flat sandy desert
    Hamada,    // Rocky desert pavement
    SaltFlat,  // Evaporated salt plain
    Badlands,  // Rust brown - eroded arid terrain
    Oasis,     // River-fed green in desert
    Volcanic,  // Dark red-brown - active volcanic
    LavaField, // Cooled lava surrounding volcanic
    MoltenWaste,  // Scorching volcanic terrain
    ScorchedRock, // Heat-blasted rock
}

impl TileType {
    /// Returns the RGB color for this tile type.
    pub fn rgb(&self) -> [u8; 3] {
        match self {
            // Water types
            Self::Sea => [0, 191, 255],         // Cyan blue
            Self::ShallowSea => [100, 200, 240],      // Light cyan - near-shore
            Self::ContinentalShelf => [70, 150, 200], // Medium cyan - shelf
            Self::DeepOcean => [0, 40, 100],          // Dark navy - abyssal plain
            Self::OceanTrench => [0, 51, 102],  // Deep navy - plate boundary depths
            Self::OceanRidge => [120, 80, 60],  // Warm brown - mid-ocean divergent ridge
            Self::River => [64, 164, 223],      // Light blue

            // Coastal
            Self::Beach => [222, 184, 135],     // Tan/burlywood
            Self::Mangrove => [60, 90, 40],     // Dark olive - tropical coastal wetland
            Self::CoralReef => [255, 127, 127], // Coral pink - warm shallow water
            Self::RockyCoast => [100, 100, 112],// Slate gray - rocky shoreline
            Self::SeaCliff => [140, 140, 160],  // Light slate - coastal cliff

            // Frozen biomes
            Self::White => [250, 252, 255],        // Bright ice white
            Self::Glacier => [230, 240, 255],      // Pale ice blue
            Self::Snow => [240, 244, 250],         // Bright snow white
            Self::IceSheet => [235, 245, 255],     // Pale ice blue
            Self::FrozenBog => [200, 215, 210],    // Pale frost-teal
            Self::Tundra => [176, 196, 176],       // Grayish green - permafrost
            Self::Taiga => [34, 85, 68],           // Dark teal - cold coniferous forest
            Self::AlpineMeadow => [140, 200, 140], // Soft green

            // Temperate biomes
            Self::Plains => [50, 205, 50],              // Lime green
            Self::Meadow => [120, 210, 90],             // Bright meadow green
            Self::Forest => [0, 100, 0],                // Dark green
            Self::DeciduousForest => [40, 140, 40],     // Medium green
            Self::TemperateRainforest => [0, 80, 50],   // Deep green
            Self::Woodland => [80, 150, 60],            // Open canopy green
            Self::Scrubland => [170, 160, 100],         // Dry shrub tan
            Self::Marsh => [85, 107, 47],               // Olive drab - wetland
            Self::Steppe => [160, 170, 110],            // Pale yellow-green
            Self::Mountain => [105, 105, 105],          // Dark gray
            Self::Plateau => [139, 69, 19],             // Saddle brown

            // Warm/subtropical biomes
            Self::SubtropicalForest => [20, 110, 30],   // Rich green
            Self::DryWoodland => [140, 130, 60],        // Olive-brown
            Self::Thornland => [150, 120, 70],          // Dry thorny brown
            Self::HighlandSavanna => [180, 175, 100],   // Warm highland tan
            Self::CloudForest => [30, 100, 60],         // Misty green

            // Hot biomes
            Self::Savanna => [189, 183, 107],   // Khaki - hot grassland
            Self::Jungle => [0, 80, 32],        // Very dark green - hot humid forest
            Self::Desert => [255, 215, 0],      // Gold
            Self::Sahara => [255, 165, 0],      // Orange
            Self::Erg => [235, 210, 140],       // Bright sandy yellow — distinct from Sahara orange
            Self::Hamada => [130, 95, 70],       // Dark rocky brown — exposed bedrock pavement
            Self::SaltFlat => [240, 235, 220],   // Near-white salt crust — bright, bleached
            Self::Badlands => [178, 102, 68],    // Rust brown - eroded arid terrain
            Self::Oasis => [60, 180, 60],        // Verdant green
            Self::Volcanic => [64, 32, 32],      // Dark red-brown - active volcanic
            Self::LavaField => [80, 40, 30],     // Cooled lava dark
            Self::MoltenWaste => [100, 30, 15],  // Red-black volcanic — darker, more menacing
            Self::ScorchedRock => [60, 55, 50],  // Dark charcoal — heat-blasted basalt
        }
    }

    /// Returns the RGBA color for this tile type.
    pub fn color(&self) -> [u8; 4] {
        let [r, g, b] = self.rgb();
        [r, g, b, 255]
    }

    /// Determine tile type from continentalness and temperature.
    /// Uses the fungal-jungle tiling strategy thresholds.
    ///
    /// # Arguments
    /// * `continentalness` - Elevation factor from noise, typically [-1.0, 1.0]
    /// * `temperature` - Temperature value, typically [-50, 100]
    /// * `sea_level` - Threshold for ocean vs land (default: -0.025)
    pub fn from_climate(continentalness: f64, temperature: f64, sea_level: f64) -> Self {
        if continentalness < sea_level {
            // Ocean
            if temperature < -15.0 {
                Self::White // Frozen ocean
            } else if temperature > 50.0 {
                Self::Desert // Hot ocean (rare)
            } else {
                Self::Sea
            }
        } else if continentalness < sea_level + 0.02 {
            // Coastal zone
            if temperature > 3.0 {
                Self::Beach
            } else {
                Self::Snow
            }
        } else if continentalness < sea_level + 0.1 {
            // Low land
            if temperature < 3.0 {
                Self::Snow
            } else if temperature > 60.0 {
                Self::Sahara
            } else {
                Self::Plains
            }
        } else if continentalness < sea_level + 0.2 {
            // Mid land
            if temperature < 3.0 {
                Self::Snow
            } else if temperature > 60.0 {
                Self::Sahara
            } else {
                Self::Forest
            }
        } else if continentalness < sea_level + 0.3 {
            // High land (mountains)
            if temperature > 70.0 {
                Self::Plateau
            } else {
                Self::Mountain
            }
        } else {
            // Extreme elevation
            if temperature < 70.0 {
                Self::Snow
            } else {
                Self::Plateau
            }
        }
    }
}

// Re-export as BiomeType for backwards compatibility
pub type BiomeType = TileType;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ocean_below_sea_level() {
        let tile = TileType::from_climate(-0.5, 20.0, -0.025);
        assert_eq!(tile, TileType::Sea);
    }

    #[test]
    fn frozen_ocean() {
        let tile = TileType::from_climate(-0.5, -30.0, -0.025);
        assert_eq!(tile, TileType::White);
    }

    #[test]
    fn beach_near_coast() {
        let tile = TileType::from_climate(-0.01, 25.0, -0.025);
        assert_eq!(tile, TileType::Beach);
    }

    #[test]
    fn plains_low_land() {
        let tile = TileType::from_climate(0.05, 25.0, -0.025);
        assert_eq!(tile, TileType::Plains);
    }

    #[test]
    fn forest_mid_land() {
        let tile = TileType::from_climate(0.15, 25.0, -0.025);
        assert_eq!(tile, TileType::Forest);
    }

    #[test]
    fn mountain_high_land() {
        let tile = TileType::from_climate(0.25, 30.0, -0.025);
        assert_eq!(tile, TileType::Mountain);
    }

    #[test]
    fn sahara_hot_lowland() {
        let tile = TileType::from_climate(0.05, 70.0, -0.025);
        assert_eq!(tile, TileType::Sahara);
    }
}

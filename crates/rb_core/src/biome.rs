/// Biome/tile types for world map generation.
/// Uses multi-axis climate classification for realistic biome placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum TileType {
    // Water types
    #[default]
    Sea,
    OceanTrench, // Deep navy - plate boundary depths
    River,       // Flowing water

    // Coastal
    Beach,

    // Frozen biomes (dark side of tidally locked planet)
    White,   // Frozen ocean/ice
    Glacier, // Ice blue - glacial ice on dark side
    Snow,    // Snow-covered land
    Tundra,  // Grayish green - permafrost
    Taiga,   // Dark teal - cold coniferous forest

    // Temperate biomes (terminator zone)
    Plains,
    Forest,
    Marsh,   // Olive drab - wetland
    Steppe,  // Pale yellow-green - dry grassland
    Mountain,
    Plateau,

    // Hot biomes (sun side)
    Savanna,  // Khaki - hot grassland
    Jungle,   // Very dark green - hot humid forest
    Desert,
    Sahara,
    Badlands, // Rust brown - eroded arid terrain
    Volcanic, // Dark red-brown - active volcanic
}

impl TileType {
    /// Returns the RGB color for this tile type.
    pub fn rgb(&self) -> [u8; 3] {
        match self {
            // Water types
            Self::Sea => [0, 191, 255],         // Cyan blue
            Self::OceanTrench => [0, 51, 102],  // Deep navy - plate boundary depths
            Self::River => [64, 164, 223],      // Light blue

            // Coastal
            Self::Beach => [222, 184, 135],     // Tan/burlywood

            // Frozen biomes
            Self::White => [255, 255, 255],     // Pure white (frozen ocean)
            Self::Glacier => [200, 220, 255],   // Ice blue - glacial ice
            Self::Snow => [211, 211, 211],      // Light gray
            Self::Tundra => [176, 196, 176],    // Grayish green - permafrost
            Self::Taiga => [34, 85, 68],        // Dark teal - cold coniferous forest

            // Temperate biomes
            Self::Plains => [50, 205, 50],      // Lime green
            Self::Forest => [0, 100, 0],        // Dark green
            Self::Marsh => [85, 107, 47],       // Olive drab - wetland
            Self::Steppe => [160, 170, 110],    // Pale yellow-green - dry grassland
            Self::Mountain => [105, 105, 105],  // Dark gray
            Self::Plateau => [139, 69, 19],     // Saddle brown

            // Hot biomes
            Self::Savanna => [189, 183, 107],   // Khaki - hot grassland
            Self::Jungle => [0, 80, 32],        // Very dark green - hot humid forest
            Self::Desert => [255, 215, 0],      // Gold
            Self::Sahara => [255, 165, 0],      // Orange
            Self::Badlands => [178, 102, 68],   // Rust brown - eroded arid terrain
            Self::Volcanic => [64, 32, 32],     // Dark red-brown - active volcanic
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

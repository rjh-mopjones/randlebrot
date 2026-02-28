/// Biome types for world map generation.
/// Determined by continentalness (elevation) and temperature.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum BiomeType {
    // Water biomes
    #[default]
    Ocean,
    IcePack,
    HotOcean,

    // Coastal biomes
    Beach,
    SnowBeach,

    // Lowland biomes
    Plains,
    Tundra,
    Desert,

    // Midland biomes
    Forest,

    // Highland biomes
    Mountain,
    Plateau,
    SnowPeaks,
}

impl BiomeType {
    /// Returns the RGBA color for this biome.
    /// Colors match the fungal-jungle style: tan land, green forests, blue ocean.
    pub fn color(&self) -> [u8; 4] {
        match self {
            // Water - cyan/turquoise blue
            Self::Ocean => [64, 191, 255, 255],      // Cyan blue
            Self::IcePack => [200, 230, 255, 255],   // Light ice blue
            Self::HotOcean => [64, 191, 255, 255],   // Same as ocean (hot water still blue)

            // Coastal - tan/sandy
            Self::Beach => [210, 180, 140, 255],     // Tan
            Self::SnowBeach => [180, 190, 200, 255], // Cool gray-tan

            // Lowland/temperate - tan for warm, green for moderate
            Self::Plains => [210, 180, 140, 255],    // Tan (warm plains are dry)
            Self::Tundra => [180, 190, 200, 255],    // Cool gray
            Self::Desert => [210, 180, 140, 255],    // Tan (same as plains - dry land)

            // Forest - dark green (cooler, wetter areas)
            Self::Forest => [0, 128, 0, 255],        // Dark green

            // Highland
            Self::Mountain => [140, 130, 120, 255],  // Gray-brown
            Self::Plateau => [180, 150, 120, 255],   // Light brown
            Self::SnowPeaks => [220, 220, 230, 255], // Light gray/white
        }
    }

    /// Determine biome from continentalness and temperature.
    ///
    /// # Arguments
    /// * `continentalness` - Elevation factor, typically in [-1.0, 1.0]
    /// * `temperature` - Temperature in Celsius-like scale, typically [-100, 100]
    /// * `sea_level` - The continentalness threshold for water vs land (default -0.025)
    pub fn from_climate(continentalness: f64, temperature: f64, sea_level: f64) -> Self {
        if continentalness < sea_level {
            // Ocean biomes
            if temperature < -15.0 {
                Self::IcePack
            } else if temperature > 50.0 {
                Self::HotOcean
            } else {
                Self::Ocean
            }
        } else if continentalness < sea_level + 0.02 {
            // Coastal/beach biomes
            if temperature > 3.0 {
                Self::Beach
            } else {
                Self::SnowBeach
            }
        } else if continentalness < sea_level + 0.1 {
            // Lowland biomes
            if temperature < 3.0 {
                Self::Tundra
            } else if temperature > 60.0 {
                Self::Desert
            } else {
                Self::Plains
            }
        } else if continentalness < sea_level + 0.2 {
            // Midland biomes
            if temperature < 3.0 {
                Self::Tundra
            } else if temperature > 60.0 {
                Self::Desert
            } else {
                Self::Forest
            }
        } else if continentalness < sea_level + 0.3 {
            // Highland biomes
            if temperature > 70.0 {
                Self::Plateau
            } else {
                Self::Mountain
            }
        } else {
            // High mountain peaks
            if temperature < 70.0 {
                Self::SnowPeaks
            } else {
                Self::Plateau
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_ocean_is_blue() {
        let biome = BiomeType::from_climate(-0.5, 20.0, -0.025);
        assert_eq!(biome, BiomeType::Ocean);
    }

    #[test]
    fn frozen_ocean_is_ice() {
        let biome = BiomeType::from_climate(-0.5, -30.0, -0.025);
        assert_eq!(biome, BiomeType::IcePack);
    }

    #[test]
    fn lowland_warm_is_plains() {
        let biome = BiomeType::from_climate(0.05, 25.0, -0.025);
        assert_eq!(biome, BiomeType::Plains);
    }

    #[test]
    fn midland_warm_is_forest() {
        let biome = BiomeType::from_climate(0.15, 25.0, -0.025);
        assert_eq!(biome, BiomeType::Forest);
    }

    #[test]
    fn highland_cool_is_mountain() {
        let biome = BiomeType::from_climate(0.25, 30.0, -0.025);
        assert_eq!(biome, BiomeType::Mountain);
    }

    #[test]
    fn very_high_cold_is_snow_peaks() {
        let biome = BiomeType::from_climate(0.35, 20.0, -0.025);
        assert_eq!(biome, BiomeType::SnowPeaks);
    }
}

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
    pub fn color(&self) -> [u8; 4] {
        match self {
            Self::Ocean => [0, 191, 255, 255],       // Deep blue
            Self::IcePack => [255, 255, 255, 255],   // White
            Self::HotOcean => [255, 165, 0, 255],    // Orange (hot water)
            Self::Beach => [222, 184, 135, 255],    // Tan/sandy
            Self::SnowBeach => [200, 200, 210, 255], // Light gray-blue
            Self::Plains => [50, 205, 50, 255],     // Lime green
            Self::Tundra => [211, 211, 211, 255],   // Light gray
            Self::Desert => [255, 165, 0, 255],     // Orange
            Self::Forest => [0, 100, 0, 255],       // Dark green
            Self::Mountain => [105, 105, 105, 255], // Dark gray
            Self::Plateau => [139, 69, 19, 255],    // Brown
            Self::SnowPeaks => [240, 240, 245, 255], // Near white
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

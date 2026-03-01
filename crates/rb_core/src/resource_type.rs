use crate::TileType;

/// Types of natural resources that can be found in the world.
/// Each resource has terrain biases that affect where it appears.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResourceType {
    // Metals (biased toward mountains/tectonic boundaries)
    Iron,
    Gold,
    Copper,
    Silver,

    // Mining resources (biased toward mountains)
    Gems,
    Coal,
    Stone,
    Salt,

    // Organic resources (biased toward specific biomes)
    Timber,      // Forests
    Fish,        // Coastal/water
    FertileSoil, // Plains
    WildGame,    // Forests/plains
}

impl ResourceType {
    /// Returns all resource types.
    pub fn all() -> &'static [ResourceType] {
        &[
            Self::Iron,
            Self::Gold,
            Self::Copper,
            Self::Silver,
            Self::Gems,
            Self::Coal,
            Self::Stone,
            Self::Salt,
            Self::Timber,
            Self::Fish,
            Self::FertileSoil,
            Self::WildGame,
        ]
    }

    /// Returns the terrain bias for this resource type.
    pub fn terrain_bias(&self) -> TerrainBias {
        match self {
            Self::Iron => TerrainBias::Mountain { weight: 0.7 },
            Self::Gold => TerrainBias::TectonicBoundary { weight: 0.8 },
            Self::Copper => TerrainBias::Mountain { weight: 0.5 },
            Self::Silver => TerrainBias::TectonicBoundary { weight: 0.6 },
            Self::Gems => TerrainBias::TectonicBoundary { weight: 0.9 },
            Self::Coal => TerrainBias::Mountain { weight: 0.6 },
            Self::Stone => TerrainBias::Mountain { weight: 0.3 },
            Self::Salt => TerrainBias::Coastal { weight: 0.7 },
            Self::Timber => TerrainBias::Biome {
                biome: TileType::Forest,
                weight: 0.9,
            },
            Self::Fish => TerrainBias::Coastal { weight: 0.95 },
            Self::FertileSoil => TerrainBias::Biome {
                biome: TileType::Plains,
                weight: 0.8,
            },
            Self::WildGame => TerrainBias::MultipleBiomes {
                biomes: &[TileType::Forest, TileType::Plains],
                weight: 0.7,
            },
        }
    }

    /// Returns the display name for this resource.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Iron => "Iron",
            Self::Gold => "Gold",
            Self::Copper => "Copper",
            Self::Silver => "Silver",
            Self::Gems => "Gems",
            Self::Coal => "Coal",
            Self::Stone => "Stone",
            Self::Salt => "Salt",
            Self::Timber => "Timber",
            Self::Fish => "Fish",
            Self::FertileSoil => "Fertile Soil",
            Self::WildGame => "Wild Game",
        }
    }

    /// Returns the display color for this resource (for visualization).
    pub fn color(&self) -> [u8; 4] {
        match self {
            Self::Iron => [139, 69, 19, 255],     // Rust brown
            Self::Gold => [255, 215, 0, 255],     // Gold
            Self::Copper => [184, 115, 51, 255],  // Copper
            Self::Silver => [192, 192, 192, 255], // Silver
            Self::Gems => [148, 0, 211, 255],     // Purple
            Self::Coal => [30, 30, 30, 255],      // Dark gray
            Self::Stone => [128, 128, 128, 255],  // Gray
            Self::Salt => [255, 250, 250, 255],   // Snow white
            Self::Timber => [34, 139, 34, 255],   // Forest green
            Self::Fish => [0, 191, 255, 255],     // Deep sky blue
            Self::FertileSoil => [139, 90, 43, 255], // Sienna
            Self::WildGame => [160, 82, 45, 255], // Sienna
        }
    }

    /// Returns a unique seed offset for this resource type.
    /// Used to ensure different noise patterns per resource.
    pub fn seed_offset(&self) -> u32 {
        match self {
            Self::Iron => 1000,
            Self::Gold => 2000,
            Self::Copper => 3000,
            Self::Silver => 4000,
            Self::Gems => 5000,
            Self::Coal => 6000,
            Self::Stone => 7000,
            Self::Salt => 8000,
            Self::Timber => 9000,
            Self::Fish => 10000,
            Self::FertileSoil => 11000,
            Self::WildGame => 12000,
        }
    }
}

/// Terrain bias determines where a resource is likely to appear.
#[derive(Clone, Copy, Debug)]
pub enum TerrainBias {
    /// Higher values in elevated terrain (mountains, plateaus).
    Mountain { weight: f64 },
    /// Higher values near tectonic plate boundaries.
    TectonicBoundary { weight: f64 },
    /// Higher values near water (coasts, rivers).
    Coastal { weight: f64 },
    /// Higher values in a specific biome.
    Biome { biome: TileType, weight: f64 },
    /// Higher values in multiple biomes.
    MultipleBiomes {
        biomes: &'static [TileType],
        weight: f64,
    },
}

impl TerrainBias {
    /// Calculate the bias multiplier given terrain context.
    pub fn calculate(
        &self,
        continentalness: f64,
        tectonic_boundary_distance: f64,
        water_distance: f64,
        biome: TileType,
    ) -> f64 {
        match self {
            TerrainBias::Mountain { weight } => {
                // Continentalness > 0.1 is elevated terrain
                let mountain_factor = ((continentalness - 0.1) / 0.4).clamp(0.0, 1.0);
                1.0 - weight + weight * mountain_factor
            }
            TerrainBias::TectonicBoundary { weight } => {
                // 0 = on boundary, 1 = center of plate
                let boundary_factor = 1.0 - tectonic_boundary_distance;
                1.0 - weight + weight * boundary_factor
            }
            TerrainBias::Coastal { weight } => {
                // 0 = on water, 1 = far from water
                let coastal_factor = 1.0 - water_distance.min(1.0);
                1.0 - weight + weight * coastal_factor
            }
            TerrainBias::Biome {
                biome: target_biome,
                weight,
            } => {
                if biome == *target_biome {
                    1.0
                } else {
                    1.0 - weight
                }
            }
            TerrainBias::MultipleBiomes { biomes, weight } => {
                if biomes.contains(&biome) {
                    1.0
                } else {
                    1.0 - weight
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_resources_returns_12_types() {
        assert_eq!(ResourceType::all().len(), 12);
    }

    #[test]
    fn mountain_bias_increases_with_continentalness() {
        let bias = TerrainBias::Mountain { weight: 0.7 };
        let low = bias.calculate(-0.5, 0.5, 0.5, TileType::Plains);
        let high = bias.calculate(0.5, 0.5, 0.5, TileType::Mountain);
        assert!(high > low);
    }

    #[test]
    fn coastal_bias_increases_near_water() {
        let bias = TerrainBias::Coastal { weight: 0.8 };
        let near = bias.calculate(0.0, 0.5, 0.1, TileType::Beach);
        let far = bias.calculate(0.0, 0.5, 0.9, TileType::Plains);
        assert!(near > far);
    }

    #[test]
    fn biome_bias_matches_target() {
        let bias = TerrainBias::Biome {
            biome: TileType::Forest,
            weight: 0.9,
        };
        let forest = bias.calculate(0.0, 0.5, 0.5, TileType::Forest);
        let plains = bias.calculate(0.0, 0.5, 0.5, TileType::Plains);
        assert_eq!(forest, 1.0);
        assert_eq!(plains, 0.1);
    }
}

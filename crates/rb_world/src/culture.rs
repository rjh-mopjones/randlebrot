//! Culture definitions for procedural civilization generation.
//!
//! Cultures represent different peoples adapted to specific environments
//! in the tidally-locked world.

use rb_core::TileType;
use serde::{Deserialize, Serialize};

/// Culture archetype representing adaptation to environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CultureType {
    /// Twilight dwellers - temperate zone specialists along the terminator.
    TwilightDweller,
    /// Cold-adapted - northern/frozen regions near the dark side.
    FrostKin,
    /// Heat-resistant - southern/desert regions near the sun side.
    SunForged,
    /// Coastal traders - shorelines and ports.
    TideWalker,
    /// Mountain folk - highlands and passes.
    StoneBorn,
}

impl CultureType {
    /// Returns all culture types.
    pub fn all() -> &'static [CultureType] {
        &[
            CultureType::TwilightDweller,
            CultureType::FrostKin,
            CultureType::SunForged,
            CultureType::TideWalker,
            CultureType::StoneBorn,
        ]
    }

    /// Returns a display name for this culture type.
    pub fn name(&self) -> &'static str {
        match self {
            CultureType::TwilightDweller => "Twilight Dweller",
            CultureType::FrostKin => "Frost Kin",
            CultureType::SunForged => "Sun Forged",
            CultureType::TideWalker => "Tide Walker",
            CultureType::StoneBorn => "Stone Born",
        }
    }

    /// Returns a default faction name for this culture.
    pub fn default_faction_name(&self) -> &'static str {
        match self {
            CultureType::TwilightDweller => "Twilight Confederacy",
            CultureType::FrostKin => "Northern Holds",
            CultureType::SunForged => "Sunward Tribes",
            CultureType::TideWalker => "Coastal League",
            CultureType::StoneBorn => "Mountain Kingdoms",
        }
    }

    /// Returns a default color for this culture (RGBA).
    pub fn default_color(&self) -> [u8; 4] {
        match self {
            CultureType::TwilightDweller => [100, 180, 100, 200], // Green
            CultureType::FrostKin => [150, 200, 255, 200],        // Ice blue
            CultureType::SunForged => [255, 180, 80, 200],        // Orange
            CultureType::TideWalker => [80, 150, 200, 200],       // Ocean blue
            CultureType::StoneBorn => [160, 140, 120, 200],       // Stone gray
        }
    }
}

/// Biome suitability preferences for a culture.
/// Values range from -1.0 (hostile) to 1.0 (ideal).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BiomePreferences {
    // Water types
    pub sea: f64,
    pub ocean_trench: f64,
    pub river: f64,

    // Coastal
    pub beach: f64,

    // Frozen biomes
    pub white: f64,   // Frozen ocean/ice
    pub glacier: f64, // Glacial ice
    pub snow: f64,
    pub tundra: f64,
    pub taiga: f64,

    // Temperate biomes
    pub plains: f64,
    pub forest: f64,
    pub marsh: f64,
    pub steppe: f64,
    pub mountain: f64,
    pub plateau: f64,

    // Hot biomes
    pub savanna: f64,
    pub jungle: f64,
    pub desert: f64,
    pub sahara: f64,
    pub badlands: f64,
    pub volcanic: f64,
}

impl Default for BiomePreferences {
    fn default() -> Self {
        Self {
            // Water
            sea: -1.0,
            ocean_trench: -1.0,
            river: 0.6, // Rivers are attractive for settlements

            // Coastal
            beach: 0.0,

            // Frozen
            white: -1.0,
            glacier: -1.0,
            snow: -0.2,
            tundra: -0.3,
            taiga: 0.1,

            // Temperate
            plains: 0.5,
            forest: 0.3,
            marsh: -0.2,
            steppe: 0.2,
            mountain: 0.0,
            plateau: 0.1,

            // Hot
            savanna: 0.2,
            jungle: -0.1,
            desert: -0.3,
            sahara: -0.5,
            badlands: -0.4,
            volcanic: -0.8,
        }
    }
}

impl BiomePreferences {
    /// Get the preference score for a given tile type.
    pub fn get(&self, tile: TileType) -> f64 {
        match tile {
            // Water
            TileType::Sea => self.sea,
            TileType::OceanTrench => self.ocean_trench,
            TileType::River => self.river,

            // Coastal
            TileType::Beach => self.beach,

            // Frozen
            TileType::White => self.white,
            TileType::Glacier => self.glacier,
            TileType::Snow => self.snow,
            TileType::Tundra => self.tundra,
            TileType::Taiga => self.taiga,

            // Temperate
            TileType::Plains => self.plains,
            TileType::Forest => self.forest,
            TileType::Marsh => self.marsh,
            TileType::Steppe => self.steppe,
            TileType::Mountain => self.mountain,
            TileType::Plateau => self.plateau,

            // Hot
            TileType::Savanna => self.savanna,
            TileType::Jungle => self.jungle,
            TileType::Desert => self.desert,
            TileType::Sahara => self.sahara,
            TileType::Badlands => self.badlands,
            TileType::Volcanic => self.volcanic,
        }
    }
}

/// Traits affecting settlement patterns and expansion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CultureTraits {
    /// 0.0 = nomadic, 1.0 = settled. Affects village density.
    pub settlement_tendency: f64,
    /// Preferred minimum distance between settlements.
    pub settlement_spacing: f64,
    /// How aggressively they expand (affects territory size).
    pub expansion_drive: f64,
    /// Trade propensity (affects road building priority).
    pub trade_focus: f64,
    /// How much they value defensible positions.
    pub defensive_preference: f64,
}

impl Default for CultureTraits {
    fn default() -> Self {
        Self {
            settlement_tendency: 0.7,
            settlement_spacing: 80.0,
            expansion_drive: 0.5,
            trade_focus: 0.5,
            defensive_preference: 0.5,
        }
    }
}

/// Complete culture definition with environmental preferences.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Culture {
    pub culture_type: CultureType,
    pub name: String,
    /// Biome suitability scores.
    pub biome_preferences: BiomePreferences,
    /// Temperature comfort range (min, max) in degrees.
    pub temperature_range: (f64, f64),
    /// Continentalness preference (min, max) - how far inland they prefer.
    pub continentalness_range: (f64, f64),
    /// Traits affecting settlement patterns.
    pub traits: CultureTraits,
}

impl Culture {
    /// Create a Twilight Dweller culture - temperate zone specialists.
    pub fn twilight_dweller() -> Self {
        Self {
            culture_type: CultureType::TwilightDweller,
            name: "Twilight Confederacy".into(),
            biome_preferences: BiomePreferences {
                // Water
                sea: -1.0,
                ocean_trench: -1.0,
                river: 0.9, // Rivers are highly valued

                // Coastal
                beach: 0.5,

                // Frozen
                white: -1.0,
                glacier: -1.0,
                snow: 0.1,
                tundra: -0.1,
                taiga: 0.5,

                // Temperate
                plains: 1.0,
                forest: 0.8,
                marsh: 0.2,
                steppe: 0.6,
                mountain: 0.3,
                plateau: 0.4,

                // Hot
                savanna: 0.3,
                jungle: 0.1,
                desert: -0.2,
                sahara: -0.5,
                badlands: -0.3,
                volcanic: -0.7,
            },
            temperature_range: (10.0, 40.0),
            continentalness_range: (0.0, 0.25),
            traits: CultureTraits {
                settlement_tendency: 0.9,
                settlement_spacing: 60.0,
                expansion_drive: 0.6,
                trade_focus: 0.8,
                defensive_preference: 0.4,
            },
        }
    }

    /// Create a Frost Kin culture - cold region specialists.
    pub fn frost_kin() -> Self {
        Self {
            culture_type: CultureType::FrostKin,
            name: "Northern Holds".into(),
            biome_preferences: BiomePreferences {
                // Water
                sea: -1.0,
                ocean_trench: -1.0,
                river: 0.7, // Frozen rivers useful

                // Coastal
                beach: 0.1,

                // Frozen - preferred
                white: 0.2, // Can settle on ice edges
                glacier: 0.3,
                snow: 1.0,
                tundra: 0.9,
                taiga: 0.8,

                // Temperate
                plains: 0.3,
                forest: 0.6,
                marsh: 0.1,
                steppe: 0.2,
                mountain: 0.5,
                plateau: 0.4,

                // Hot - hostile
                savanna: -0.5,
                jungle: -0.7,
                desert: -0.8,
                sahara: -1.0,
                badlands: -0.6,
                volcanic: -0.4, // Geothermal heat useful
            },
            temperature_range: (-40.0, 10.0),
            continentalness_range: (0.05, 0.35),
            traits: CultureTraits {
                settlement_tendency: 0.7,
                settlement_spacing: 100.0, // Sparse settlements
                expansion_drive: 0.3,
                trade_focus: 0.4,
                defensive_preference: 0.7,
            },
        }
    }

    /// Create a Sun Forged culture - desert/hot region specialists.
    pub fn sun_forged() -> Self {
        Self {
            culture_type: CultureType::SunForged,
            name: "Sunward Tribes".into(),
            biome_preferences: BiomePreferences {
                // Water
                sea: -1.0,
                ocean_trench: -1.0,
                river: 1.0, // Oases/rivers vital in desert

                // Coastal
                beach: 0.3,

                // Frozen - hostile
                white: -1.0,
                glacier: -1.0,
                snow: -1.0,
                tundra: -0.8,
                taiga: -0.6,

                // Temperate
                plains: 0.4,
                forest: -0.2,
                marsh: -0.3,
                steppe: 0.5,
                mountain: 0.2,
                plateau: 0.7,

                // Hot - preferred
                savanna: 0.9,
                jungle: 0.3,
                desert: 0.8,
                sahara: 1.0,
                badlands: 0.7,
                volcanic: 0.5, // Familiar with heat
            },
            temperature_range: (40.0, 100.0),
            continentalness_range: (0.0, 0.3),
            traits: CultureTraits {
                settlement_tendency: 0.5, // More nomadic
                settlement_spacing: 90.0,
                expansion_drive: 0.4,
                trade_focus: 0.6, // Trade caravans
                defensive_preference: 0.3,
            },
        }
    }

    /// Create a Tide Walker culture - coastal specialists.
    pub fn tide_walker() -> Self {
        Self {
            culture_type: CultureType::TideWalker,
            name: "Coastal League".into(),
            biome_preferences: BiomePreferences {
                // Water
                sea: 0.3, // Can settle near water
                ocean_trench: -0.5,
                river: 0.9, // Rivers are trade routes

                // Coastal - preferred
                beach: 1.0,

                // Frozen
                white: -0.5,
                glacier: -0.6,
                snow: 0.1,
                tundra: 0.0,
                taiga: 0.2,

                // Temperate
                plains: 0.5,
                forest: 0.3,
                marsh: 0.7, // Coastal marshes
                steppe: 0.2,
                mountain: -0.2,
                plateau: 0.0,

                // Hot
                savanna: 0.2,
                jungle: 0.4, // Coastal jungle
                desert: 0.1,
                sahara: -0.3,
                badlands: -0.2,
                volcanic: -0.4,
            },
            temperature_range: (5.0, 50.0),
            continentalness_range: (-0.02, 0.1), // Very coastal
            traits: CultureTraits {
                settlement_tendency: 0.8,
                settlement_spacing: 50.0, // Dense coastal settlements
                expansion_drive: 0.5,
                trade_focus: 1.0, // Highly trade-focused
                defensive_preference: 0.4,
            },
        }
    }

    /// Create a Stone Born culture - mountain specialists.
    pub fn stone_born() -> Self {
        Self {
            culture_type: CultureType::StoneBorn,
            name: "Mountain Kingdoms".into(),
            biome_preferences: BiomePreferences {
                // Water
                sea: -1.0,
                ocean_trench: -1.0,
                river: 0.5, // Mountain streams useful

                // Coastal
                beach: -0.3,

                // Frozen
                white: -0.5,
                glacier: 0.3, // Mountain glaciers
                snow: 0.5,
                tundra: 0.2,
                taiga: 0.4,

                // Temperate - mountain areas
                plains: 0.1,
                forest: 0.3,
                marsh: -0.4,
                steppe: 0.3,
                mountain: 1.0, // Preferred
                plateau: 0.9,

                // Hot
                savanna: 0.1,
                jungle: -0.3,
                desert: 0.0,
                sahara: -0.2,
                badlands: 0.4, // Rocky terrain
                volcanic: 0.6, // Volcanic mountains
            },
            temperature_range: (-20.0, 50.0),
            continentalness_range: (0.2, 0.5), // High elevation
            traits: CultureTraits {
                settlement_tendency: 0.8,
                settlement_spacing: 80.0,
                expansion_drive: 0.4,
                trade_focus: 0.5, // Control mountain passes
                defensive_preference: 1.0, // Highly defensive
            },
        }
    }

    /// Create a default culture from a culture type.
    pub fn from_type(culture_type: CultureType) -> Self {
        match culture_type {
            CultureType::TwilightDweller => Self::twilight_dweller(),
            CultureType::FrostKin => Self::frost_kin(),
            CultureType::SunForged => Self::sun_forged(),
            CultureType::TideWalker => Self::tide_walker(),
            CultureType::StoneBorn => Self::stone_born(),
        }
    }

    /// Get all default cultures.
    pub fn all_defaults() -> Vec<Self> {
        CultureType::all()
            .iter()
            .map(|&ct| Self::from_type(ct))
            .collect()
    }

    /// Calculate suitability score for a location.
    pub fn calculate_suitability(
        &self,
        biome: TileType,
        temperature: f64,
        continentalness: f64,
    ) -> f64 {
        // Biome preference (40% weight)
        let biome_score = (self.biome_preferences.get(biome) + 1.0) / 2.0; // Normalize to [0, 1]

        // Temperature comfort (30% weight)
        let (min_temp, max_temp) = self.temperature_range;
        let temp_score = if temperature < min_temp {
            (1.0 - (min_temp - temperature) / 50.0).max(0.0)
        } else if temperature > max_temp {
            (1.0 - (temperature - max_temp) / 50.0).max(0.0)
        } else {
            1.0
        };

        // Continentalness preference (30% weight)
        let (min_cont, max_cont) = self.continentalness_range;
        let cont_score = if continentalness < min_cont {
            (1.0 - (min_cont - continentalness) / 0.3).max(0.0)
        } else if continentalness > max_cont {
            (1.0 - (continentalness - max_cont) / 0.3).max(0.0)
        } else {
            1.0
        };

        0.4 * biome_score + 0.3 * temp_score + 0.3 * cont_score
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_cultures_have_defaults() {
        let cultures = Culture::all_defaults();
        assert_eq!(cultures.len(), 5);
    }

    #[test]
    fn twilight_dweller_prefers_plains() {
        let culture = Culture::twilight_dweller();
        assert!(culture.biome_preferences.plains > culture.biome_preferences.desert);
    }

    #[test]
    fn frost_kin_prefers_snow() {
        let culture = Culture::frost_kin();
        assert!(culture.biome_preferences.snow > culture.biome_preferences.sahara);
    }

    #[test]
    fn suitability_in_range() {
        let culture = Culture::twilight_dweller();
        let score = culture.calculate_suitability(TileType::Plains, 25.0, 0.1);
        assert!(score >= 0.0 && score <= 1.0);
    }
}

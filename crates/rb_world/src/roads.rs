//! Roads and trade routes for procedural civilization generation.
//!
//! Roads connect settlements, and trade routes represent economic connections.

use crate::definition::Point2D;
use rb_core::TileType;
use serde::{Deserialize, Serialize};

/// Road quality/importance type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RoadType {
    /// Major highway between capitals.
    Imperial,
    /// Standard road between towns.
    Provincial,
    /// Minor path to villages.
    Trail,
}

impl RoadType {
    /// Get the display width in pixels for this road type.
    pub fn width(&self) -> f32 {
        match self {
            RoadType::Imperial => 3.0,
            RoadType::Provincial => 2.0,
            RoadType::Trail => 1.0,
        }
    }

    /// Get the display color (RGB) for this road type.
    pub fn color(&self) -> [u8; 3] {
        match self {
            RoadType::Imperial => [220, 180, 80],   // Gold
            RoadType::Provincial => [180, 180, 180], // Silver
            RoadType::Trail => [140, 110, 80],       // Brown
        }
    }
}

/// A road connecting two settlements.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Road {
    /// Unique identifier.
    pub id: u32,
    /// Ordered list of waypoints (world coordinates).
    pub waypoints: Vec<Point2D>,
    /// Road quality/type.
    pub road_type: RoadType,
    /// Settlement IDs this road connects (from, to).
    pub connects: (u32, u32),
}

impl Road {
    /// Create a new road.
    pub fn new(id: u32, connects: (u32, u32), road_type: RoadType) -> Self {
        Self {
            id,
            waypoints: Vec::new(),
            road_type,
            connects,
        }
    }

    /// Calculate the total length of the road.
    pub fn length(&self) -> f64 {
        if self.waypoints.len() < 2 {
            return 0.0;
        }

        self.waypoints
            .windows(2)
            .map(|w| {
                let dx = w[1].x - w[0].x;
                let dy = w[1].y - w[0].y;
                (dx * dx + dy * dy).sqrt()
            })
            .sum()
    }

    /// Check if this road connects a specific settlement.
    pub fn connects_settlement(&self, city_id: u32) -> bool {
        self.connects.0 == city_id || self.connects.1 == city_id
    }
}

/// Types of tradeable goods.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TradeGood {
    /// Agricultural products.
    Food,
    /// Metals and minerals.
    Ore,
    /// Wood and forest products.
    Timber,
    /// Cloth and clothing.
    Textiles,
    /// High-value goods (spices, gems, art).
    Luxury,
    /// Military equipment.
    Weapons,
    /// Salt, preserved foods.
    Salt,
    /// Fish and sea products.
    Fish,
    /// Furs and hides.
    Furs,
}

impl TradeGood {
    /// Get a display name for this trade good.
    pub fn name(&self) -> &'static str {
        match self {
            TradeGood::Food => "Food",
            TradeGood::Ore => "Ore",
            TradeGood::Timber => "Timber",
            TradeGood::Textiles => "Textiles",
            TradeGood::Luxury => "Luxury Goods",
            TradeGood::Weapons => "Weapons",
            TradeGood::Salt => "Salt",
            TradeGood::Fish => "Fish",
            TradeGood::Furs => "Furs",
        }
    }

    /// Get typical goods produced by a biome.
    pub fn from_biome(biome: TileType) -> Vec<TradeGood> {
        match biome {
            // Water
            TileType::Sea | TileType::ShallowSea | TileType::ContinentalShelf => vec![TradeGood::Fish],
            TileType::OceanTrench | TileType::DeepOcean => vec![TradeGood::Fish],
            TileType::OceanRidge => vec![TradeGood::Fish, TradeGood::Ore],
            TileType::CoralReef => vec![TradeGood::Fish, TradeGood::Luxury],
            TileType::River => vec![TradeGood::Fish, TradeGood::Food],

            // Coastal
            TileType::Beach => vec![TradeGood::Fish, TradeGood::Salt],
            TileType::RockyCoast | TileType::SeaCliff => vec![TradeGood::Fish, TradeGood::Salt],
            TileType::Mangrove => vec![TradeGood::Fish, TradeGood::Timber],

            // Frozen
            TileType::White | TileType::IceSheet => vec![TradeGood::Fish, TradeGood::Furs],
            TileType::Glacier => vec![TradeGood::Furs],
            TileType::Snow => vec![TradeGood::Furs],
            TileType::FrozenBog => vec![TradeGood::Furs],
            TileType::Tundra => vec![TradeGood::Furs],
            TileType::Taiga => vec![TradeGood::Timber, TradeGood::Furs],
            TileType::AlpineMeadow => vec![TradeGood::Food, TradeGood::Furs],

            // Temperate
            TileType::Plains | TileType::Meadow => vec![TradeGood::Food, TradeGood::Textiles],
            TileType::Forest | TileType::DeciduousForest => vec![TradeGood::Timber, TradeGood::Furs],
            TileType::TemperateRainforest => vec![TradeGood::Timber, TradeGood::Luxury],
            TileType::Woodland | TileType::DryWoodland => vec![TradeGood::Timber, TradeGood::Food],
            TileType::Scrubland | TileType::Thornland => vec![TradeGood::Furs],
            TileType::Marsh => vec![TradeGood::Fish, TradeGood::Food],
            TileType::Steppe => vec![TradeGood::Food, TradeGood::Furs],
            TileType::Mountain => vec![TradeGood::Ore, TradeGood::Weapons],
            TileType::Plateau => vec![TradeGood::Ore, TradeGood::Food],

            // Warm/subtropical
            TileType::SubtropicalForest | TileType::CloudForest => vec![TradeGood::Timber, TradeGood::Luxury],
            TileType::HighlandSavanna => vec![TradeGood::Food, TradeGood::Furs],

            // Hot
            TileType::Savanna => vec![TradeGood::Food, TradeGood::Furs],
            TileType::Jungle => vec![TradeGood::Timber, TradeGood::Luxury],
            TileType::Desert | TileType::Erg => vec![TradeGood::Salt, TradeGood::Luxury],
            TileType::Sahara | TileType::Hamada => vec![TradeGood::Luxury],
            TileType::SaltFlat => vec![TradeGood::Salt],
            TileType::Badlands | TileType::ScorchedRock => vec![TradeGood::Ore],
            TileType::Oasis => vec![TradeGood::Food, TradeGood::Luxury],
            TileType::Volcanic | TileType::LavaField => vec![TradeGood::Ore, TradeGood::Luxury],
            TileType::MoltenWaste => vec![TradeGood::Ore],
        }
    }
}

/// A trade route connecting multiple settlements.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeRoute {
    /// Unique identifier.
    pub id: u32,
    /// Road IDs that form this trade route.
    pub road_ids: Vec<u32>,
    /// Faction IDs involved in this trade route.
    pub faction_ids: Vec<u32>,
    /// Settlement IDs along this route (endpoints and waypoints).
    pub settlement_ids: Vec<u32>,
    /// Goods typically traded on this route.
    pub goods: Vec<TradeGood>,
    /// Economic importance [0.0, 1.0].
    pub importance: f64,
}

impl TradeRoute {
    /// Create a new trade route.
    pub fn new(id: u32) -> Self {
        Self {
            id,
            road_ids: Vec::new(),
            faction_ids: Vec::new(),
            settlement_ids: Vec::new(),
            goods: Vec::new(),
            importance: 0.5,
        }
    }

    /// Check if this route involves a specific faction.
    pub fn involves_faction(&self, faction_id: u32) -> bool {
        self.faction_ids.contains(&faction_id)
    }

    /// Check if this is an international trade route (multiple factions).
    pub fn is_international(&self) -> bool {
        self.faction_ids.len() > 1
    }
}

/// Movement cost for pathfinding through different terrain.
pub fn terrain_movement_cost(biome: TileType) -> f64 {
    match biome {
        // Impassable by land
        TileType::Sea | TileType::OceanTrench | TileType::White | TileType::Glacier
        | TileType::ShallowSea | TileType::ContinentalShelf | TileType::DeepOcean
        | TileType::OceanRidge | TileType::CoralReef | TileType::IceSheet => {
            f64::INFINITY
        }
        TileType::Volcanic | TileType::MoltenWaste => 10.0,
        TileType::LavaField => 8.0,

        // Difficult terrain
        TileType::Mountain => 8.0,
        TileType::Snow | TileType::Tundra | TileType::FrozenBog => 6.0,
        TileType::SeaCliff => 6.0,
        TileType::Badlands | TileType::ScorchedRock => 5.5,
        TileType::Plateau => 5.0,
        TileType::Jungle | TileType::Marsh | TileType::Mangrove
        | TileType::TemperateRainforest => 4.5,
        TileType::Desert | TileType::Sahara | TileType::Erg
        | TileType::SaltFlat | TileType::Hamada => 4.0,
        TileType::Taiga | TileType::CloudForest => 3.5,

        // Moderate terrain
        TileType::Forest | TileType::DeciduousForest | TileType::SubtropicalForest => 3.0,
        TileType::Thornland | TileType::Scrubland => 2.5,
        TileType::Woodland | TileType::DryWoodland => 2.0,
        TileType::River => 2.0,
        TileType::Steppe | TileType::Savanna | TileType::HighlandSavanna => 1.5,
        TileType::Beach | TileType::RockyCoast => 1.5,
        TileType::AlpineMeadow => 1.5,

        // Easy terrain
        TileType::Plains | TileType::Meadow | TileType::Oasis => 1.0,
    }
}

/// Check if terrain is passable for road building.
pub fn is_passable(biome: TileType) -> bool {
    !matches!(
        biome,
        TileType::Sea | TileType::OceanTrench | TileType::White | TileType::Glacier
            | TileType::ShallowSea | TileType::ContinentalShelf | TileType::DeepOcean
            | TileType::OceanRidge | TileType::CoralReef | TileType::IceSheet
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn road_length_empty() {
        let road = Road::new(1, (1, 2), RoadType::Trail);
        assert_eq!(road.length(), 0.0);
    }

    #[test]
    fn road_length_single_segment() {
        let mut road = Road::new(1, (1, 2), RoadType::Trail);
        road.waypoints = vec![Point2D::new(0.0, 0.0), Point2D::new(3.0, 4.0)];
        assert_eq!(road.length(), 5.0);
    }

    #[test]
    fn plains_are_cheapest() {
        assert!(terrain_movement_cost(TileType::Plains) < terrain_movement_cost(TileType::Mountain));
    }

    #[test]
    fn sea_is_impassable() {
        assert!(!is_passable(TileType::Sea));
        assert!(terrain_movement_cost(TileType::Sea).is_infinite());
    }

    #[test]
    fn plains_produce_food() {
        let goods = TradeGood::from_biome(TileType::Plains);
        assert!(goods.contains(&TradeGood::Food));
    }
}

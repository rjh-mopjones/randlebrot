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
            TileType::Plains => vec![TradeGood::Food, TradeGood::Textiles],
            TileType::Forest => vec![TradeGood::Timber, TradeGood::Furs],
            TileType::Mountain => vec![TradeGood::Ore, TradeGood::Weapons],
            TileType::Plateau => vec![TradeGood::Ore, TradeGood::Food],
            TileType::Beach => vec![TradeGood::Fish, TradeGood::Salt],
            TileType::Sea => vec![TradeGood::Fish],
            TileType::Desert => vec![TradeGood::Salt, TradeGood::Luxury],
            TileType::Sahara => vec![TradeGood::Luxury],
            TileType::Snow => vec![TradeGood::Furs],
            TileType::White => vec![TradeGood::Fish, TradeGood::Furs],
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
        TileType::Sea | TileType::White => f64::INFINITY, // Impassable by land
        TileType::Mountain => 8.0,
        TileType::Plateau => 5.0,
        TileType::Snow => 6.0,
        TileType::Desert | TileType::Sahara => 4.0,
        TileType::Forest => 3.0,
        TileType::Beach => 1.5,
        TileType::Plains => 1.0, // Ideal for roads
    }
}

/// Check if terrain is passable for road building.
pub fn is_passable(biome: TileType) -> bool {
    !matches!(biome, TileType::Sea | TileType::White)
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

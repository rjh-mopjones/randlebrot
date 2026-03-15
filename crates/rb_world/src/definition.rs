use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::culture::Culture;
use crate::faction::Faction;
use crate::roads::{Road, TradeRoute};
use crate::territory::TerritoryMap;

/// World definition resource containing all authored world data.
///
/// This is the top-level serializable structure for a world,
/// containing both procedural parameters and authored content.
#[derive(Resource, Debug, Clone, Serialize, Deserialize)]
pub struct WorldDefinition {
    /// Human-readable name for this world.
    pub name: String,
    /// World seed for procedural generation.
    pub seed: u32,
    /// Civilisation seed (allows iterating on politics without regenerating terrain).
    #[serde(default = "default_civ_seed")]
    pub civ_seed: u32,
    /// Map width in pixels (MacroMap).
    pub width: usize,
    /// Map height in pixels (MacroMap).
    pub height: usize,
    /// Sea level threshold for continentalness.
    pub sea_level: f64,
    /// X position of the terminator line (twilight zone center).
    pub terminator_x: f64,
    /// Width of the habitable twilight zone.
    pub twilight_width: f64,
    /// Sub-stellar point position (normalized 0-1). Default: (0.5, 1.0) = bottom center.
    #[serde(default = "default_sub_stellar")]
    pub sub_stellar: (f64, f64),
    /// Noise parameters for world generation.
    pub noise_params: NoiseParams,
    /// Authored regions (countries, territories).
    pub regions: Vec<Region>,
    /// Authored cities.
    pub cities: Vec<City>,
    /// Authored landmarks.
    pub landmarks: Vec<Landmark>,
    /// Cultures present in this world.
    pub cultures: Vec<Culture>,
    /// Political factions.
    pub factions: Vec<Faction>,
    /// Road network.
    pub roads: Vec<Road>,
    /// Trade routes.
    pub trade_routes: Vec<TradeRoute>,
    /// Cached territory ownership (regenerated on load, not serialized).
    #[serde(skip)]
    pub territory_cache: Option<TerritoryMap>,
}

impl Default for WorldDefinition {
    fn default() -> Self {
        Self {
            name: "New World".to_string(),
            seed: 42,
            civ_seed: default_civ_seed(),
            width: 1024,
            height: 512,
            sea_level: -0.025,
            terminator_x: 512.0,
            twilight_width: 200.0,
            sub_stellar: default_sub_stellar(),
            noise_params: NoiseParams::default(),
            regions: Vec::new(),
            cities: Vec::new(),
            landmarks: Vec::new(),
            cultures: Vec::new(),
            factions: Vec::new(),
            roads: Vec::new(),
            trade_routes: Vec::new(),
            territory_cache: None,
        }
    }
}

fn default_sub_stellar() -> (f64, f64) {
    (0.5, 1.0)
}

fn default_civ_seed() -> u32 {
    137
}

/// Noise generation parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoiseParams {
    /// Number of octaves for continentalness noise.
    pub continentalness_octaves: u32,
    /// Persistence for continentalness noise.
    pub continentalness_persistence: f64,
    /// Lacunarity for continentalness noise.
    pub continentalness_lacunarity: f64,
    /// Number of octaves for temperature noise (legacy, kept for save compat).
    #[serde(default = "default_temp_octaves")]
    pub temperature_octaves: u32,
    /// Persistence for temperature noise (legacy, kept for save compat).
    #[serde(default = "default_temp_persistence")]
    pub temperature_persistence: f64,
    /// Number of octaves for rock hardness noise.
    #[serde(default = "default_rock_hardness_octaves")]
    pub rock_hardness_octaves: u32,
    /// Persistence for rock hardness noise.
    #[serde(default = "default_rock_hardness_persistence")]
    pub rock_hardness_persistence: f64,
}

fn default_temp_octaves() -> u32 { 8 }
fn default_temp_persistence() -> f64 { 0.59 }
fn default_rock_hardness_octaves() -> u32 { 3 }
fn default_rock_hardness_persistence() -> f64 { 0.6 }

impl Default for NoiseParams {
    fn default() -> Self {
        Self {
            continentalness_octaves: 16,
            continentalness_persistence: 0.59,
            continentalness_lacunarity: 2.0,
            temperature_octaves: 8,
            temperature_persistence: 0.59,
            rock_hardness_octaves: 3,
            rock_hardness_persistence: 0.6,
        }
    }
}

/// A 2D point used for world coordinates.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Point2D {
    pub x: f64,
    pub y: f64,
}

impl Point2D {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// A polygon defined by a series of vertices.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Polygon {
    pub vertices: Vec<Point2D>,
}

impl Polygon {
    pub fn new(vertices: Vec<Point2D>) -> Self {
        Self { vertices }
    }

    /// Check if the polygon is closed (has at least 3 vertices).
    pub fn is_closed(&self) -> bool {
        self.vertices.len() >= 3
    }

    /// Check if a point is inside the polygon using ray casting.
    pub fn contains(&self, point: Point2D) -> bool {
        if !self.is_closed() {
            return false;
        }

        let mut inside = false;
        let n = self.vertices.len();

        for i in 0..n {
            let j = (i + 1) % n;
            let vi = &self.vertices[i];
            let vj = &self.vertices[j];

            if ((vi.y > point.y) != (vj.y > point.y))
                && (point.x < (vj.x - vi.x) * (point.y - vi.y) / (vj.y - vi.y) + vi.x)
            {
                inside = !inside;
            }
        }

        inside
    }
}

/// An authored region (country, territory, biome override zone).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Region {
    /// Unique identifier.
    pub id: u32,
    /// Display name.
    pub name: String,
    /// Polygon boundary.
    pub bounds: Polygon,
    /// Optional biome override (forces this biome within region).
    pub biome_override: Option<String>,
    /// Faction or nation this region belongs to.
    pub faction: Option<String>,
    /// Display color (RGBA).
    pub color: [u8; 4],
}

impl Region {
    pub fn new(id: u32, name: String, bounds: Polygon) -> Self {
        Self {
            id,
            name,
            bounds,
            biome_override: None,
            faction: None,
            color: [100, 100, 200, 128], // Semi-transparent blue
        }
    }
}

/// City population tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CityTier {
    /// Fully authored, tile-by-tile designed.
    Capital,
    /// Light parameters, procedural fills the rest.
    Town,
    /// Just a pin + seed, everything generated.
    #[default]
    Village,
}

impl CityTier {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Capital => "Capital",
            Self::Town => "Town",
            Self::Village => "Village",
        }
    }

    /// Get approximate population range for this tier.
    pub fn population_range(&self) -> (u32, u32) {
        match self {
            Self::Capital => (50_000, 500_000),
            Self::Town => (5_000, 50_000),
            Self::Village => (100, 5_000),
        }
    }
}

/// An authored city.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct City {
    /// Unique identifier.
    pub id: u32,
    /// City name.
    pub name: String,
    /// World position (MacroMap coordinates).
    pub position: Point2D,
    /// Population tier.
    pub tier: CityTier,
    /// Estimated population.
    pub population: u32,
    /// Whether this city has custom chunk data.
    pub is_authored: bool,
    /// Industry types (e.g., "mining", "fishing", "trade").
    pub industries: Vec<String>,
}

impl City {
    pub fn new(id: u32, name: String, position: Point2D, tier: CityTier) -> Self {
        let (min_pop, max_pop) = tier.population_range();
        let population = (min_pop + max_pop) / 2;

        Self {
            id,
            name,
            position,
            tier,
            population,
            is_authored: matches!(tier, CityTier::Capital),
            industries: Vec::new(),
        }
    }
}

/// Landmark type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LandmarkKind {
    #[default]
    Ruin,
    Temple,
    Tower,
    Cave,
    Bridge,
    Monument,
    Mine,
    Port,
    Other,
}

impl LandmarkKind {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Ruin => "Ruin",
            Self::Temple => "Temple",
            Self::Tower => "Tower",
            Self::Cave => "Cave",
            Self::Bridge => "Bridge",
            Self::Monument => "Monument",
            Self::Mine => "Mine",
            Self::Port => "Port",
            Self::Other => "Other",
        }
    }

    pub fn all() -> &'static [LandmarkKind] {
        &[
            Self::Ruin,
            Self::Temple,
            Self::Tower,
            Self::Cave,
            Self::Bridge,
            Self::Monument,
            Self::Mine,
            Self::Port,
            Self::Other,
        ]
    }
}

/// An authored landmark.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Landmark {
    /// Unique identifier.
    pub id: u32,
    /// Landmark name.
    pub name: String,
    /// World position (MacroMap coordinates).
    pub position: Point2D,
    /// Type of landmark.
    pub kind: LandmarkKind,
    /// Optional description.
    pub description: Option<String>,
}

impl Landmark {
    pub fn new(id: u32, name: String, position: Point2D, kind: LandmarkKind) -> Self {
        Self {
            id,
            name,
            position,
            kind,
            description: None,
        }
    }
}

/// Resource tracking the next available ID for world objects.
#[derive(Resource, Default, Debug)]
pub struct WorldIdGenerator {
    next_region_id: u32,
    next_city_id: u32,
    next_landmark_id: u32,
    next_faction_id: u32,
    next_road_id: u32,
    next_trade_route_id: u32,
}

impl WorldIdGenerator {
    pub fn next_region_id(&mut self) -> u32 {
        let id = self.next_region_id;
        self.next_region_id += 1;
        id
    }

    pub fn next_city_id(&mut self) -> u32 {
        let id = self.next_city_id;
        self.next_city_id += 1;
        id
    }

    pub fn next_landmark_id(&mut self) -> u32 {
        let id = self.next_landmark_id;
        self.next_landmark_id += 1;
        id
    }

    pub fn next_faction_id(&mut self) -> u32 {
        let id = self.next_faction_id;
        self.next_faction_id += 1;
        id
    }

    pub fn next_road_id(&mut self) -> u32 {
        let id = self.next_road_id;
        self.next_road_id += 1;
        id
    }

    pub fn next_trade_route_id(&mut self) -> u32 {
        let id = self.next_trade_route_id;
        self.next_trade_route_id += 1;
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polygon_contains_point() {
        let square = Polygon::new(vec![
            Point2D::new(0.0, 0.0),
            Point2D::new(10.0, 0.0),
            Point2D::new(10.0, 10.0),
            Point2D::new(0.0, 10.0),
        ]);

        assert!(square.contains(Point2D::new(5.0, 5.0)));
        assert!(!square.contains(Point2D::new(15.0, 5.0)));
    }

    #[test]
    fn world_definition_serializes() {
        let world = WorldDefinition::default();
        let ron = ron::to_string(&world).unwrap();
        let _: WorldDefinition = ron::from_str(&ron).unwrap();
    }

    #[test]
    fn city_tier_has_valid_ranges() {
        for tier in [CityTier::Capital, CityTier::Town, CityTier::Village] {
            let (min, max) = tier.population_range();
            assert!(min < max);
        }
    }
}

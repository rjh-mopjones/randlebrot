//! Civilization generation orchestrator.
//!
//! This module ties together all civilization generation:
//! cultures, settlements, factions, roads, and territories.

use crate::culture::{Culture, CultureType};
use crate::definition::{City, CityTier, Point2D, WorldDefinition};
use crate::faction::{Faction, FactionDisposition};
use crate::roads::{terrain_movement_cost, Road, RoadType, TradeGood, TradeRoute};
use crate::settlement_placement::place_settlements;
use crate::territory::{terrain_influence_decay, TerritoryMap};
use pathfinding::prelude::astar;
use rb_noise::BiomeMap;

/// Configuration for civilization generation.
#[derive(Debug, Clone)]
pub struct CivilizationConfig {
    /// Maximum number of settlements to place.
    pub max_settlements: usize,
    /// Whether to generate roads.
    pub generate_roads: bool,
    /// Whether to generate trade routes.
    pub generate_trade_routes: bool,
    /// Whether to generate territories.
    pub generate_territories: bool,
    /// Minimum influence for territory expansion.
    pub territory_threshold: f64,
}

impl Default for CivilizationConfig {
    fn default() -> Self {
        Self {
            max_settlements: 50,
            generate_roads: true,
            generate_trade_routes: true,
            generate_territories: true,
            territory_threshold: 0.1,
        }
    }
}

/// Result of civilization generation.
#[derive(Debug)]
pub struct CivilizationResult {
    pub settlements_placed: usize,
    pub factions_created: usize,
    pub roads_built: usize,
    pub trade_routes_created: usize,
}

/// Main civilization generator.
pub struct CivilizationGenerator {
    seed: u32,
    config: CivilizationConfig,
}

impl CivilizationGenerator {
    /// Create a new generator.
    pub fn new(seed: u32, config: CivilizationConfig) -> Self {
        Self { seed, config }
    }

    /// Generate complete civilization for a world.
    pub fn generate(
        &self,
        biome_map: &BiomeMap,
        world_def: &mut WorldDefinition,
    ) -> CivilizationResult {
        // Step 1: Create default cultures
        let cultures = Culture::all_defaults();
        world_def.cultures = cultures.clone();

        // Step 2: Place settlements
        let placement_result = place_settlements(
            biome_map,
            &cultures,
            self.seed,
            self.config.max_settlements,
        );
        world_def.cities = placement_result.settlements;

        // Step 3: Create factions and assign settlements
        let factions = self.create_factions(&world_def.cities, self.seed);
        world_def.factions = factions;

        // Step 4: Generate road network
        let roads_built = if self.config.generate_roads {
            let roads = self.generate_roads(biome_map, &world_def.cities);
            world_def.roads = roads;
            world_def.roads.len()
        } else {
            0
        };

        // Step 5: Generate trade routes
        let trade_routes_created = if self.config.generate_trade_routes && !world_def.roads.is_empty() {
            let trade_routes = self.generate_trade_routes(
                &world_def.roads,
                &world_def.cities,
                &world_def.factions,
                biome_map,
            );
            world_def.trade_routes = trade_routes;
            world_def.trade_routes.len()
        } else {
            0
        };

        // Step 6: Generate territories
        if self.config.generate_territories {
            let territory = self.generate_territories(
                biome_map,
                &world_def.cities,
                &world_def.factions,
            );
            world_def.territory_cache = Some(territory);
        }

        CivilizationResult {
            settlements_placed: world_def.cities.len(),
            factions_created: world_def.factions.len(),
            roads_built,
            trade_routes_created,
        }
    }

    /// Create factions from settled cultures.
    fn create_factions(&self, cities: &[City], seed: u32) -> Vec<Faction> {
        let mut factions = Vec::new();
        let mut faction_id = 1u32;

        // Group cities by culture (determined by their position in the biome map)
        // For now, assign culture based on city name patterns or just distribute

        // Find capitals and create factions for each
        for culture_type in CultureType::all() {
            // Find the best capital candidate for this culture
            // (In reality, we'd check which cities belong to which culture)
            let capital = cities
                .iter()
                .find(|c| c.tier == CityTier::Capital)
                .or_else(|| cities.iter().find(|c| c.tier == CityTier::Town));

            if capital.is_some() {
                let mut faction = Faction::new(
                    faction_id,
                    culture_type.default_faction_name().to_string(),
                    *culture_type,
                );
                faction.disposition = FactionDisposition::from_culture_and_seed(
                    *culture_type,
                    seed.wrapping_add(faction_id),
                );
                factions.push(faction);
                faction_id += 1;
            }
        }

        // Assign cities to factions based on proximity to capitals
        // For simplicity, distribute cities round-robin for now
        if !factions.is_empty() {
            for (i, city) in cities.iter().enumerate() {
                let faction_idx = i % factions.len();
                factions[faction_idx].add_settlement(city.id);

                if city.tier == CityTier::Capital && factions[faction_idx].capital_id.is_none() {
                    factions[faction_idx].set_capital(city.id);
                }
            }
        }

        factions
    }

    /// Generate road network using A* pathfinding.
    fn generate_roads(&self, biome_map: &BiomeMap, cities: &[City]) -> Vec<Road> {
        let mut roads = Vec::new();
        let mut road_id = 1u32;

        // Build roads using minimum spanning tree approach
        // Start with capitals, then connect to nearest neighbors
        let mut connected: Vec<u32> = Vec::new();
        let mut unconnected: Vec<u32> = cities.iter().map(|c| c.id).collect();

        if unconnected.is_empty() {
            return roads;
        }

        // Start with first city
        connected.push(unconnected.remove(0));

        while !unconnected.is_empty() {
            // Find closest pair between connected and unconnected
            let mut best_pair: Option<(u32, u32, f64)> = None;

            for &conn_id in &connected {
                let conn_city = cities.iter().find(|c| c.id == conn_id).unwrap();

                for &unconn_id in &unconnected {
                    let unconn_city = cities.iter().find(|c| c.id == unconn_id).unwrap();

                    let dx = unconn_city.position.x - conn_city.position.x;
                    let dy = unconn_city.position.y - conn_city.position.y;
                    let dist = (dx * dx + dy * dy).sqrt();

                    if best_pair.is_none() || dist < best_pair.unwrap().2 {
                        best_pair = Some((conn_id, unconn_id, dist));
                    }
                }
            }

            if let Some((from_id, to_id, _)) = best_pair {
                let from_city = cities.iter().find(|c| c.id == from_id).unwrap();
                let to_city = cities.iter().find(|c| c.id == to_id).unwrap();

                // Find path using A*
                let waypoints = self.find_path(biome_map, from_city.position, to_city.position);

                let road_type = determine_road_type(from_city.tier, to_city.tier);

                roads.push(Road {
                    id: road_id,
                    waypoints,
                    road_type,
                    connects: (from_id, to_id),
                });
                road_id += 1;

                // Move to connected
                unconnected.retain(|&id| id != to_id);
                connected.push(to_id);
            } else {
                break;
            }
        }

        roads
    }

    /// Find path between two points using A*.
    fn find_path(&self, biome_map: &BiomeMap, from: Point2D, to: Point2D) -> Vec<Point2D> {
        let start = (from.x as i32, from.y as i32);
        let goal = (to.x as i32, to.y as i32);

        let result = astar(
            &start,
            |&(x, y)| {
                // Generate successors (8-connected neighbors)
                let mut neighbors = Vec::with_capacity(8);
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        if dx == 0 && dy == 0 {
                            continue;
                        }
                        let nx = x + dx;
                        let ny = y + dy;

                        if nx >= 0
                            && ny >= 0
                            && (nx as usize) < biome_map.width
                            && (ny as usize) < biome_map.height
                        {
                            if let Some(biome) = biome_map.get_biome(nx as usize, ny as usize) {
                                let cost = terrain_movement_cost(biome);
                                if cost.is_finite() {
                                    let move_cost = if dx != 0 && dy != 0 {
                                        (cost * 1.414) as i32 // Diagonal
                                    } else {
                                        cost as i32
                                    };
                                    neighbors.push(((nx, ny), move_cost.max(1)));
                                }
                            }
                        }
                    }
                }
                neighbors
            },
            |&(x, y)| {
                // Heuristic: Manhattan distance
                ((x - goal.0).abs() + (y - goal.1).abs()) as i32
            },
            |&pos| pos == goal,
        );

        match result {
            Some((path, _cost)) => {
                // Simplify path - only keep waypoints where direction changes
                simplify_path(&path)
            }
            None => {
                // No path found - return direct line
                vec![from, to]
            }
        }
    }

    /// Generate trade routes between faction capitals.
    fn generate_trade_routes(
        &self,
        roads: &[Road],
        cities: &[City],
        factions: &[Faction],
        biome_map: &BiomeMap,
    ) -> Vec<TradeRoute> {
        let mut trade_routes = Vec::new();
        let mut route_id = 1u32;

        // Create trade routes between each pair of factions
        for (i, faction_a) in factions.iter().enumerate() {
            for faction_b in factions.iter().skip(i + 1) {
                let Some(cap_a_id) = faction_a.capital_id else { continue };
                let Some(cap_b_id) = faction_b.capital_id else { continue };

                // Find roads that connect these capitals
                let route_roads = find_route_roads(roads, cap_a_id, cap_b_id);

                if route_roads.is_empty() {
                    continue;
                }

                // Determine goods based on faction regions
                let cap_a = cities.iter().find(|c| c.id == cap_a_id);
                let cap_b = cities.iter().find(|c| c.id == cap_b_id);

                let mut goods = Vec::new();
                if let (Some(a), Some(b)) = (cap_a, cap_b) {
                    let biome_a = biome_map.get_biome(a.position.x as usize, a.position.y as usize);
                    let biome_b = biome_map.get_biome(b.position.x as usize, b.position.y as usize);

                    if let Some(ba) = biome_a {
                        goods.extend(TradeGood::from_biome(ba));
                    }
                    if let Some(bb) = biome_b {
                        for g in TradeGood::from_biome(bb) {
                            if !goods.contains(&g) {
                                goods.push(g);
                            }
                        }
                    }
                }

                trade_routes.push(TradeRoute {
                    id: route_id,
                    road_ids: route_roads,
                    faction_ids: vec![faction_a.id, faction_b.id],
                    settlement_ids: vec![cap_a_id, cap_b_id],
                    goods,
                    importance: 0.7, // Capital-to-capital routes are important
                });
                route_id += 1;
            }
        }

        trade_routes
    }

    /// Generate territory map via flood-fill.
    fn generate_territories(
        &self,
        biome_map: &BiomeMap,
        cities: &[City],
        factions: &[Faction],
    ) -> TerritoryMap {
        let mut territory = TerritoryMap::new(biome_map.width, biome_map.height);

        // Initialize settlements with faction ownership
        for faction in factions {
            for &city_id in &faction.settlement_ids {
                if let Some(city) = cities.iter().find(|c| c.id == city_id) {
                    let x = city.position.x as usize;
                    let y = city.position.y as usize;
                    if x < territory.width && y < territory.height {
                        // Capital has stronger initial influence
                        let influence = if faction.capital_id == Some(city_id) {
                            1.0
                        } else {
                            match city.tier {
                                CityTier::Capital => 1.0,
                                CityTier::Town => 0.8,
                                CityTier::Village => 0.5,
                            }
                        };
                        territory.set(x, y, faction.id, influence);
                    }
                }
            }
        }

        // Flood-fill expansion
        let mut changed = true;
        let mut iterations = 0;
        const MAX_ITERATIONS: usize = 200;

        while changed && iterations < MAX_ITERATIONS {
            changed = false;
            iterations += 1;

            for y in 0..territory.height {
                for x in 0..territory.width {
                    if territory.is_claimed(x, y) {
                        continue;
                    }

                    // Get biome for decay calculation
                    let Some(biome) = biome_map.get_biome(x, y) else {
                        continue;
                    };

                    // Skip impassable terrain
                    let decay = terrain_influence_decay(biome);
                    if decay == 0.0 {
                        continue;
                    }

                    // Find strongest neighbor influence
                    let mut best_faction = 0u32;
                    let mut best_influence = 0.0f64;

                    for (nx, ny) in territory.neighbors(x, y) {
                        let owner = territory.get_owner(nx, ny);
                        let inf = territory.get_influence(nx, ny);

                        if owner != 0 && inf > best_influence {
                            best_faction = owner;
                            best_influence = inf;
                        }
                    }

                    // Apply decay and check threshold
                    let new_influence = best_influence * decay;
                    if new_influence > self.config.territory_threshold {
                        territory.set(x, y, best_faction, new_influence);
                        changed = true;
                    }
                }
            }
        }

        territory
    }
}

/// Determine road type based on connected city tiers.
fn determine_road_type(tier_a: CityTier, tier_b: CityTier) -> RoadType {
    match (tier_a, tier_b) {
        (CityTier::Capital, CityTier::Capital) => RoadType::Imperial,
        (CityTier::Capital, CityTier::Town) | (CityTier::Town, CityTier::Capital) => {
            RoadType::Provincial
        }
        (CityTier::Town, CityTier::Town) => RoadType::Provincial,
        _ => RoadType::Trail,
    }
}

/// Simplify a path by keeping only direction changes.
fn simplify_path(path: &[(i32, i32)]) -> Vec<Point2D> {
    if path.len() < 2 {
        return path
            .iter()
            .map(|&(x, y)| Point2D::new(x as f64, y as f64))
            .collect();
    }

    let mut result = vec![Point2D::new(path[0].0 as f64, path[0].1 as f64)];
    let mut prev_dir = (0i32, 0i32);

    for i in 1..path.len() {
        let curr = path[i];
        let prev = path[i - 1];
        let dir = (curr.0 - prev.0, curr.1 - prev.1);

        if dir != prev_dir {
            result.push(Point2D::new(prev.0 as f64, prev.1 as f64));
            prev_dir = dir;
        }
    }

    result.push(Point2D::new(
        path.last().unwrap().0 as f64,
        path.last().unwrap().1 as f64,
    ));

    result
}

/// Find roads connecting two settlements (simplified - returns direct roads).
fn find_route_roads(roads: &[Road], from_id: u32, to_id: u32) -> Vec<u32> {
    // Simplified: just find direct road if it exists
    for road in roads {
        if (road.connects.0 == from_id && road.connects.1 == to_id)
            || (road.connects.0 == to_id && road.connects.1 == from_id)
        {
            return vec![road.id];
        }
    }

    // TODO: Implement multi-hop route finding
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generator_creates_factions() {
        let biome_map = BiomeMap::generate(42, 256, 128);
        let mut world_def = WorldDefinition::default();
        world_def.seed = 42;

        let generator = CivilizationGenerator::new(42, CivilizationConfig::default());
        let result = generator.generate(&biome_map, &mut world_def);

        assert!(result.settlements_placed > 0);
        assert!(result.factions_created > 0);
    }

    #[test]
    fn road_type_determination() {
        assert_eq!(
            determine_road_type(CityTier::Capital, CityTier::Capital),
            RoadType::Imperial
        );
        assert_eq!(
            determine_road_type(CityTier::Village, CityTier::Village),
            RoadType::Trail
        );
    }
}

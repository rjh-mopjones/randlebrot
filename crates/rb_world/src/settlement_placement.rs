//! Settlement placement algorithm for procedural civilization generation.
//!
//! Places settlements based on terrain suitability, culture preferences,
//! and spacing constraints.

use crate::culture::{Culture, CultureType};
use crate::definition::{City, CityTier, Point2D};
use rb_core::TileType;
use rb_noise::BiomeMap;
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;

/// Minimum distance between settlements (in world units).
const MIN_SETTLEMENT_DISTANCE: f64 = 40.0;

/// Threshold for settlement placement.
const SETTLEMENT_THRESHOLD: f64 = 0.3;

/// A candidate site for settlement placement.
#[derive(Debug, Clone)]
pub struct SettlementCandidate {
    pub position: Point2D,
    pub suitability: f64,
    pub culture_type: CultureType,
    pub biome: TileType,
    pub temperature: f64,
    pub continentalness: f64,
}

/// Result of settlement placement.
#[derive(Debug)]
pub struct PlacementResult {
    pub settlements: Vec<City>,
    pub candidates_evaluated: usize,
    pub candidates_placed: usize,
}

/// Calculate suitability score for a specific culture at a location.
pub fn calculate_culture_suitability(
    culture: &Culture,
    biome: TileType,
    temperature: f64,
    continentalness: f64,
) -> f64 {
    culture.calculate_suitability(biome, temperature, continentalness)
}

/// Find the best culture for a given location.
pub fn find_best_culture(
    cultures: &[Culture],
    biome: TileType,
    temperature: f64,
    continentalness: f64,
) -> (CultureType, f64) {
    cultures
        .iter()
        .map(|c| {
            let score = calculate_culture_suitability(c, biome, temperature, continentalness);
            (c.culture_type, score)
        })
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
        .unwrap_or((CultureType::TwilightDweller, 0.0))
}

/// Calculate local resource score by examining surrounding tiles.
fn local_resource_score(biome_map: &BiomeMap, x: usize, y: usize, radius: usize) -> f64 {
    let mut diversity = std::collections::HashSet::new();
    let mut good_tiles = 0;
    let mut total = 0;

    let x_start = x.saturating_sub(radius);
    let x_end = (x + radius).min(biome_map.width - 1);
    let y_start = y.saturating_sub(radius);
    let y_end = (y + radius).min(biome_map.height - 1);

    for ny in y_start..=y_end {
        for nx in x_start..=x_end {
            if let Some(biome) = biome_map.get_biome(nx, ny) {
                diversity.insert(biome);
                total += 1;

                // Good tiles for resources
                match biome {
                    TileType::Plains | TileType::Forest | TileType::Beach => good_tiles += 1,
                    _ => {}
                }
            }
        }
    }

    if total == 0 {
        return 0.0;
    }

    // Diversity bonus (more variety = better resources)
    let diversity_score = (diversity.len() as f64 / 5.0).min(1.0);
    // Good tile ratio
    let good_ratio = good_tiles as f64 / total as f64;

    diversity_score * 0.4 + good_ratio * 0.6
}

/// Calculate water access score (proximity to coast).
fn water_access_score(biome_map: &BiomeMap, x: usize, y: usize, search_radius: usize) -> f64 {
    let x_start = x.saturating_sub(search_radius);
    let x_end = (x + search_radius).min(biome_map.width - 1);
    let y_start = y.saturating_sub(search_radius);
    let y_end = (y + search_radius).min(biome_map.height - 1);

    for ny in y_start..=y_end {
        for nx in x_start..=x_end {
            if let Some(biome) = biome_map.get_biome(nx, ny) {
                if matches!(biome, TileType::Sea | TileType::Beach) {
                    // Found water - closer is better
                    let dx = (nx as f64 - x as f64).abs();
                    let dy = (ny as f64 - y as f64).abs();
                    let dist = (dx * dx + dy * dy).sqrt();
                    return 1.0 - (dist / search_radius as f64);
                }
            }
        }
    }

    0.0 // No water found
}

/// Calculate defensibility score (nearby mountains/plateaus).
fn defensibility_score(biome_map: &BiomeMap, x: usize, y: usize, radius: usize) -> f64 {
    let mut defensive_tiles = 0;
    let mut total = 0;

    let x_start = x.saturating_sub(radius);
    let x_end = (x + radius).min(biome_map.width - 1);
    let y_start = y.saturating_sub(radius);
    let y_end = (y + radius).min(biome_map.height - 1);

    for ny in y_start..=y_end {
        for nx in x_start..=x_end {
            if let Some(biome) = biome_map.get_biome(nx, ny) {
                total += 1;
                if matches!(biome, TileType::Mountain | TileType::Plateau) {
                    defensive_tiles += 1;
                }
            }
        }
    }

    if total == 0 {
        return 0.0;
    }

    // Some nearby defensive terrain is good, but too much means we're on a mountain
    let ratio = defensive_tiles as f64 / total as f64;
    if ratio > 0.5 {
        // Too mountainous for a good settlement
        0.5 - (ratio - 0.5)
    } else {
        ratio * 2.0 // Scale up so 25% nearby mountains = score of 0.5
    }
}

/// Calculate full site suitability combining all factors.
pub fn calculate_site_suitability(
    biome_map: &BiomeMap,
    x: usize,
    y: usize,
    culture: &Culture,
) -> f64 {
    let Some(biome) = biome_map.get_biome(x, y) else {
        return 0.0;
    };

    // Can't place settlements in water
    if matches!(biome, TileType::Sea | TileType::White) {
        return 0.0;
    }

    let temperature = biome_map.get_temperature(x, y).unwrap_or(20.0);
    let continentalness = biome_map.get_continentalness(x, y).unwrap_or(0.1);

    // Culture preference for this location (40%)
    let culture_score = culture.calculate_suitability(biome, temperature, continentalness);

    // Local resources (20%)
    let resource_score = local_resource_score(biome_map, x, y, 5);

    // Water access (10%)
    let water_score = water_access_score(biome_map, x, y, 15);

    // Defensibility (5%)
    let defense_score = defensibility_score(biome_map, x, y, 8);

    // Flat land bonus (25%) - plains and beaches are easier to build on
    let flat_land_score = match biome {
        TileType::Plains => 1.0,
        TileType::Beach => 0.9,
        TileType::Forest => 0.7,
        TileType::Desert => 0.6,
        TileType::Plateau => 0.4,
        _ => 0.3,
    };

    0.40 * culture_score
        + 0.25 * flat_land_score
        + 0.20 * resource_score
        + 0.10 * water_score
        + 0.05 * defense_score
}

/// Check if a position respects minimum spacing from existing settlements.
fn respects_spacing(settlements: &[City], pos: Point2D, min_distance: f64) -> bool {
    for city in settlements {
        let dx = city.position.x - pos.x;
        let dy = city.position.y - pos.y;
        let dist = (dx * dx + dy * dy).sqrt();
        if dist < min_distance {
            return false;
        }
    }
    true
}

/// Find local maxima in suitability across the map.
fn find_local_maxima(
    biome_map: &BiomeMap,
    cultures: &[Culture],
    step: usize,
) -> Vec<SettlementCandidate> {
    let mut candidates = Vec::new();

    // Sample at regular intervals
    for y in (0..biome_map.height).step_by(step) {
        for x in (0..biome_map.width).step_by(step) {
            let Some(biome) = biome_map.get_biome(x, y) else {
                continue;
            };

            // Skip water
            if matches!(biome, TileType::Sea | TileType::White) {
                continue;
            }

            let temperature = biome_map.get_temperature(x, y).unwrap_or(20.0);
            let continentalness = biome_map.get_continentalness(x, y).unwrap_or(0.1);

            // Find best culture for this location
            let (best_culture, _) = find_best_culture(cultures, biome, temperature, continentalness);
            let culture = cultures
                .iter()
                .find(|c| c.culture_type == best_culture)
                .unwrap();

            let suitability = calculate_site_suitability(biome_map, x, y, culture);

            if suitability > SETTLEMENT_THRESHOLD {
                // Check if this is a local maximum
                let is_local_max = is_local_maximum(biome_map, x, y, culture, suitability, step);

                if is_local_max {
                    candidates.push(SettlementCandidate {
                        position: Point2D::new(x as f64, y as f64),
                        suitability,
                        culture_type: best_culture,
                        biome,
                        temperature,
                        continentalness,
                    });
                }
            }
        }
    }

    candidates
}

/// Check if a position is a local maximum in suitability.
fn is_local_maximum(
    biome_map: &BiomeMap,
    x: usize,
    y: usize,
    culture: &Culture,
    current_suitability: f64,
    radius: usize,
) -> bool {
    let x_start = x.saturating_sub(radius);
    let x_end = (x + radius).min(biome_map.width - 1);
    let y_start = y.saturating_sub(radius);
    let y_end = (y + radius).min(biome_map.height - 1);

    for ny in y_start..=y_end {
        for nx in x_start..=x_end {
            if nx == x && ny == y {
                continue;
            }

            let neighbor_suitability = calculate_site_suitability(biome_map, nx, ny, culture);
            if neighbor_suitability > current_suitability {
                return false;
            }
        }
    }

    true
}

/// Determine city tier based on suitability and strategic value.
fn determine_tier(
    suitability: f64,
    is_capital: bool,
    candidate: &SettlementCandidate,
) -> CityTier {
    if is_capital {
        return CityTier::Capital;
    }

    // Higher suitability = more important settlement
    if suitability > 0.7 {
        CityTier::Town
    } else if suitability > 0.5 {
        // Coastal towns are often important
        if matches!(candidate.biome, TileType::Beach) {
            CityTier::Town
        } else {
            CityTier::Village
        }
    } else {
        CityTier::Village
    }
}

/// Generate a procedural name based on culture and biome.
fn generate_name(culture: CultureType, biome: TileType, tier: CityTier, rng: &mut impl Rng) -> String {
    let prefixes = match culture {
        CultureType::TwilightDweller => &["New ", "Old ", "Great ", ""][..],
        CultureType::FrostKin => &["North", "Ice", "Frost", "Winter"][..],
        CultureType::SunForged => &["Sun", "Gold", "Bright", "Fire"][..],
        CultureType::TideWalker => &["Port ", "Sea", "Harbor ", ""][..],
        CultureType::StoneBorn => &["High", "Stone", "Iron", "Mount "][..],
    };

    let roots = match biome {
        TileType::Plains => &["field", "dale", "meadow", "green"][..],
        TileType::Forest => &["wood", "grove", "glen", "shade"][..],
        TileType::Mountain => &["peak", "crag", "tor", "hold"][..],
        TileType::Beach => &["haven", "cove", "bay", "shore"][..],
        TileType::Desert => &["oasis", "dune", "sand", "mirage"][..],
        TileType::Snow => &["frost", "ice", "white", "cold"][..],
        _ => &["town", "stead", "burg", "haven"][..],
    };

    let suffixes = match tier {
        CityTier::Capital => &[" City", " Capital", "", " Prime"][..],
        CityTier::Town => &["ton", "ville", "burg", ""][..],
        CityTier::Village => &["", " Village", " Hamlet", ""][..],
    };

    let prefix = prefixes[rng.gen_range(0..prefixes.len())];
    let root = roots[rng.gen_range(0..roots.len())];
    let suffix = suffixes[rng.gen_range(0..suffixes.len())];

    format!("{}{}{}", prefix, root, suffix)
}

/// Place settlements across the map.
pub fn place_settlements(
    biome_map: &BiomeMap,
    cultures: &[Culture],
    seed: u32,
    max_settlements: usize,
) -> PlacementResult {
    let mut rng = ChaCha8Rng::seed_from_u64(seed as u64);
    let mut settlements = Vec::new();
    let mut next_id = 1u32;

    // Find candidate locations (sample every 8 pixels for performance)
    let mut candidates = find_local_maxima(biome_map, cultures, 8);
    let candidates_evaluated = candidates.len();

    // Sort by suitability (best first)
    candidates.sort_by(|a, b| b.suitability.partial_cmp(&a.suitability).unwrap());

    // Track which cultures have capitals
    let mut has_capital: std::collections::HashSet<CultureType> = std::collections::HashSet::new();

    // Place settlements
    for candidate in candidates {
        if settlements.len() >= max_settlements {
            break;
        }

        // Check spacing constraint
        let culture = cultures
            .iter()
            .find(|c| c.culture_type == candidate.culture_type)
            .unwrap();
        let min_dist = culture.traits.settlement_spacing.max(MIN_SETTLEMENT_DISTANCE);

        if !respects_spacing(&settlements, candidate.position, min_dist) {
            continue;
        }

        // Determine if this should be a capital
        let is_capital = !has_capital.contains(&candidate.culture_type);
        if is_capital {
            has_capital.insert(candidate.culture_type);
        }

        let tier = determine_tier(candidate.suitability, is_capital, &candidate);
        let name = generate_name(candidate.culture_type, candidate.biome, tier, &mut rng);

        settlements.push(City::new(next_id, name, candidate.position, tier));
        next_id += 1;
    }

    PlacementResult {
        candidates_placed: settlements.len(),
        candidates_evaluated,
        settlements,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suitability_rejects_water() {
        // Water should never be suitable
        let culture = Culture::twilight_dweller();
        // Sea tiles have continentalness below sea level
        let score = culture.calculate_suitability(TileType::Sea, 20.0, -0.5);
        assert!(score < 0.3);
    }

    #[test]
    fn spacing_check_works() {
        let cities = vec![
            City::new(1, "Test".into(), Point2D::new(100.0, 100.0), CityTier::Town),
        ];

        // Too close
        assert!(!respects_spacing(&cities, Point2D::new(110.0, 100.0), 50.0));
        // Far enough
        assert!(respects_spacing(&cities, Point2D::new(200.0, 100.0), 50.0));
    }

    #[test]
    fn placement_is_deterministic() {
        let biome_map = BiomeMap::generate(42, 256, 128);
        let cultures = Culture::all_defaults();

        let result1 = place_settlements(&biome_map, &cultures, 123, 20);
        let result2 = place_settlements(&biome_map, &cultures, 123, 20);

        assert_eq!(result1.settlements.len(), result2.settlements.len());
        for (a, b) in result1.settlements.iter().zip(result2.settlements.iter()) {
            assert_eq!(a.position.x, b.position.x);
            assert_eq!(a.position.y, b.position.y);
        }
    }
}

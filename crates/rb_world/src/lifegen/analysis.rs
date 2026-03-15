use rb_core::{TerrainQuery, TileType};
use rayon::prelude::*;
use std::collections::VecDeque;

/// Precompute river distance field via BFS.
///
/// Returns a `Vec<f32>` of length `w * h` where each value is the
/// approximate Euclidean distance to the nearest river pixel.
/// River pixels have distance 0.0.
pub fn compute_river_distance_field(terrain: &dyn TerrainQuery) -> Vec<f32> {
    let w = terrain.width();
    let h = terrain.height();
    let mut dist = vec![f32::MAX; w * h];
    let mut queue = VecDeque::new();

    // Seed BFS with river pixels
    for y in 0..h {
        for x in 0..w {
            if terrain.is_river(x, y) {
                let idx = y * w + x;
                dist[idx] = 0.0;
                queue.push_back((x, y));
            }
        }
    }

    // BFS (4-connected)
    while let Some((x, y)) = queue.pop_front() {
        let current_dist = dist[y * w + x];
        let offsets: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
        for &(dx, dy) in &offsets {
            let nx = x as i32 + dx;
            let ny = y as i32 + dy;
            if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                continue;
            }
            let nux = nx as usize;
            let nuy = ny as usize;
            let ni = nuy * w + nux;
            let new_dist = current_dist + 1.0;
            if new_dist < dist[ni] {
                dist[ni] = new_dist;
                queue.push_back((nux, nuy));
            }
        }
    }

    // Parallel refinement pass: tighten with diagonal cost
    let snapshot = dist.clone();
    dist.par_chunks_mut(w)
        .enumerate()
        .for_each(|(y, row)| {
            for x in 0..w {
                let diag_offsets: [(i32, i32, f32); 8] = [
                    (-1, -1, 1.414), (0, -1, 1.0), (1, -1, 1.414),
                    (-1,  0, 1.0),                  (1,  0, 1.0),
                    (-1,  1, 1.414), (0,  1, 1.0),  (1,  1, 1.414),
                ];
                for &(dx, dy, cost) in &diag_offsets {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        continue;
                    }
                    let ni = ny as usize * w + nx as usize;
                    let candidate = snapshot[ni] + cost;
                    if candidate < row[x] {
                        row[x] = candidate;
                    }
                }
            }
        });

    dist
}

/// Compute habitability score [0.0, 1.0] for every meso pixel.
///
/// Four weighted factors:
/// - Temperature comfort (35%): sigmoid peaking at 15 C
/// - Water availability (30%): river proximity + humidity + drainage
/// - Elevation comfort (20%): sweet spot 0.0-0.3
/// - Terrain stability (15%): low slope, low tectonic stress
///
/// Ocean pixels are always 0.0.
pub fn compute_habitability(terrain: &dyn TerrainQuery, river_dist: &[f32]) -> Vec<f32> {
    let w = terrain.width();
    let h = terrain.height();
    let mut grid = vec![0.0f32; w * h];

    grid.par_chunks_mut(w)
        .enumerate()
        .for_each(|(y, row)| {
            for x in 0..w {
                if terrain.is_ocean(x, y) {
                    row[x] = 0.0;
                    continue;
                }

                // --- Temperature comfort (35%) ---
                let temp = terrain.temperature_at(x, y);
                let temp_score = (1.0 - ((temp - 15.0) / 35.0).powi(2)).clamp(0.0, 1.0);

                // --- Water availability (30%) ---
                let humidity = terrain.humidity_at(x, y);
                let drainage = terrain.drainage_at(x, y);

                let idx = y * w + x;
                let rd = river_dist[idx];
                let river_bonus = if rd < 1.0 {
                    0.4
                } else if rd < 12.0 {
                    0.3 * (1.0 - rd as f64 / 12.0)
                } else {
                    0.0
                };

                let drainage_score = (drainage / 1000.0).min(1.0) * 0.1;
                let water_score = (humidity * 0.5 + river_bonus + drainage_score).min(1.0);

                // --- Elevation comfort (20%) ---
                let height = terrain.heightmap_at(x, y);
                let elev_score = if height < 0.0 {
                    0.2
                } else if height <= 0.3 {
                    1.0
                } else if height <= 0.6 {
                    // Linear decline from 1.0 at 0.3 to 0.5 at 0.6
                    1.0 - (height - 0.3) / 0.3 * 0.5
                } else {
                    0.3 * (1.0 - height)
                };

                // --- Terrain stability (15%) ---
                let slope = terrain.slope_at(x, y);
                let tectonic = terrain.tectonic_at(x, y);
                let slope_penalty = (slope * 5.0).min(1.0);
                let tectonic_penalty = (tectonic * 0.5).min(0.5);
                let stability_score = (1.0 - slope_penalty - tectonic_penalty).max(0.0);

                let composite = temp_score * 0.35
                    + water_score * 0.30
                    + elev_score * 0.20
                    + stability_score * 0.15;

                row[x] = composite.clamp(0.0, 1.0) as f32;
            }
        });

    grid
}

/// Compute navigation cost [0.0, 1.0] for every meso pixel.
///
/// 0.0 = impassable, 1.0 = trivially easy traversal.
/// Based on biome traversability minus slope and elevation penalties.
/// River crossings are expensive (bridge cost), but near-river pixels
/// get a bonus (valley roads).
/// Ocean pixels are 0.0.
pub fn compute_navigation_cost(terrain: &dyn TerrainQuery, river_dist: &[f32]) -> Vec<f32> {
    let w = terrain.width();
    let h = terrain.height();
    let mut grid = vec![0.0f32; w * h];

    grid.par_chunks_mut(w)
        .enumerate()
        .for_each(|(y, row)| {
            for x in 0..w {
                if terrain.is_ocean(x, y) {
                    row[x] = 0.0;
                    continue;
                }

                let biome = terrain.biome_at(x, y);
                let biome_cost = biome_traversability(biome) as f64;

                let slope = terrain.slope_at(x, y);
                let slope_penalty = (slope * 3.0).min(0.7);

                let height = terrain.heightmap_at(x, y);
                let elev_penalty = if height > 0.7 {
                    (height - 0.7) * 2.0
                } else {
                    0.0
                };

                let mut result = (biome_cost - slope_penalty - elev_penalty).clamp(0.0, 1.0);

                // River interaction: crossing is expensive, following alongside is cheap
                let idx = y * w + x;
                let rd = river_dist[idx];
                if rd < 1.0 {
                    // On a river pixel: heavy crossing penalty (bridge cost)
                    result *= 0.15;
                } else if rd < 20.0 {
                    // Near river: valley bonus — roads prefer to follow rivers
                    let bonus = 0.15 * (1.0 - rd as f64 / 20.0);
                    result = (result + bonus).min(1.0);
                }

                row[x] = result as f32;
            }
        });

    grid
}

/// Compute resource desirability [0.0, 1.0] for every meso pixel.
///
/// Geology-driven resource potential:
/// - Mineral score (40%): tectonic activity
/// - Rock score (25%): rock hardness
/// - Exposure score (20%): erosion exposes resources
/// - Fertility score (15%): humidity-driven agricultural potential
pub fn compute_resource_desirability(terrain: &dyn TerrainQuery) -> Vec<f32> {
    let w = terrain.width();
    let h = terrain.height();
    let mut grid = vec![0.0f32; w * h];

    grid.par_chunks_mut(w)
        .enumerate()
        .for_each(|(y, row)| {
            for x in 0..w {
                let tectonic = terrain.tectonic_at(x, y);
                let rock_hardness = terrain.rock_hardness_at(x, y);
                let erosion = terrain.erosion_at(x, y);
                let humidity = terrain.humidity_at(x, y);

                let mineral_score = (tectonic * 1.5).min(1.0);
                let rock_score = rock_hardness;
                let exposure_score = (erosion * 2.0).min(1.0);
                let fertility_score = (humidity * (1.0 - erosion * 0.5)).max(0.0).min(1.0);

                let composite = mineral_score * 0.40
                    + rock_score * 0.25
                    + exposure_score * 0.20
                    + fertility_score * 0.15;

                row[x] = composite.clamp(0.0, 1.0) as f32;
            }
        });

    grid
}

/// Map every `TileType` variant to a traversability score [0.0, 1.0].
///
/// 0.0 = impassable, 1.0 = trivially easy to cross.
fn biome_traversability(biome: TileType) -> f32 {
    match biome {
        // Easiest terrain
        TileType::Plains | TileType::Meadow | TileType::Oasis => 1.0,

        // Open, mostly flat
        TileType::Beach
        | TileType::Steppe
        | TileType::Savanna
        | TileType::AlpineMeadow => 0.85,

        // Forested -- passable but slow
        TileType::Forest
        | TileType::Woodland
        | TileType::DeciduousForest
        | TileType::TemperateRainforest
        | TileType::Taiga
        | TileType::SubtropicalForest
        | TileType::CloudForest => 0.7,

        // River crossing
        TileType::River => 0.6,

        // Coastal rocky terrain
        TileType::Mangrove | TileType::RockyCoast | TileType::SeaCliff => 0.5,

        // Scrubby / rough
        TileType::Scrubland | TileType::Badlands | TileType::Hamada => 0.5,

        // Harsh but traversable
        TileType::Desert
        | TileType::Sahara
        | TileType::Jungle
        | TileType::Marsh
        | TileType::FrozenBog
        | TileType::DryWoodland
        | TileType::Thornland
        | TileType::HighlandSavanna => 0.4,

        // Frozen / sandy
        TileType::Snow | TileType::Tundra | TileType::IceSheet | TileType::Glacier => 0.3,
        TileType::Erg | TileType::SaltFlat => 0.3,

        // High elevation
        TileType::Mountain | TileType::Plateau => 0.2,

        // Volcanic / extreme heat
        TileType::Volcanic
        | TileType::LavaField
        | TileType::MoltenWaste
        | TileType::ScorchedRock => 0.1,

        // Ocean types -- impassable by land
        TileType::Sea
        | TileType::ShallowSea
        | TileType::ContinentalShelf
        | TileType::DeepOcean
        | TileType::OceanTrench
        | TileType::OceanRidge
        | TileType::White => 0.0,

        // Coral reef -- underwater
        TileType::CoralReef => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockTerrain {
        width: usize,
        height: usize,
        rivers: Vec<bool>,
    }

    impl TerrainQuery for MockTerrain {
        fn width(&self) -> usize { self.width }
        fn height(&self) -> usize { self.height }
        fn heightmap_at(&self, _x: usize, _y: usize) -> f64 { 0.1 }
        fn biome_at(&self, _x: usize, _y: usize) -> TileType { TileType::Plains }
        fn temperature_at(&self, _x: usize, _y: usize) -> f64 { 15.0 }
        fn humidity_at(&self, _x: usize, _y: usize) -> f64 { 0.5 }
        fn continentalness_at(&self, _x: usize, _y: usize) -> f64 { 0.5 }
        fn erosion_at(&self, _x: usize, _y: usize) -> f64 { 0.1 }
        fn light_level_at(&self, _x: usize, _y: usize) -> f64 { 0.5 }
        fn rock_hardness_at(&self, _x: usize, _y: usize) -> f64 { 0.5 }
        fn river_at(&self, x: usize, y: usize) -> f64 {
            if y < self.height && x < self.width && self.rivers[y * self.width + x] { 1.0 } else { 0.0 }
        }
        fn drainage_at(&self, _x: usize, _y: usize) -> f64 { 0.0 }
        fn tectonic_at(&self, _x: usize, _y: usize) -> f64 { 0.0 }
        fn peaks_valleys_at(&self, _x: usize, _y: usize) -> f64 { 0.0 }
        fn aridity_at(&self, _x: usize, _y: usize) -> f64 { 0.0 }
        fn slope_at(&self, _x: usize, _y: usize) -> f64 { 0.0 }
        fn is_ocean(&self, _x: usize, _y: usize) -> bool { false }
        fn is_river(&self, x: usize, y: usize) -> bool {
            y < self.height && x < self.width && self.rivers[y * self.width + x]
        }
    }

    #[test]
    fn river_distance_field_correctness() {
        let w = 100;
        let h = 100;
        let mut rivers = vec![false; w * h];
        // Place river along row 50
        for x in 0..w {
            rivers[50 * w + x] = true;
        }
        let terrain = MockTerrain { width: w, height: h, rivers };
        let dist = compute_river_distance_field(&terrain);

        // River pixels should have dist 0
        for x in 0..w {
            assert_eq!(dist[50 * w + x], 0.0, "river pixel at ({}, 50) should be 0", x);
        }
        // Adjacent row should be ~1
        for x in 0..w {
            let d = dist[49 * w + x];
            assert!(d <= 1.01, "pixel at ({}, 49) dist={} should be ~1", x, d);
        }
        // Distance should increase monotonically away from river
        for x in [10, 50, 90] {
            for y in 0..49 {
                assert!(
                    dist[y * w + x] >= dist[(y + 1) * w + x],
                    "dist should decrease toward river at ({}, {}): {} vs {}",
                    x, y, dist[y * w + x], dist[(y + 1) * w + x]
                );
            }
        }
    }

    /// Verify every TileType variant is handled and returns a valid score.
    #[test]
    fn biome_traversability_all_variants_valid() {
        let all_biomes = [
            TileType::Sea,
            TileType::ShallowSea,
            TileType::ContinentalShelf,
            TileType::DeepOcean,
            TileType::OceanTrench,
            TileType::OceanRidge,
            TileType::River,
            TileType::Beach,
            TileType::Mangrove,
            TileType::CoralReef,
            TileType::RockyCoast,
            TileType::SeaCliff,
            TileType::White,
            TileType::Glacier,
            TileType::Snow,
            TileType::IceSheet,
            TileType::FrozenBog,
            TileType::Tundra,
            TileType::Taiga,
            TileType::AlpineMeadow,
            TileType::Plains,
            TileType::Meadow,
            TileType::Forest,
            TileType::DeciduousForest,
            TileType::TemperateRainforest,
            TileType::Woodland,
            TileType::Scrubland,
            TileType::Marsh,
            TileType::Steppe,
            TileType::Mountain,
            TileType::Plateau,
            TileType::SubtropicalForest,
            TileType::DryWoodland,
            TileType::Thornland,
            TileType::HighlandSavanna,
            TileType::CloudForest,
            TileType::Savanna,
            TileType::Jungle,
            TileType::Desert,
            TileType::Sahara,
            TileType::Erg,
            TileType::Hamada,
            TileType::SaltFlat,
            TileType::Badlands,
            TileType::Oasis,
            TileType::Volcanic,
            TileType::LavaField,
            TileType::MoltenWaste,
            TileType::ScorchedRock,
        ];

        for biome in &all_biomes {
            let score = biome_traversability(*biome);
            assert!(
                (0.0..=1.0).contains(&score),
                "biome_traversability({:?}) = {} is out of [0.0, 1.0]",
                biome,
                score
            );
        }
    }

    #[test]
    fn ocean_types_are_impassable() {
        assert_eq!(biome_traversability(TileType::Sea), 0.0);
        assert_eq!(biome_traversability(TileType::DeepOcean), 0.0);
        assert_eq!(biome_traversability(TileType::CoralReef), 0.0);
    }

    #[test]
    fn plains_are_easiest() {
        assert_eq!(biome_traversability(TileType::Plains), 1.0);
        assert_eq!(biome_traversability(TileType::Meadow), 1.0);
    }
}

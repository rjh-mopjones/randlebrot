use rb_core::{TerrainQuery, TileType};
use rayon::prelude::*;

/// Compute habitability score [0.0, 1.0] for every meso pixel.
///
/// Four weighted factors:
/// - Temperature comfort (35%): sigmoid peaking at 15 C
/// - Water availability (30%): river proximity + humidity + drainage
/// - Elevation comfort (20%): sweet spot 0.0-0.3
/// - Terrain stability (15%): low slope, low tectonic stress
///
/// Ocean pixels are always 0.0.
pub fn compute_habitability(terrain: &dyn TerrainQuery) -> Vec<f32> {
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

                let river_bonus = if terrain.is_river(x, y) {
                    0.4
                } else {
                    // Scan 8-pixel radius for nearby river
                    let mut best_dist = f64::MAX;
                    let r: i64 = 8;
                    'scan: for dy in -r..=r {
                        for dx in -r..=r {
                            let nx = x as i64 + dx;
                            let ny = y as i64 + dy;
                            if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                                continue;
                            }
                            if terrain.is_river(nx as usize, ny as usize) {
                                let dist = ((dx * dx + dy * dy) as f64).sqrt();
                                if dist < best_dist {
                                    best_dist = dist;
                                }
                                // Early exit: found adjacent river, can't do better than ~1
                                if dist <= 1.0 {
                                    break 'scan;
                                }
                            }
                        }
                    }
                    if best_dist < 12.0 {
                        0.3 * (1.0 - best_dist / 12.0)
                    } else {
                        0.0
                    }
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
/// Ocean pixels are 0.0.
pub fn compute_navigation_cost(terrain: &dyn TerrainQuery) -> Vec<f32> {
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

                let result = (biome_cost - slope_penalty - elev_penalty).clamp(0.0, 1.0);
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

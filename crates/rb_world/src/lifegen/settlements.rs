use crate::lifegen_data::{PoliticalState, Province, SettlementSeed, SettlementTier, SizeClass};
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use rb_core::TerrainQuery;
use std::collections::HashSet;

/// Place settlement seeds within provinces based on habitability, area, and terrain.
///
/// `capital_provinces` is a slice of `(faction_id, province_id)` pairs identifying
/// which provinces are faction capitals (from `factions::place_capitals`).
pub fn place_settlements(
    provinces: &[Province],
    province_ids: &[u16],
    _terrain: &dyn TerrainQuery,
    habitability: &[f32],
    width: usize,
    height: usize,
    seed: u32,
    capital_provinces: &[(u32, u16)],
) -> Vec<SettlementSeed> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed as u64 ^ 0x5E77_1E00);
    let mut settlements: Vec<SettlementSeed> = Vec::new();
    let mut next_id: u32 = 1;

    // Build a set of capital province IDs for fast lookup
    let capital_set: HashSet<u16> = capital_provinces.iter().map(|&(_, pid)| pid).collect();

    for province in provinces {
        // Determine how many settlements this province gets
        let max_settlements = settlement_count_for_province(province);
        if max_settlements == 0 {
            continue;
        }

        // Find the best pixels within this province by habitability
        let best_positions =
            find_best_positions(province, province_ids, habitability, width, height, max_settlements);

        let is_capital = capital_set.contains(&province.id);

        for (slot_idx, (px, py)) in best_positions.iter().enumerate() {
            let position = (*px as f64, *py as f64);

            let size_class = determine_size_class(
                province,
                is_capital && slot_idx == 0,
                &mut rng,
            );

            let tier = tier_from_size_class(size_class);

            // Settlements in unclaimed/uninhabited provinces are downgraded
            let (size_class, tier) = if matches!(
                province.political_state,
                PoliticalState::Unclaimed | PoliticalState::Uninhabited
            ) {
                let capped_size = match size_class {
                    SizeClass::Metropolis | SizeClass::City | SizeClass::Town => SizeClass::Village,
                    SizeClass::Village => SizeClass::Outpost,
                    other => other,
                };
                (capped_size, tier_from_size_class(capped_size))
            } else {
                (size_class, tier)
            };

            settlements.push(SettlementSeed {
                id: next_id,
                position,
                province_id: province.id,
                size_class,
                tier,
            });
            next_id += 1;
        }
    }

    settlements
}

/// Determine how many settlements a province gets.
/// Every province gets a base of 3; high habitability grants bonus settlements.
/// Tiny provinces (area_px < 200) are capped at 1 to avoid cramming.
fn settlement_count_for_province(province: &Province) -> usize {
    if province.area_px < 200 {
        return 1;
    }
    if province.habitability > 0.7 {
        5
    } else if province.habitability > 0.5 {
        4
    } else {
        3
    }
}

/// Find the N highest-habitability pixels within a province, each at least 60px apart.
///
/// Returns pixel coordinates `(x, y)` in meso pixel space.
fn find_best_positions(
    province: &Province,
    province_ids: &[u16],
    habitability: &[f32],
    width: usize,
    height: usize,
    count: usize,
) -> Vec<(usize, usize)> {
    let pid = province.id;

    // Collect all pixels belonging to this province with their habitability
    // For performance, we don't need to store all - just find top candidates.
    // We'll do a single pass collecting the top-scoring pixels.
    let mut best: Vec<(f32, usize, usize)> = Vec::new();

    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            if province_ids[idx] == pid {
                let hab = habitability[idx];
                best.push((hab, x, y));
            }
        }
    }

    // Sort by habitability descending
    best.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    let min_spacing_sq: f64 = 60.0 * 60.0;
    let mut selected: Vec<(usize, usize)> = Vec::new();

    for (_, x, y) in &best {
        if selected.len() >= count {
            break;
        }

        // Check spacing from already-selected positions
        let too_close = selected.iter().any(|&(sx, sy)| {
            let dx = *x as f64 - sx as f64;
            let dy = *y as f64 - sy as f64;
            dx * dx + dy * dy < min_spacing_sq
        });

        if !too_close {
            selected.push((*x, *y));
        }
    }

    selected
}

/// Determine the size class for a settlement based on province habitability.
/// Habitability controls size, not existence — even desolate provinces get Ruins.
fn determine_size_class(
    province: &Province,
    is_capital_settlement: bool,
    _rng: &mut ChaCha8Rng,
) -> SizeClass {
    if is_capital_settlement {
        return SizeClass::Metropolis;
    }

    if province.habitability > 0.7 {
        SizeClass::City
    } else if province.habitability > 0.5 && province.is_river_junction {
        SizeClass::City
    } else if province.habitability > 0.3 {
        SizeClass::Town
    } else if province.habitability > 0.15 {
        SizeClass::Village
    } else if province.habitability > 0.05 {
        SizeClass::Outpost
    } else {
        SizeClass::Ruins
    }
}

/// Map a SizeClass to its corresponding SettlementTier.
fn tier_from_size_class(size_class: SizeClass) -> SettlementTier {
    match size_class {
        SizeClass::Metropolis => SettlementTier::Capital,
        SizeClass::City => SettlementTier::Major,
        SizeClass::Town => SettlementTier::Minor,
        SizeClass::Village => SettlementTier::Outpost,
        SizeClass::Outpost => SettlementTier::Camp,
        SizeClass::Ruins => SettlementTier::Ruins,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rb_core::TileType;

    fn make_province(id: u16, hab: f32, area: u32) -> Province {
        Province {
            id,
            site: (100.0, 100.0),
            biome: TileType::Plains,
            habitability: hab,
            area_px: area,
            is_coastal: false,
            is_river_junction: false,
            elevation_mean: 0.1,
            terrain_cost: 1.0,
            political_state: PoliticalState::Claimed { faction_id: 1 },
        }
    }

    #[test]
    fn settlement_count_tiers() {
        // High hab -> 5
        assert_eq!(
            settlement_count_for_province(&make_province(1, 0.8, 2000)),
            5
        );
        // Medium-high hab -> 4
        assert_eq!(
            settlement_count_for_province(&make_province(1, 0.6, 600)),
            4
        );
        // Low hab -> 3 (base)
        assert_eq!(
            settlement_count_for_province(&make_province(1, 0.2, 500)),
            3
        );
        // Very low hab still gets 3
        assert_eq!(
            settlement_count_for_province(&make_province(1, 0.01, 500)),
            3
        );
        // Tiny province capped to 1
        assert_eq!(
            settlement_count_for_province(&make_province(1, 0.8, 100)),
            1
        );
    }

    #[test]
    fn tier_mapping() {
        assert_eq!(tier_from_size_class(SizeClass::Metropolis), SettlementTier::Capital);
        assert_eq!(tier_from_size_class(SizeClass::City), SettlementTier::Major);
        assert_eq!(tier_from_size_class(SizeClass::Town), SettlementTier::Minor);
        assert_eq!(tier_from_size_class(SizeClass::Village), SettlementTier::Outpost);
        assert_eq!(tier_from_size_class(SizeClass::Outpost), SettlementTier::Camp);
        assert_eq!(tier_from_size_class(SizeClass::Ruins), SettlementTier::Ruins);
    }

    #[test]
    fn unclaimed_provinces_capped_to_village() {
        let size = SizeClass::City;
        // Simulating the cap logic from place_settlements
        let capped = match size {
            SizeClass::Metropolis | SizeClass::City | SizeClass::Town => SizeClass::Village,
            SizeClass::Village => SizeClass::Outpost,
            other => other,
        };
        assert_eq!(capped, SizeClass::Village);
    }

    #[test]
    fn very_low_hab_produces_ruins() {
        let province = make_province(1, 0.02, 500);
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let size = determine_size_class(&province, false, &mut rng);
        assert_eq!(size, SizeClass::Ruins);
    }

    #[test]
    fn find_positions_respects_spacing() {
        let width = 200;
        let height = 1;
        let pid = 1u16;
        let province_ids: Vec<u16> = vec![pid; width * height];
        // Create a habitability gradient: highest at x=0, then x=10, then x=100
        let mut habitability = vec![0.1f32; width * height];
        habitability[0] = 0.9;
        habitability[10] = 0.85; // Only 10 pixels from first, should be skipped
        habitability[100] = 0.8; // 100 pixels from first, OK

        let province = Province {
            id: pid,
            site: (100.0, 0.0),
            biome: TileType::Plains,
            habitability: 0.8,
            area_px: 200,
            is_coastal: false,
            is_river_junction: false,
            elevation_mean: 0.1,
            terrain_cost: 1.0,
            political_state: PoliticalState::Claimed { faction_id: 1 },
        };

        let positions = find_best_positions(&province, &province_ids, &habitability, width, height, 3);
        // First two must be x=0 and x=100 (x=10 skipped due to 60px spacing)
        assert!(positions.len() >= 2);
        assert_eq!(positions[0], (0, 0));
        assert_eq!(positions[1], (100, 0));
        // All positions must respect 60px minimum spacing
        for i in 0..positions.len() {
            for j in (i + 1)..positions.len() {
                let dx = positions[i].0 as f64 - positions[j].0 as f64;
                let dy = positions[i].1 as f64 - positions[j].1 as f64;
                assert!(dx * dx + dy * dy >= 60.0 * 60.0);
            }
        }
    }

    #[test]
    fn capital_settlement_is_metropolis() {
        let province = make_province(1, 0.8, 2000);
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let size = determine_size_class(&province, true, &mut rng);
        assert_eq!(size, SizeClass::Metropolis);
    }

    #[test]
    fn non_capital_high_hab_is_city() {
        let province = make_province(1, 0.8, 2000);
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let size = determine_size_class(&province, false, &mut rng);
        assert_eq!(size, SizeClass::City);
    }

    #[test]
    fn mid_hab_river_junction_is_city() {
        let mut province = make_province(1, 0.6, 2000);
        province.is_river_junction = true;
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let size = determine_size_class(&province, false, &mut rng);
        assert_eq!(size, SizeClass::City);
    }
}

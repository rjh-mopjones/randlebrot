use crate::lifegen_data::{FactionData, PoliticalState, Province};
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};

/// Select capital provinces for factions.
///
/// If `num_factions` is 0, auto-calculate from the number of habitable provinces.
/// Returns `Vec<(faction_id, province_id)>` with faction IDs starting at 1.
pub fn place_capitals(
    provinces: &[Province],
    num_factions: usize,
    seed: u32,
) -> Vec<(u32, u16)> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed as u64);

    let habitable_count = provinces
        .iter()
        .filter(|p| p.habitability > 0.15)
        .count();

    let target = if num_factions == 0 {
        5 + (habitable_count / 150).min(10)
    } else {
        num_factions
    };

    // Score candidates: must have habitability > 0.5
    let mut candidates: Vec<(u16, f32)> = provinces
        .iter()
        .filter(|p| p.habitability > 0.5)
        .map(|p| {
            let river_bonus = if p.is_river_junction { 0.2 } else { 0.0 };
            let coastal_bonus = if p.is_coastal { 0.1 } else { 0.0 };
            let score = p.habitability + river_bonus + coastal_bonus;
            (p.id, score)
        })
        .collect();

    // Sort by score descending, deterministic tiebreak on province id
    candidates.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });

    let min_distance_sq: f64 = 300.0 * 300.0;
    let mut selected: Vec<(u32, u16)> = Vec::new();
    let mut selected_sites: Vec<(f64, f64)> = Vec::new();
    let mut faction_id: u32 = 1;

    for (pid, _score) in &candidates {
        if selected.len() >= target {
            break;
        }

        // Find the province to get its site
        let province = match provinces.iter().find(|p| p.id == *pid) {
            Some(p) => p,
            None => continue,
        };

        // Check minimum distance to all existing capitals
        let too_close = selected_sites.iter().any(|site| {
            let dx = province.site.0 - site.0;
            let dy = province.site.1 - site.1;
            dx * dx + dy * dy < min_distance_sq
        });

        if too_close {
            continue;
        }

        selected.push((faction_id, *pid));
        selected_sites.push(province.site);
        faction_id += 1;
    }

    selected
}

/// Expand factions outward from capitals through neighboring provinces.
///
/// Uses priority-queue flood fill with terrain-cost-based expansion.
/// Provinces with habitability < 0.1 are never claimed (stay Uninhabited).
/// After all factions expand, remaining provinces with habitability >= 0.1 become Unclaimed.
pub fn grow_factions(
    provinces: &mut [Province],
    province_ids: &[u16],
    _navigation_cost: &[f32],
    capitals: &[(u32, u16)],
    width: usize,
    height: usize,
    seed: u32,
) {
    let num_provinces = provinces.len();
    if num_provinces == 0 || capitals.is_empty() {
        return;
    }

    let adjacency = build_adjacency(province_ids, width, height, num_provinces);

    let mut rng = ChaCha8Rng::seed_from_u64(seed as u64 ^ 0xFACE);

    // Map province id -> index in provinces slice for fast lookup
    let mut id_to_idx: Vec<usize> = vec![0; num_provinces + 1];
    for (i, p) in provinces.iter().enumerate() {
        if (p.id as usize) < id_to_idx.len() {
            id_to_idx[p.id as usize] = i;
        }
    }

    // Track which provinces have been claimed by any faction
    let mut claimed: Vec<bool> = vec![false; num_provinces + 1];

    // Process each faction: flood fill from capital
    for &(faction_id, capital_pid) in capitals {
        let jitter: f32 = rng.gen::<f32>(); // [0, 1)
        let cost_limit = 80.0 + jitter * 20.0;

        // Priority queue: (Reverse(cost), province_id)
        let mut pq: BinaryHeap<(Reverse<OrderedFloat>, u16)> = BinaryHeap::new();

        // Mark capital
        if (capital_pid as usize) < claimed.len() {
            claimed[capital_pid as usize] = true;
        }
        if let Some(idx) = id_to_idx.get(capital_pid as usize).copied() {
            if let Some(p) = provinces.get_mut(idx) {
                p.political_state = PoliticalState::Claimed { faction_id };
            }
        }

        // Get capital site for distance calculation
        let capital_site = provinces
            .iter()
            .find(|p| p.id == capital_pid)
            .map(|p| p.site)
            .unwrap_or((0.0, 0.0));

        // Seed neighbors of capital into priority queue
        if (capital_pid as usize) < adjacency.len() {
            for &neighbor_id in &adjacency[capital_pid as usize] {
                if (neighbor_id as usize) < claimed.len() && !claimed[neighbor_id as usize] {
                    if let Some(idx) = id_to_idx.get(neighbor_id as usize).copied() {
                        let neighbor = &provinces[idx];
                        if neighbor.habitability < 0.1 {
                            continue;
                        }
                        let dx = neighbor.site.0 - capital_site.0;
                        let dy = neighbor.site.1 - capital_site.1;
                        let dist = (dx * dx + dy * dy).sqrt();
                        let cost =
                            neighbor.terrain_cost * 10.0 + dist as f32 * 0.1;
                        pq.push((Reverse(OrderedFloat(cost)), neighbor_id));
                    }
                }
            }
        }

        // Flood fill
        while let Some((Reverse(OrderedFloat(cost)), pid)) = pq.pop() {
            if cost > cost_limit {
                break;
            }

            if (pid as usize) < claimed.len() && claimed[pid as usize] {
                continue;
            }

            // Check habitability
            let hab = id_to_idx
                .get(pid as usize)
                .and_then(|&idx| provinces.get(idx))
                .map(|p| p.habitability)
                .unwrap_or(0.0);

            if hab < 0.1 {
                continue;
            }

            // Claim this province
            claimed[pid as usize] = true;
            if let Some(idx) = id_to_idx.get(pid as usize).copied() {
                if let Some(p) = provinces.get_mut(idx) {
                    p.political_state = PoliticalState::Claimed { faction_id };
                }
            }

            // Add unclaimed neighbors to queue
            if (pid as usize) < adjacency.len() {
                for &neighbor_id in &adjacency[pid as usize] {
                    if (neighbor_id as usize) < claimed.len() && !claimed[neighbor_id as usize] {
                        if let Some(idx) = id_to_idx.get(neighbor_id as usize).copied() {
                            let neighbor = &provinces[idx];
                            if neighbor.habitability < 0.1 {
                                continue;
                            }
                            let dx = neighbor.site.0 - capital_site.0;
                            let dy = neighbor.site.1 - capital_site.1;
                            let dist = (dx * dx + dy * dy).sqrt();
                            let neighbor_cost =
                                neighbor.terrain_cost * 10.0 + dist as f32 * 0.1;
                            pq.push((
                                Reverse(OrderedFloat(neighbor_cost)),
                                neighbor_id,
                            ));
                        }
                    }
                }
            }
        }
    }

    // Mark remaining unclaimed provinces
    for p in provinces.iter_mut() {
        if matches!(p.political_state, PoliticalState::Uninhabited) {
            if p.habitability >= 0.1 && !claimed.get(p.id as usize).copied().unwrap_or(false) {
                p.political_state = PoliticalState::Unclaimed;
            }
        }
    }
}

/// Create FactionData structs from capitals and provinces.
pub fn build_faction_data(
    provinces: &[Province],
    capitals: &[(u32, u16)],
    seed: u32,
) -> Vec<FactionData> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed as u64 ^ 0xFA01);

    capitals
        .iter()
        .map(|&(faction_id, capital_province)| {
            let name = generate_faction_name(&mut rng);
            let h = faction_id.wrapping_mul(2654435761);
            let colour = [
                ((h) & 0xFF) as u8 | 0x40,
                ((h >> 8) & 0xFF) as u8 | 0x40,
                ((h >> 16) & 0xFF) as u8 | 0x40,
                255,
            ];
            let _ = provinces; // provinces available for future enrichment
            FactionData {
                id: faction_id,
                name,
                colour,
                capital_province,
            }
        })
        .collect()
}

/// Build adjacency list indexed by province id.
///
/// Iterates pixels and checks right and down neighbors to find province borders.
fn build_adjacency(
    province_ids: &[u16],
    width: usize,
    height: usize,
    num_provinces: usize,
) -> Vec<Vec<u16>> {
    let mut adjacency: Vec<HashSet<u16>> = vec![HashSet::new(); num_provinces + 1];

    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            let pid = province_ids[idx];
            if pid == 0 {
                continue;
            }

            // Check right neighbor
            if x + 1 < width {
                let right = province_ids[idx + 1];
                if right != 0 && right != pid {
                    if (pid as usize) < adjacency.len() && (right as usize) < adjacency.len() {
                        adjacency[pid as usize].insert(right);
                        adjacency[right as usize].insert(pid);
                    }
                }
            }

            // Check down neighbor
            if y + 1 < height {
                let down = province_ids[idx + width];
                if down != 0 && down != pid {
                    if (pid as usize) < adjacency.len() && (down as usize) < adjacency.len() {
                        adjacency[pid as usize].insert(down);
                        adjacency[down as usize].insert(pid);
                    }
                }
            }
        }
    }

    // Convert HashSets to Vecs
    adjacency.into_iter().map(|s| s.into_iter().collect()).collect()
}

/// Generate a random faction name from prefix + suffix parts.
fn generate_faction_name(rng: &mut ChaCha8Rng) -> String {
    const PREFIXES: &[&str] = &[
        "Kingdom of",
        "Republic of",
        "Dominion of",
        "The Confederacy of",
        "Empire of",
        "Principality of",
        "Grand Duchy of",
        "The Free State of",
        "Sultanate of",
        "The Commonwealth of",
        "Realm of",
        "Federation of",
    ];

    const FIRST_PARTS: &[&str] = &[
        "Val", "Mor", "Ash", "Kel", "Thar", "Dra", "Sul", "Vor", "Fen", "Bel", "Cor", "Gar",
        "Hal", "Ith", "Kal", "Lor", "Nar", "Pel", "Ren", "Sel", "Tor", "Vyn", "Zan", "Eld",
        "Grim", "Arn", "Bor", "Dun", "Fal", "Gil",
    ];

    const SECOND_PARTS: &[&str] = &[
        "dris", "ren", "ford", "mark", "heim", "gar", "holm", "mund", "rik", "sten", "dale",
        "mere", "fell", "gate", "haven", "keep", "moor", "vale", "wood", "crest", "thorn",
        "wick", "lyn", "shire", "ton", "berg", "land", "ros", "ven", "ane",
    ];

    let prefix = PREFIXES[rng.gen_range(0..PREFIXES.len())];
    let first = FIRST_PARTS[rng.gen_range(0..FIRST_PARTS.len())];
    let second = SECOND_PARTS[rng.gen_range(0..SECOND_PARTS.len())];

    format!("{} {}{}", prefix, first, second)
}

/// Wrapper for f32 that implements Ord, used in the priority queue.
/// NaN is treated as greater than all values (pushed to back).
#[derive(Debug, Clone, Copy, PartialEq)]
struct OrderedFloat(f32);

impl Eq for OrderedFloat {}

impl PartialOrd for OrderedFloat {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrderedFloat {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.partial_cmp(&other.0).unwrap_or_else(|| {
            // NaN handling: NaN sorts last
            if self.0.is_nan() && other.0.is_nan() {
                std::cmp::Ordering::Equal
            } else if self.0.is_nan() {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Less
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rb_core::TileType;

    fn make_province(id: u16, hab: f32, site: (f64, f64)) -> Province {
        Province {
            id,
            site,
            biome: TileType::Plains,
            habitability: hab,
            area_px: 2000,
            is_coastal: false,
            is_river_junction: false,
            elevation_mean: 0.1,
            terrain_cost: 1.0,
            political_state: PoliticalState::Uninhabited,
        }
    }

    #[test]
    fn capitals_respect_min_distance() {
        // Two provinces that are close together: only one should be selected
        let provinces = vec![
            make_province(1, 0.9, (100.0, 100.0)),
            make_province(2, 0.85, (110.0, 110.0)), // ~14 pixels apart, well under 300
            make_province(3, 0.8, (500.0, 500.0)),
        ];
        let caps = place_capitals(&provinces, 3, 42);
        // Should get at most 2 (provinces 1 and 3, skipping 2 because too close to 1)
        assert!(caps.len() <= 2);
        // Both selected should be at least 300 apart
        for i in 0..caps.len() {
            for j in (i + 1)..caps.len() {
                let a = provinces.iter().find(|p| p.id == caps[i].1).unwrap();
                let b = provinces.iter().find(|p| p.id == caps[j].1).unwrap();
                let dx = a.site.0 - b.site.0;
                let dy = a.site.1 - b.site.1;
                assert!(dx * dx + dy * dy >= 300.0 * 300.0);
            }
        }
    }

    #[test]
    fn capitals_require_minimum_habitability() {
        let provinces = vec![
            make_province(1, 0.3, (100.0, 100.0)), // Below 0.5 threshold
            make_province(2, 0.4, (500.0, 500.0)), // Below 0.5 threshold
        ];
        let caps = place_capitals(&provinces, 2, 42);
        assert!(caps.is_empty());
    }

    #[test]
    fn auto_faction_count() {
        // 300 habitable provinces -> 5 + 2 = 7
        let mut provinces = Vec::new();
        for i in 1..=300u16 {
            let x = (i as f64) * 20.0;
            let y = (i as f64) * 10.0;
            provinces.push(make_province(i, 0.6, (x, y)));
        }
        let caps = place_capitals(&provinces, 0, 42);
        // Expected target: 5 + (300/150).min(10) = 7
        // Actual may be fewer due to distance constraint
        assert!(caps.len() <= 7);
        assert!(!caps.is_empty());
    }

    #[test]
    fn grow_factions_claims_reachable() {
        // Simple 4-province grid, capital at province 1
        let mut provinces = vec![
            make_province(1, 0.8, (10.0, 10.0)),
            make_province(2, 0.6, (20.0, 10.0)),
            make_province(3, 0.05, (30.0, 10.0)), // Too low habitability
            make_province(4, 0.5, (40.0, 10.0)),
        ];

        // Simple 1D grid: [1, 1, 2, 2, 3, 3, 4, 4]
        let width = 8;
        let height = 1;
        let province_ids: Vec<u16> = vec![1, 1, 2, 2, 3, 3, 4, 4];
        let nav_cost = vec![1.0f32; width * height];
        let capitals = vec![(1u32, 1u16)];

        grow_factions(
            &mut provinces,
            &province_ids,
            &nav_cost,
            &capitals,
            width,
            height,
            42,
        );

        // Province 1 (capital) should be claimed
        assert!(matches!(
            provinces[0].political_state,
            PoliticalState::Claimed { faction_id: 1 }
        ));
        // Province 2 (adjacent, habitable) should be claimed
        assert!(matches!(
            provinces[1].political_state,
            PoliticalState::Claimed { faction_id: 1 }
        ));
        // Province 3 (hab < 0.1) should stay uninhabited
        assert!(matches!(
            provinces[2].political_state,
            PoliticalState::Uninhabited
        ));
        // Province 4 is blocked by province 3, so it should be unclaimed
        assert!(matches!(
            provinces[3].political_state,
            PoliticalState::Unclaimed
        ));
    }

    #[test]
    fn faction_names_are_non_empty() {
        let mut rng = ChaCha8Rng::seed_from_u64(123);
        for _ in 0..20 {
            let name = generate_faction_name(&mut rng);
            assert!(!name.is_empty());
            assert!(name.contains(' ')); // Should have prefix + name
        }
    }

    #[test]
    fn build_faction_data_produces_correct_count() {
        let provinces = vec![make_province(1, 0.8, (10.0, 10.0))];
        let capitals = vec![(1, 1), (2, 1)];
        let data = build_faction_data(&provinces, &capitals, 42);
        assert_eq!(data.len(), 2);
        assert_eq!(data[0].id, 1);
        assert_eq!(data[1].id, 2);
        assert_eq!(data[0].colour[3], 255);
    }
}

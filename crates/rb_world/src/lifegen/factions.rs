use crate::lifegen_data::{FactionData, PoliticalState, Province};
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

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
        let base = 50;
        let bonus = (habitable_count / 80).min(30);
        base + bonus
    } else {
        num_factions.max(50)
    };

    // Score candidates: must have habitability > 0.35
    let mut candidates: Vec<(u16, f32)> = provinces
        .iter()
        .filter(|p| p.habitability > 0.35)
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

    let min_distance_sq: f64 = 150.0 * 150.0;
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

    // Fallback: if we haven't reached 50 factions, do a second pass with halved spacing
    if selected.len() < 50 {
        let relaxed_distance_sq = min_distance_sq / 4.0;
        for (pid, _score) in &candidates {
            if selected.len() >= target {
                break;
            }
            // Skip already selected
            if selected.iter().any(|&(_, sp)| sp == *pid) {
                continue;
            }
            let province = match provinces.iter().find(|p| p.id == *pid) {
                Some(p) => p,
                None => continue,
            };
            let too_close = selected_sites.iter().any(|site| {
                let dx = province.site.0 - site.0;
                let dy = province.site.1 - site.1;
                dx * dx + dy * dy < relaxed_distance_sq
            });
            if too_close {
                continue;
            }
            selected.push((faction_id, *pid));
            selected_sites.push(province.site);
            faction_id += 1;
        }
    }

    selected
}

/// Compute a province budget for a faction based on its capital province.
///
/// All factions are capped at 25 provinces max. Better capitals get
/// slightly larger budgets within that cap.
fn province_budget(capital: &Province, rng: &mut ChaCha8Rng) -> usize {
    let mut score: f32 = capital.habitability;
    if capital.is_coastal {
        score += 0.1;
    }
    if capital.is_river_junction {
        score += 0.1;
    }

    let (lo, hi) = if score > 0.75 {
        (15, 25)
    } else if score > 0.55 {
        (10, 18)
    } else {
        (5, 12)
    };

    rng.gen_range(lo..=hi)
}

/// Expand factions outward from capitals through neighboring provinces.
///
/// Uses priority-queue flood fill with terrain-cost-based expansion.
/// Each faction has a province budget based on its capital quality, creating
/// a hierarchy from empires (25-35 provinces) to city-states (4-8 provinces).
/// Provinces with habitability < 0.1 are never claimed (stay Uninhabited).
/// After all factions expand, remaining provinces with habitability >= 0.1 become Unclaimed.
pub fn grow_factions(
    provinces: &mut [Province],
    adjacency: &[Vec<u16>],
    capitals: &[(u32, u16)],
    seed: u32,
) {
    let num_provinces = provinces.len();
    if num_provinces == 0 || capitals.is_empty() {
        return;
    }

    let mut rng = ChaCha8Rng::seed_from_u64(seed as u64 ^ 0xFACE);

    // Map province id -> index in provinces slice for fast lookup
    let mut id_to_idx: Vec<usize> = vec![0; num_provinces + 1];
    for (i, p) in provinces.iter().enumerate() {
        if (p.id as usize) < id_to_idx.len() {
            id_to_idx[p.id as usize] = i;
        }
    }

    // Compute province budget for each faction
    let mut faction_budget: std::collections::HashMap<u32, usize> =
        std::collections::HashMap::new();
    let mut faction_claimed: std::collections::HashMap<u32, usize> =
        std::collections::HashMap::new();
    for &(faction_id, capital_pid) in capitals {
        let budget = if let Some(idx) = id_to_idx.get(capital_pid as usize).copied() {
            province_budget(&provinces[idx], &mut rng)
        } else {
            10
        };
        faction_budget.insert(faction_id, budget);
        faction_claimed.insert(faction_id, 0);
    }

    // Multi-source Dijkstra: all factions expand simultaneously from one queue.
    // Whichever faction reaches a province first (cheapest cumulative path) claims it.
    let mut claimed: Vec<bool> = vec![false; num_provinces + 1];
    let mut best_cost: Vec<f32> = vec![f32::MAX; num_provinces + 1];

    // Priority queue: (Reverse(cost), province_id, faction_id)
    let mut pq: BinaryHeap<(Reverse<OrderedFloat>, u16, u32)> = BinaryHeap::new();

    // Seed all capitals into the queue at cost 0
    for &(faction_id, capital_pid) in capitals {
        if (capital_pid as usize) < claimed.len() {
            claimed[capital_pid as usize] = true;
            best_cost[capital_pid as usize] = 0.0;
            *faction_claimed.get_mut(&faction_id).unwrap() += 1;
        }
        if let Some(idx) = id_to_idx.get(capital_pid as usize).copied() {
            if let Some(p) = provinces.get_mut(idx) {
                p.political_state = PoliticalState::Claimed { faction_id };
            }
        }

        // Push capital's neighbors into the shared queue
        if (capital_pid as usize) < adjacency.len() {
            let capital_site = provinces
                .iter()
                .find(|p| p.id == capital_pid)
                .map(|p| p.site)
                .unwrap_or((0.0, 0.0));

            for &neighbor_id in &adjacency[capital_pid as usize] {
                if let Some(idx) = id_to_idx.get(neighbor_id as usize).copied() {
                    let neighbor = &provinces[idx];
                    if neighbor.habitability < 0.1 {
                        continue;
                    }
                    let dx = neighbor.site.0 - capital_site.0;
                    let dy = neighbor.site.1 - capital_site.1;
                    let hop_dist = (dx * dx + dy * dy).sqrt() as f32;
                    let cost = neighbor.terrain_cost * 5.0 + hop_dist * 0.05;
                    pq.push((Reverse(OrderedFloat(cost)), neighbor_id, faction_id));
                }
            }
        }
    }

    // Expand all factions simultaneously
    while let Some((Reverse(OrderedFloat(cost)), pid, faction_id)) = pq.pop() {
        if (pid as usize) >= claimed.len() {
            continue;
        }

        // Already claimed by any faction — skip
        if claimed[pid as usize] {
            continue;
        }

        // Faction hit its province budget — stop expanding
        let budget = faction_budget.get(&faction_id).copied().unwrap_or(0);
        let count = faction_claimed.get(&faction_id).copied().unwrap_or(0);
        if count >= budget {
            continue;
        }

        // Skip if a cheaper path was already found
        if cost >= best_cost[pid as usize] {
            continue;
        }
        best_cost[pid as usize] = cost;

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
        *faction_claimed.get_mut(&faction_id).unwrap() += 1;
        if let Some(idx) = id_to_idx.get(pid as usize).copied() {
            if let Some(p) = provinces.get_mut(idx) {
                p.political_state = PoliticalState::Claimed { faction_id };
            }
        }

        // Check if faction is now full
        if faction_claimed[&faction_id] >= budget {
            continue; // Don't push more neighbors
        }

        // Get current province site for hop distance calculation
        let current_site = id_to_idx
            .get(pid as usize)
            .and_then(|&idx| provinces.get(idx))
            .map(|p| p.site)
            .unwrap_or((0.0, 0.0));

        // Push unclaimed neighbors
        if (pid as usize) < adjacency.len() {
            for &neighbor_id in &adjacency[pid as usize] {
                if (neighbor_id as usize) < claimed.len() && !claimed[neighbor_id as usize] {
                    if let Some(idx) = id_to_idx.get(neighbor_id as usize).copied() {
                        let neighbor = &provinces[idx];
                        if neighbor.habitability < 0.1 {
                            continue;
                        }
                        let dx = neighbor.site.0 - current_site.0;
                        let dy = neighbor.site.1 - current_site.1;
                        let hop_dist = (dx * dx + dy * dy).sqrt() as f32;
                        let hop_cost = neighbor.terrain_cost * 5.0 + hop_dist * 0.05;
                        let total = cost + hop_cost;
                        if total < best_cost.get(neighbor_id as usize).copied().unwrap_or(f32::MAX) {
                            pq.push((
                                Reverse(OrderedFloat(total)),
                                neighbor_id,
                                faction_id,
                            ));
                        }
                    }
                }
            }
        }
    }

    // Absorb remaining habitable provinces into adjacent factions.
    // Prefer the smallest adjacent faction so large factions don't keep growing.
    // Repeat until no more provinces can be absorbed (handles chains of unclaimed).
    let mut faction_sizes: std::collections::HashMap<u32, usize> =
        std::collections::HashMap::new();
    for p in provinces.iter() {
        if let PoliticalState::Claimed { faction_id } = p.political_state {
            *faction_sizes.entry(faction_id).or_insert(0) += 1;
        }
    }

    loop {
        let mut changed = false;
        for pi in 0..provinces.len() {
            let p_id = provinces[pi].id;
            if !matches!(provinces[pi].political_state, PoliticalState::Uninhabited) {
                continue;
            }
            if provinces[pi].habitability < 0.1 {
                continue;
            }
            if claimed.get(p_id as usize).copied().unwrap_or(false) {
                continue;
            }

            // Find adjacent factions under 25 provinces, pick the smallest one
            let mut best_faction: Option<u32> = None;
            let mut best_size = usize::MAX;
            if (p_id as usize) < adjacency.len() {
                for &nid in &adjacency[p_id as usize] {
                    if let Some(&ni) = id_to_idx.get(nid as usize) {
                        if let PoliticalState::Claimed { faction_id } =
                            provinces[ni].political_state
                        {
                            let size = faction_sizes.get(&faction_id).copied().unwrap_or(0);
                            if size < 25 && size < best_size {
                                best_size = size;
                                best_faction = Some(faction_id);
                            }
                        }
                    }
                }
            }

            if let Some(fid) = best_faction {
                provinces[pi].political_state = PoliticalState::Claimed { faction_id: fid };
                if (p_id as usize) < claimed.len() {
                    claimed[p_id as usize] = true;
                }
                *faction_sizes.entry(fid).or_insert(0) += 1;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    // Any remaining habitable provinces that couldn't be absorbed become
    // independent single-province factions
    let mut next_faction_id = capitals.iter().map(|&(fid, _)| fid).max().unwrap_or(0) + 1;
    for p in provinces.iter_mut() {
        if matches!(p.political_state, PoliticalState::Uninhabited) {
            if p.habitability >= 0.1 && !claimed.get(p.id as usize).copied().unwrap_or(false) {
                p.political_state = PoliticalState::Claimed {
                    faction_id: next_faction_id,
                };
                next_faction_id += 1;
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
            make_province(2, 0.85, (110.0, 110.0)), // ~14 pixels apart, well under 150
            make_province(3, 0.8, (500.0, 500.0)),
        ];
        let caps = place_capitals(&provinces, 50, 42);
        // Both selected should be at least 150 apart
        for i in 0..caps.len() {
            for j in (i + 1)..caps.len() {
                let a = provinces.iter().find(|p| p.id == caps[i].1).unwrap();
                let b = provinces.iter().find(|p| p.id == caps[j].1).unwrap();
                let dx = a.site.0 - b.site.0;
                let dy = a.site.1 - b.site.1;
                assert!(dx * dx + dy * dy >= 150.0 * 150.0);
            }
        }
    }

    #[test]
    fn capitals_require_minimum_habitability() {
        let provinces = vec![
            make_province(1, 0.2, (100.0, 100.0)), // Below 0.35 threshold
            make_province(2, 0.3, (500.0, 500.0)), // Below 0.35 threshold
        ];
        let caps = place_capitals(&provinces, 50, 42);
        assert!(caps.is_empty());
    }

    #[test]
    fn auto_faction_count() {
        // 300 habitable provinces -> target = 50 + (300/80).min(30) = 53
        let mut provinces = Vec::new();
        for i in 1..=300u16 {
            let x = (i as f64) * 20.0;
            let y = (i as f64) * 10.0;
            provinces.push(make_province(i, 0.6, (x, y)));
        }
        let caps = place_capitals(&provinces, 0, 42);
        // Actual may be fewer due to distance constraint, but should have some
        assert!(!caps.is_empty());
    }

    #[test]
    fn many_habitable_produces_at_least_50_factions() {
        let mut provinces = Vec::new();
        for i in 1..=600u16 {
            // Spread provinces far apart to avoid spacing constraint
            let x = ((i - 1) % 30) as f64 * 200.0;
            let y = ((i - 1) / 30) as f64 * 200.0;
            provinces.push(make_province(i, 0.6, (x, y)));
        }
        let caps = place_capitals(&provinces, 0, 42);
        assert!(caps.len() >= 50, "expected >= 50 factions, got {}", caps.len());
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

        // Pre-computed adjacency: 1-2, 2-3, 3-4
        let adjacency: Vec<Vec<u16>> = vec![
            vec![],           // 0 (unused)
            vec![2],          // province 1 neighbors
            vec![1, 3],       // province 2 neighbors
            vec![2, 4],       // province 3 neighbors
            vec![3],          // province 4 neighbors
        ];
        let capitals = vec![(1u32, 1u16)];

        grow_factions(
            &mut provinces,
            &adjacency,
            &capitals,
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
        // Province 4 is blocked by province 3, becomes its own independent faction
        assert!(matches!(
            provinces[3].political_state,
            PoliticalState::Claimed { .. }
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

use crate::lifegen_data::{Province, RoadSegment, SettlementSeed, SizeClass};

/// Build a simple directed acyclic graph of trade flows.
///
/// Returns `Vec<(from_settlement_id, to_settlement_id, trade_value)>`.
///
/// Trade flows from smaller settlements toward larger ones along the settlement
/// hierarchy. Each settlement finds the nearest larger settlement that is
/// approximately road-connected (any road segment endpoint within 50px of both
/// settlements). Trade value equals the source province's habitability.
pub fn build_trade_dag(
    settlements: &[SettlementSeed],
    road_segments: &[RoadSegment],
    provinces: &[Province],
) -> Vec<(u32, u32, f32)> {
    let mut flows: Vec<(u32, u32, f32)> = Vec::new();

    for source in settlements {
        let source_rank = size_rank(source.size_class);

        // Find nearest larger settlement that is road-connected
        let mut best_target: Option<&SettlementSeed> = None;
        let mut best_dist = f64::MAX;

        for candidate in settlements {
            if candidate.id == source.id {
                continue;
            }

            let candidate_rank = size_rank(candidate.size_class);
            if candidate_rank <= source_rank {
                continue; // Must be strictly larger
            }

            // Check approximate road connectivity: at least one road segment
            // has an endpoint within 50px of source AND an endpoint within 50px
            // of candidate.
            if !approximately_connected(source, candidate, road_segments) {
                continue;
            }

            let dist = euclidean_dist(source.position, candidate.position);
            if dist < best_dist {
                best_dist = dist;
                best_target = Some(candidate);
            }
        }

        if let Some(target) = best_target {
            // Trade value = source province habitability
            let trade_value = province_habitability(source.province_id, provinces);
            if trade_value > 0.0 {
                flows.push((source.id, target.id, trade_value));
            }
        }
        // If no larger settlement is nearby/connected, this settlement doesn't trade
    }

    flows
}

/// Numeric rank for settlement size. Higher = larger.
fn size_rank(sc: SizeClass) -> u8 {
    match sc {
        SizeClass::Outpost => 0,
        SizeClass::Village => 1,
        SizeClass::Town => 2,
        SizeClass::City => 3,
        SizeClass::Metropolis => 4,
    }
}

/// Check if two settlements are approximately connected by the road network.
///
/// A pair is considered connected if any single road segment has one endpoint
/// within 50px of settlement A and the other endpoint within 50px of settlement B,
/// OR if there exists a chain of segments where at least one segment is near A
/// and at least one segment is near B. For simplicity this stub only checks
/// single-segment proximity.
// TODO: richer trade model — walk the full road graph for multi-hop connectivity
fn approximately_connected(
    a: &SettlementSeed,
    b: &SettlementSeed,
    road_segments: &[RoadSegment],
) -> bool {
    const PROXIMITY: f64 = 50.0;
    const PROXIMITY_SQ: f64 = PROXIMITY * PROXIMITY;

    // Check if any road segment has endpoints near both settlements.
    // We check both orientations of the segment (from/to near A or B).
    for seg in road_segments {
        let seg_from = seg.from;
        let seg_to = seg.to;

        let from_near_a = dist_sq(seg_from, a.position) <= PROXIMITY_SQ;
        let from_near_b = dist_sq(seg_from, b.position) <= PROXIMITY_SQ;
        let to_near_a = dist_sq(seg_to, a.position) <= PROXIMITY_SQ;
        let to_near_b = dist_sq(seg_to, b.position) <= PROXIMITY_SQ;

        if (from_near_a && to_near_b) || (from_near_b && to_near_a) {
            return true;
        }
    }

    // TODO: richer trade model — multi-hop road graph traversal
    // For now, also check if any road segment endpoint is near A
    // and any (possibly different) segment endpoint is near B,
    // indicating both are on the network even if not directly linked.
    let a_on_network = road_segments.iter().any(|seg| {
        dist_sq(seg.from, a.position) <= PROXIMITY_SQ
            || dist_sq(seg.to, a.position) <= PROXIMITY_SQ
    });

    let b_on_network = road_segments.iter().any(|seg| {
        dist_sq(seg.from, b.position) <= PROXIMITY_SQ
            || dist_sq(seg.to, b.position) <= PROXIMITY_SQ
    });

    // If both settlements are on the road network, assume transitively connected.
    // This is a rough approximation that will be replaced with proper graph search.
    // TODO: richer trade model — replace with BFS/DFS on road graph adjacency
    a_on_network && b_on_network
}

/// Squared Euclidean distance between two points.
fn dist_sq(a: (f64, f64), b: (f64, f64)) -> f64 {
    let dx = a.0 - b.0;
    let dy = a.1 - b.1;
    dx * dx + dy * dy
}

/// Euclidean distance between two points.
fn euclidean_dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    dist_sq(a, b).sqrt()
}

/// Look up a province's habitability by id. Returns 0.0 if not found.
fn province_habitability(province_id: u16, provinces: &[Province]) -> f32 {
    provinces
        .iter()
        .find(|p| p.id == province_id)
        .map(|p| p.habitability)
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifegen_data::{PoliticalState, SettlementTier};
    use rb_core::TileType;

    fn make_settlement(
        id: u32,
        x: f64,
        y: f64,
        sc: SizeClass,
        province_id: u16,
    ) -> SettlementSeed {
        SettlementSeed {
            id,
            position: (x, y),
            province_id,
            size_class: sc,
            tier: match sc {
                SizeClass::Metropolis => SettlementTier::Capital,
                SizeClass::City => SettlementTier::Major,
                SizeClass::Town => SettlementTier::Minor,
                SizeClass::Village => SettlementTier::Minor,
                SizeClass::Outpost => SettlementTier::Outpost,
            },
        }
    }

    fn make_province(id: u16, habitability: f32) -> Province {
        Province {
            id,
            site: (0.0, 0.0),
            biome: TileType::Plains,
            habitability,
            area_px: 1000,
            is_coastal: false,
            is_river_junction: false,
            elevation_mean: 0.1,
            terrain_cost: 0.5,
            political_state: PoliticalState::Unclaimed,
        }
    }

    fn make_road(from: (f64, f64), to: (f64, f64)) -> RoadSegment {
        RoadSegment {
            from,
            to,
            cost: 1.0,
            road_type_id: 2,
        }
    }

    #[test]
    fn size_rank_ordering() {
        assert!(size_rank(SizeClass::Outpost) < size_rank(SizeClass::Village));
        assert!(size_rank(SizeClass::Village) < size_rank(SizeClass::Town));
        assert!(size_rank(SizeClass::Town) < size_rank(SizeClass::City));
        assert!(size_rank(SizeClass::City) < size_rank(SizeClass::Metropolis));
    }

    #[test]
    fn empty_inputs_no_trade() {
        let result = build_trade_dag(&[], &[], &[]);
        assert!(result.is_empty());
    }

    #[test]
    fn single_settlement_no_trade() {
        let settlements = vec![make_settlement(0, 100.0, 100.0, SizeClass::Town, 1)];
        let provinces = vec![make_province(1, 0.8)];
        let result = build_trade_dag(&settlements, &[], &provinces);
        assert!(result.is_empty());
    }

    #[test]
    fn trade_flows_upward() {
        let settlements = vec![
            make_settlement(0, 100.0, 100.0, SizeClass::Village, 1),
            make_settlement(1, 120.0, 100.0, SizeClass::City, 2),
        ];
        let roads = vec![make_road((100.0, 100.0), (120.0, 100.0))];
        let provinces = vec![make_province(1, 0.7), make_province(2, 0.9)];

        let flows = build_trade_dag(&settlements, &roads, &provinces);

        // Village should trade toward City
        assert!(flows.iter().any(|&(from, to, _)| from == 0 && to == 1));
        // City should NOT trade toward Village (no larger neighbor)
        assert!(!flows.iter().any(|&(from, _, _)| from == 1));
    }

    #[test]
    fn no_trade_without_road() {
        let settlements = vec![
            make_settlement(0, 100.0, 100.0, SizeClass::Village, 1),
            make_settlement(1, 5000.0, 5000.0, SizeClass::City, 2),
        ];
        let provinces = vec![make_province(1, 0.7), make_province(2, 0.9)];

        // No roads at all
        let flows = build_trade_dag(&settlements, &[], &provinces);
        assert!(flows.is_empty());
    }

    #[test]
    fn trade_value_from_province_habitability() {
        let settlements = vec![
            make_settlement(0, 100.0, 100.0, SizeClass::Village, 1),
            make_settlement(1, 110.0, 100.0, SizeClass::Metropolis, 2),
        ];
        let roads = vec![make_road((100.0, 100.0), (110.0, 100.0))];
        let provinces = vec![make_province(1, 0.42), make_province(2, 0.9)];

        let flows = build_trade_dag(&settlements, &roads, &provinces);
        assert_eq!(flows.len(), 1);

        let (from, to, value) = flows[0];
        assert_eq!(from, 0);
        assert_eq!(to, 1);
        assert!((value - 0.42).abs() < 1e-5);
    }
}

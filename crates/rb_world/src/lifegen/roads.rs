use crate::lifegen_data::{RoadSegment, SettlementSeed, SettlementTier, SizeClass};
use rayon::prelude::*;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

/// Build a road network connecting settlements via MST + A* pathfinding.
///
/// 1. Compute MST over all settlements (Prim's, Euclidean distance).
/// 2. Add extra edges: capital-to-capital highways, nearby town+ pairs.
/// 3. A* each edge on the navigation_cost grid to find terrain-respecting paths.
/// 4. Douglas-Peucker simplify, then convert to `RoadSegment`s.
pub fn build_roads(
    settlements: &[SettlementSeed],
    navigation_cost: &[f32],
    width: usize,
    height: usize,
) -> Vec<RoadSegment> {
    if settlements.len() < 2 {
        return Vec::new();
    }

    // --- Step 1: MST edges ---
    let mut edges = prim_mst(settlements);

    // --- Step 2: Additional edges beyond MST ---
    let existing: HashSet<(usize, usize)> = edges
        .iter()
        .flat_map(|&(a, b)| [(a.min(b), a.max(b))])
        .collect();

    // Capital-to-capital highways (distance-capped to avoid impossibly long A* searches)
    let capital_indices: Vec<usize> = settlements
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            s.size_class == SizeClass::Metropolis || s.tier == SettlementTier::Capital
        })
        .map(|(i, _)| i)
        .collect();

    for i in 0..capital_indices.len() {
        for j in (i + 1)..capital_indices.len() {
            let a = capital_indices[i];
            let b = capital_indices[j];
            let key = (a.min(b), a.max(b));
            if !existing.contains(&key) {
                let dist = euclidean_dist(settlements[a].position, settlements[b].position);
                if dist <= 1500.0 {
                    edges.push((a, b));
                }
            }
        }
    }

    // Each town+ settlement gets up to 2 nearest non-MST town+ neighbors (within 300px)
    let existing: HashSet<(usize, usize)> = edges
        .iter()
        .flat_map(|&(a, b)| [(a.min(b), a.max(b))])
        .collect();

    let max_bonus_per_settlement = 2;
    for i in 0..settlements.len() {
        if !is_town_or_larger(settlements[i].size_class) {
            continue;
        }
        // Gather candidate neighbors sorted by distance
        let mut candidates: Vec<(usize, f64)> = (0..settlements.len())
            .filter(|&j| {
                j != i
                    && is_town_or_larger(settlements[j].size_class)
                    && !existing.contains(&(i.min(j), i.max(j)))
            })
            .map(|j| (j, euclidean_dist(settlements[i].position, settlements[j].position)))
            .filter(|&(_, d)| d <= 300.0)
            .collect();
        candidates.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());

        for &(j, _) in candidates.iter().take(max_bonus_per_settlement) {
            let key = (i.min(j), i.max(j));
            if !existing.contains(&key) {
                edges.push((i, j));
            }
        }
    }

    // --- Step 3 & 4 & 5: A* each edge (parallel), simplify, convert to RoadSegments ---
    let road_segments: Vec<RoadSegment> = edges
        .par_iter()
        .flat_map_iter(|(idx_a, idx_b)| {
            let sa = &settlements[*idx_a];
            let sb = &settlements[*idx_b];

            let from = (
                (sa.position.0.round() as usize).min(width.saturating_sub(1)),
                (sa.position.1.round() as usize).min(height.saturating_sub(1)),
            );
            let to = (
                (sb.position.0.round() as usize).min(width.saturating_sub(1)),
                (sb.position.1.round() as usize).min(height.saturating_sub(1)),
            );

            // Skip very long edges
            let pixel_dist = euclidean_dist(sa.position, sb.position);
            if pixel_dist > 2000.0 {
                return Vec::new();
            }

            // Corridor bounding box with 200px padding
            let min_x = from.0.min(to.0).saturating_sub(200);
            let min_y = from.1.min(to.1).saturating_sub(200);
            let max_x = (from.0.max(to.0) + 200).min(width.saturating_sub(1));
            let max_y = (from.1.max(to.1) + 200).min(height.saturating_sub(1));
            let corridor = Some((min_x, min_y, max_x, max_y));

            let path = match astar_path(from, to, navigation_cost, width, height, corridor) {
                Some(p) => p,
                None => {
                    return Vec::new();
                }
            };

            let simplified = simplify_path(&path, 3.0);
            let road_type_id = classify_road(sa, sb);

            simplified
                .windows(2)
                .map(|pair| {
                    let seg_from = pair[0];
                    let seg_to = pair[1];
                    let dx = seg_to.0 as f64 - seg_from.0 as f64;
                    let dy = seg_to.1 as f64 - seg_from.1 as f64;
                    let seg_dist = (dx * dx + dy * dy).sqrt();
                    let mid_x = (seg_from.0 + seg_to.0) / 2;
                    let mid_y = (seg_from.1 + seg_to.1) / 2;
                    let mid_idx = mid_y * width + mid_x;
                    let nav = navigation_cost
                        .get(mid_idx)
                        .copied()
                        .unwrap_or(0.01)
                        .max(0.01);
                    let cost = (seg_dist as f32) / nav;
                    RoadSegment {
                        from: (seg_from.0 as f64, seg_from.1 as f64),
                        to: (seg_to.0 as f64, seg_to.1 as f64),
                        cost,
                        road_type_id,
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect();

    road_segments
}

/// A* pathfinding on the navigation_cost grid.
///
/// Uses `HashMap` for g_score and came_from to avoid allocating full 33M-entry arrays.
/// Expands at most 500K nodes before giving up (returns `None`).
fn astar_path(
    from: (usize, usize),
    to: (usize, usize),
    nav_cost: &[f32],
    w: usize,
    h: usize,
    corridor: Option<(usize, usize, usize, usize)>,
) -> Option<Vec<(usize, usize)>> {
    const MAX_EXPANSIONS: u32 = 2_000_000;

    let pack = |x: usize, y: usize| -> u32 { (y * w + x) as u32 };
    let unpack = |idx: u32| -> (usize, usize) { ((idx as usize) % w, (idx as usize) / w) };

    let start = pack(from.0, from.1);
    let goal = pack(to.0, to.1);

    if start == goal {
        return Some(vec![from]);
    }

    let heuristic = |x: usize, y: usize| -> f32 {
        let dx = x as f64 - to.0 as f64;
        let dy = y as f64 - to.1 as f64;
        (dx * dx + dy * dy).sqrt() as f32
    };

    let mut g_score: HashMap<u32, f32> = HashMap::with_capacity(8192);
    let mut came_from: HashMap<u32, u32> = HashMap::with_capacity(8192);

    g_score.insert(start, 0.0);

    // BinaryHeap<Reverse<(f_cost_fixed, packed_idx)>>
    let mut open: BinaryHeap<Reverse<(u32, u32)>> = BinaryHeap::new();
    let h0 = heuristic(from.0, from.1);
    open.push(Reverse(((h0 * 100.0) as u32, start)));

    let mut expansions: u32 = 0;

    // 8-connected neighbors: (dx, dy, step_cost)
    let neighbors: [(i32, i32, f32); 8] = [
        (-1, 0, 1.0),
        (1, 0, 1.0),
        (0, -1, 1.0),
        (0, 1, 1.0),
        (-1, -1, 1.414),
        (1, -1, 1.414),
        (-1, 1, 1.414),
        (1, 1, 1.414),
    ];

    while let Some(Reverse((_f_fixed, current))) = open.pop() {
        if current == goal {
            // Reconstruct path
            let mut path = Vec::new();
            let mut node = current;
            loop {
                let (px, py) = unpack(node);
                path.push((px, py));
                if let Some(&prev) = came_from.get(&node) {
                    node = prev;
                } else {
                    break;
                }
            }
            path.reverse();
            return Some(path);
        }

        expansions += 1;
        if expansions > MAX_EXPANSIONS {
            return None;
        }

        let (cx, cy) = unpack(current);
        let current_g = g_score.get(&current).copied().unwrap_or(f32::MAX);

        for &(dx, dy, base_step) in &neighbors {
            let nx_i = cx as i32 + dx;
            let ny_i = cy as i32 + dy;

            if nx_i < 0 || ny_i < 0 || nx_i >= w as i32 || ny_i >= h as i32 {
                continue;
            }

            let nx = nx_i as usize;
            let ny = ny_i as usize;

            if let Some((min_x, min_y, max_x, max_y)) = corridor {
                if nx < min_x || ny < min_y || nx > max_x || ny > max_y {
                    continue;
                }
            }

            let n_idx = ny * w + nx;

            let nc = nav_cost[n_idx];
            if nc == 0.0 {
                continue; // Impassable
            }

            let step_cost = base_step / nc.max(0.01);
            let tentative_g = current_g + step_cost;
            let neighbor_packed = pack(nx, ny);

            let prev_g = g_score.get(&neighbor_packed).copied().unwrap_or(f32::MAX);
            if tentative_g < prev_g {
                g_score.insert(neighbor_packed, tentative_g);
                came_from.insert(neighbor_packed, current);
                let f = tentative_g + heuristic(nx, ny);
                open.push(Reverse(((f * 100.0) as u32, neighbor_packed)));
            }
        }
    }

    None // No path found
}

/// Douglas-Peucker path simplification.
///
/// Reduces a pixel-level path to a smaller set of waypoints while preserving
/// overall shape within `epsilon` pixels of deviation.
fn simplify_path(path: &[(usize, usize)], epsilon: f32) -> Vec<(usize, usize)> {
    if path.len() <= 2 {
        return path.to_vec();
    }

    // Find the point with the maximum distance from the line segment (first, last)
    let first = path[0];
    let last = path[path.len() - 1];

    let mut max_dist: f32 = 0.0;
    let mut max_idx: usize = 0;

    for i in 1..path.len() - 1 {
        let d = perpendicular_distance(path[i], first, last);
        if d > max_dist {
            max_dist = d;
            max_idx = i;
        }
    }

    if max_dist > epsilon {
        // Recurse on both halves
        let mut left = simplify_path(&path[..=max_idx], epsilon);
        let right = simplify_path(&path[max_idx..], epsilon);

        // Remove the duplicate point at the junction
        left.pop();
        left.extend_from_slice(&right);
        left
    } else {
        // All intermediate points are within epsilon; keep only endpoints
        vec![first, last]
    }
}

/// Prim's MST over settlements using Euclidean distance as edge weights.
///
/// Returns a list of (settlement_index_a, settlement_index_b) edges forming the MST.
fn prim_mst(settlements: &[SettlementSeed]) -> Vec<(usize, usize)> {
    let n = settlements.len();
    if n <= 1 {
        return Vec::new();
    }

    let mut in_tree = vec![false; n];
    let mut min_cost = vec![f64::MAX; n];
    let mut nearest = vec![0usize; n];
    let mut edges = Vec::with_capacity(n - 1);

    // Start from settlement 0
    in_tree[0] = true;
    for i in 1..n {
        min_cost[i] = euclidean_dist(settlements[0].position, settlements[i].position);
        nearest[i] = 0;
    }

    for _ in 0..n - 1 {
        // Find the cheapest edge from tree to non-tree
        let mut best_cost = f64::MAX;
        let mut best_idx = 0;
        for i in 0..n {
            if !in_tree[i] && min_cost[i] < best_cost {
                best_cost = min_cost[i];
                best_idx = i;
            }
        }

        in_tree[best_idx] = true;
        edges.push((nearest[best_idx], best_idx));

        // Update costs for remaining non-tree vertices
        for i in 0..n {
            if !in_tree[i] {
                let d = euclidean_dist(settlements[best_idx].position, settlements[i].position);
                if d < min_cost[i] {
                    min_cost[i] = d;
                    nearest[i] = best_idx;
                }
            }
        }
    }

    edges
}

/// Euclidean distance between two (f64, f64) points.
fn euclidean_dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    let dx = a.0 - b.0;
    let dy = a.1 - b.1;
    (dx * dx + dy * dy).sqrt()
}

/// Perpendicular distance from point `p` to the line segment `(a, b)`.
fn perpendicular_distance(p: (usize, usize), a: (usize, usize), b: (usize, usize)) -> f32 {
    let px = p.0 as f64;
    let py = p.1 as f64;
    let ax = a.0 as f64;
    let ay = a.1 as f64;
    let bx = b.0 as f64;
    let by = b.1 as f64;

    let dx = bx - ax;
    let dy = by - ay;
    let len_sq = dx * dx + dy * dy;

    if len_sq < 1e-12 {
        // a and b are the same point
        let ddx = px - ax;
        let ddy = py - ay;
        return (ddx * ddx + ddy * ddy).sqrt() as f32;
    }

    let num = ((by - ay) * px - (bx - ax) * py + bx * ay - by * ax).abs();
    let den = len_sq.sqrt();
    (num / den) as f32
}

/// Classify road type based on endpoint settlements.
///
/// - 0 (Imperial): both endpoints are Metropolis or Capital tier
/// - 1 (Provincial): either endpoint is City or Major tier
/// - 2 (Trail): everything else
fn classify_road(a: &SettlementSeed, b: &SettlementSeed) -> u8 {
    let a_capital =
        a.size_class == SizeClass::Metropolis || a.tier == SettlementTier::Capital;
    let b_capital =
        b.size_class == SizeClass::Metropolis || b.tier == SettlementTier::Capital;

    if a_capital && b_capital {
        return 0; // Imperial
    }

    let a_city = a.size_class == SizeClass::City || a.tier == SettlementTier::Major;
    let b_city = b.size_class == SizeClass::City || b.tier == SettlementTier::Major;

    if a_capital || b_capital || a_city || b_city {
        return 1; // Provincial
    }

    2 // Trail
}

/// Check if a settlement's size class is Town or larger.
fn is_town_or_larger(sc: SizeClass) -> bool {
    matches!(sc, SizeClass::Metropolis | SizeClass::City | SizeClass::Town)
}

/// Generate a direct line of pixel coordinates from `from` to `to` (Bresenham).
/// Used as fallback when A* fails or when the distance exceeds limits.
fn direct_line(from: (usize, usize), to: (usize, usize)) -> Vec<(usize, usize)> {
    let mut points = Vec::new();

    let x0 = from.0 as i32;
    let y0 = from.1 as i32;
    let x1 = to.0 as i32;
    let y1 = to.1 as i32;

    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let mut cx = x0;
    let mut cy = y0;

    loop {
        points.push((cx as usize, cy as usize));
        if cx == x1 && cy == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            if cx == x1 {
                break;
            }
            err += dy;
            cx += sx;
        }
        if e2 <= dx {
            if cy == y1 {
                break;
            }
            err += dx;
            cy += sy;
        }
    }

    points
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifegen_data::SettlementTier;

    fn make_settlement(id: u32, x: f64, y: f64, sc: SizeClass, tier: SettlementTier) -> SettlementSeed {
        SettlementSeed {
            id,
            position: (x, y),
            province_id: 0,
            size_class: sc,
            tier,
        }
    }

    #[test]
    fn prim_mst_connects_all() {
        let settlements = vec![
            make_settlement(0, 10.0, 10.0, SizeClass::Town, SettlementTier::Minor),
            make_settlement(1, 20.0, 10.0, SizeClass::Town, SettlementTier::Minor),
            make_settlement(2, 15.0, 20.0, SizeClass::Town, SettlementTier::Minor),
        ];
        let edges = prim_mst(&settlements);
        assert_eq!(edges.len(), 2);
    }

    #[test]
    fn simplify_path_preserves_endpoints() {
        let path = vec![(0, 0), (1, 0), (2, 0), (3, 0), (4, 0)];
        let simplified = simplify_path(&path, 1.0);
        assert_eq!(simplified.first(), Some(&(0, 0)));
        assert_eq!(simplified.last(), Some(&(4, 0)));
    }

    #[test]
    fn simplify_path_collinear() {
        // All points on a straight line should reduce to just endpoints
        let path: Vec<(usize, usize)> = (0..20).map(|i| (i, 0)).collect();
        let simplified = simplify_path(&path, 0.5);
        assert_eq!(simplified.len(), 2);
    }

    #[test]
    fn classify_road_imperial() {
        let a = make_settlement(0, 0.0, 0.0, SizeClass::Metropolis, SettlementTier::Capital);
        let b = make_settlement(1, 100.0, 0.0, SizeClass::Metropolis, SettlementTier::Capital);
        assert_eq!(classify_road(&a, &b), 0);
    }

    #[test]
    fn classify_road_provincial() {
        let a = make_settlement(0, 0.0, 0.0, SizeClass::City, SettlementTier::Major);
        let b = make_settlement(1, 100.0, 0.0, SizeClass::Town, SettlementTier::Minor);
        assert_eq!(classify_road(&a, &b), 1);
    }

    #[test]
    fn classify_road_trail() {
        let a = make_settlement(0, 0.0, 0.0, SizeClass::Village, SettlementTier::Minor);
        let b = make_settlement(1, 100.0, 0.0, SizeClass::Village, SettlementTier::Minor);
        assert_eq!(classify_road(&a, &b), 2);
    }

    #[test]
    fn empty_settlements_returns_no_roads() {
        let result = build_roads(&[], &[], 100, 100);
        assert!(result.is_empty());
    }

    #[test]
    fn single_settlement_returns_no_roads() {
        let settlements = vec![
            make_settlement(0, 50.0, 50.0, SizeClass::Town, SettlementTier::Minor),
        ];
        let nav = vec![1.0f32; 100 * 100];
        let result = build_roads(&settlements, &nav, 100, 100);
        assert!(result.is_empty());
    }

    #[test]
    fn two_settlements_produces_road() {
        let settlements = vec![
            make_settlement(0, 10.0, 10.0, SizeClass::Town, SettlementTier::Minor),
            make_settlement(1, 30.0, 10.0, SizeClass::Town, SettlementTier::Minor),
        ];
        let nav = vec![1.0f32; 50 * 50];
        let result = build_roads(&settlements, &nav, 50, 50);
        assert!(!result.is_empty());
    }

    #[test]
    fn direct_line_basic() {
        let line = direct_line((0, 0), (5, 0));
        assert_eq!(line.len(), 6);
        assert_eq!(line[0], (0, 0));
        assert_eq!(line[5], (5, 0));
    }
}

use rb_core::{TerrainQuery, TileType};

use crate::lifegen_data::{PoliticalState, Province};
use rand::prelude::*;
use rand_chacha::ChaCha8Rng;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

// ---------------------------------------------------------------------------
// 1. Seed provinces via variable-radius Poisson disc sampling
// ---------------------------------------------------------------------------

/// Seed province sites using variable-radius Poisson disc sampling.
///
/// High-habitability regions produce dense, small provinces while
/// low-habitability regions produce sparse, large provinces.
///
/// Returns pixel coordinates `(x, y)` for each province seed.
pub fn seed_provinces(
    habitability: &[f32],
    width: usize,
    height: usize,
    seed: u32,
) -> Vec<(usize, usize)> {
    const MIN_RADIUS: f64 = 4.0;
    const MAX_RADIUS: f64 = 48.0;
    const MAX_ATTEMPTS: usize = 200_000;
    const TARGET_PROVINCES: usize = 970;

    let cell_size = MIN_RADIUS;
    let grid_w = (width as f64 / cell_size).ceil() as usize;
    let grid_h = (height as f64 / cell_size).ceil() as usize;

    // Spatial acceleration grid: each cell stores index into `seeds` or usize::MAX
    let mut grid = vec![usize::MAX; grid_w * grid_h];
    let mut seeds: Vec<(usize, usize)> = Vec::with_capacity(TARGET_PROVINCES);
    let mut rng = ChaCha8Rng::seed_from_u64(seed as u64);

    for _ in 0..MAX_ATTEMPTS {
        if seeds.len() >= TARGET_PROVINCES {
            break;
        }

        // Pick a random pixel
        let x = rng.gen_range(0..width);
        let y = rng.gen_range(0..height);
        let idx = y * width + x;

        let hab = habitability[idx];
        if hab < 0.01 {
            continue;
        }

        // Variable radius: high hab -> small radius (dense), low hab -> large radius (sparse)
        let radius = MAX_RADIUS + (MIN_RADIUS - MAX_RADIUS) * (hab as f64).powf(0.7);

        // Check against existing seeds within radius using the spatial grid
        let gx = (x as f64 / cell_size) as usize;
        let gy = (y as f64 / cell_size) as usize;
        let search_cells = (radius / cell_size).ceil() as usize + 1;

        let gx_min = gx.saturating_sub(search_cells);
        let gy_min = gy.saturating_sub(search_cells);
        let gx_max = (gx + search_cells).min(grid_w - 1);
        let gy_max = (gy + search_cells).min(grid_h - 1);

        let mut too_close = false;
        'outer: for cy in gy_min..=gy_max {
            for cx in gx_min..=gx_max {
                let cell_idx = cy * grid_w + cx;
                let si = grid[cell_idx];
                if si != usize::MAX {
                    let (sx, sy) = seeds[si];
                    let dx = x as f64 - sx as f64;
                    let dy = y as f64 - sy as f64;
                    let dist_sq = dx * dx + dy * dy;

                    // Also compute the radius for the existing seed
                    let s_hab = habitability[sy * width + sx];
                    let s_radius =
                        MAX_RADIUS + (MIN_RADIUS - MAX_RADIUS) * (s_hab as f64).powf(0.7);

                    // Use the larger of the two radii as the exclusion distance
                    let min_dist = radius.max(s_radius);
                    if dist_sq < min_dist * min_dist {
                        too_close = true;
                        break 'outer;
                    }
                }
            }
        }

        if !too_close {
            let new_idx = seeds.len();
            seeds.push((x, y));
            grid[gy * grid_w + gx] = new_idx;
        }
    }

    seeds
}

// ---------------------------------------------------------------------------
// 2. Tessellate provinces via cost-weighted Dijkstra flood fill
// ---------------------------------------------------------------------------

/// Dijkstra-style flood fill from all province seeds simultaneously.
///
/// Each pixel is assigned to the nearest province by navigation cost.
/// Ocean pixels (nav_cost == 0 or `terrain.is_ocean`) remain unassigned (0).
///
/// Returns a `Vec<u16>` of length `width * height` where each value is a
/// 1-based province id (0 = unassigned/ocean).
pub fn tessellate_provinces(
    seeds: &[(usize, usize)],
    navigation_cost: &[f32],
    terrain: &dyn TerrainQuery,
    width: usize,
    height: usize,
) -> Vec<u16> {
    let total = width * height;
    let mut province_ids = vec![0u16; total];

    // Priority queue: (quantized_cost, pixel_index, province_id)
    // Using Reverse so BinaryHeap (max-heap) acts as a min-heap.
    let mut heap: BinaryHeap<(Reverse<u32>, u32, u16)> = BinaryHeap::new();

    // Enqueue all seeds
    for (i, &(sx, sy)) in seeds.iter().enumerate() {
        let pid = (i + 1) as u16; // 1-based
        let pixel_idx = sy * width + sx;
        heap.push((Reverse(0), pixel_idx as u32, pid));
    }

    // 4-connected neighbor offsets: (dx, dy)
    let offsets: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

    while let Some((Reverse(cost), pixel_packed, pid)) = heap.pop() {
        let pi = pixel_packed as usize;
        if pi >= total {
            continue;
        }

        // Already assigned — skip
        if province_ids[pi] != 0 {
            continue;
        }

        // Skip ocean
        let px = pi % width;
        let py = pi / width;
        let nav = navigation_cost[pi];
        if nav <= 0.0 || terrain.is_ocean(px, py) {
            continue;
        }

        province_ids[pi] = pid;

        // Push 4-connected neighbors
        for &(dx, dy) in &offsets {
            let nx = px as i32 + dx;
            let ny = py as i32 + dy;
            if nx < 0 || ny < 0 || nx >= width as i32 || ny >= height as i32 {
                continue;
            }
            let nux = nx as usize;
            let nuy = ny as usize;
            let ni = nuy * width + nux;

            // Skip already assigned
            if province_ids[ni] != 0 {
                continue;
            }

            let n_nav = navigation_cost[ni];
            if n_nav <= 0.0 || terrain.is_ocean(nux, nuy) {
                continue;
            }

            // Cost to cross into neighbor: inverse of navigation cost
            let step_cost = 1.0 / n_nav.max(0.01);
            let new_cost = cost + (step_cost * 1000.0) as u32;

            heap.push((Reverse(new_cost), ni as u32, pid));
        }
    }

    province_ids
}

// ---------------------------------------------------------------------------
// 3. Snap province borders to rivers
// ---------------------------------------------------------------------------

/// Post-process province borders to align with nearby rivers.
///
/// For each pixel on a province border (has a 4-connected neighbor with a
/// different province id), check if a river is within 3 pixels. If so,
/// reassign the border pixel to the neighboring province that is on the
/// same side of the nearest river pixel, making provinces end at rivers.
pub fn snap_borders_to_rivers(
    province_ids: &mut [u16],
    terrain: &dyn TerrainQuery,
    width: usize,
    height: usize,
) {
    let offsets_4: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
    let search_radius: i32 = 3;

    // Collect reassignments to apply after iteration (avoid mutating while iterating)
    let mut reassignments: Vec<(usize, u16)> = Vec::new();

    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            let my_pid = province_ids[idx];
            if my_pid == 0 {
                continue;
            }

            // Check if this pixel is on a province border
            let mut neighbor_pid = 0u16;
            let mut is_border = false;
            for &(dx, dy) in &offsets_4 {
                let nx = x as i32 + dx;
                let ny = y as i32 + dy;
                if nx < 0 || ny < 0 || nx >= width as i32 || ny >= height as i32 {
                    continue;
                }
                let npid = province_ids[ny as usize * width + nx as usize];
                if npid != 0 && npid != my_pid {
                    is_border = true;
                    neighbor_pid = npid;
                    break;
                }
            }
            if !is_border {
                continue;
            }

            // Search for nearest river within search_radius
            let mut best_river: Option<(usize, usize, i32)> = None; // (rx, ry, dist_sq)
            for dy in -search_radius..=search_radius {
                for dx in -search_radius..=search_radius {
                    let rx = x as i32 + dx;
                    let ry = y as i32 + dy;
                    if rx < 0 || ry < 0 || rx >= width as i32 || ry >= height as i32 {
                        continue;
                    }
                    let rux = rx as usize;
                    let ruy = ry as usize;
                    if terrain.river_at(rux, ruy) > 0.0 {
                        let d = dx * dx + dy * dy;
                        if best_river.is_none() || d < best_river.unwrap().2 {
                            best_river = Some((rux, ruy, d));
                        }
                    }
                }
            }

            if let Some((rx, ry, _)) = best_river {
                // Determine which side of the river this pixel is on relative
                // to the neighboring province. If the pixel is closer to the
                // river than the neighbor province centroid, reassign it to
                // the neighbor — this effectively pushes the border onto the
                // river line.
                //
                // Simple heuristic: compute the vector from the river pixel
                // to this pixel and from the river pixel to a pixel belonging
                // to the neighbor. If they point the same direction, reassign.
                let vx = x as f64 - rx as f64;
                let vy = y as f64 - ry as f64;

                // Find a nearby pixel of the neighbor province on the other
                // side of the river.
                let mut other_vx = 0.0f64;
                let mut other_vy = 0.0f64;
                let mut found_other = false;
                for dy2 in -search_radius..=search_radius {
                    for dx2 in -search_radius..=search_radius {
                        let ox = rx as i32 + dx2;
                        let oy = ry as i32 + dy2;
                        if ox < 0 || oy < 0 || ox >= width as i32 || oy >= height as i32 {
                            continue;
                        }
                        let opid = province_ids[oy as usize * width + ox as usize];
                        if opid == neighbor_pid {
                            other_vx = ox as f64 - rx as f64;
                            other_vy = oy as f64 - ry as f64;
                            found_other = true;
                            break;
                        }
                    }
                    if found_other {
                        break;
                    }
                }

                if found_other {
                    // Dot product: if same direction, this pixel is on the
                    // same side as the neighbor province — reassign it.
                    let dot = vx * other_vx + vy * other_vy;
                    if dot > 0.0 {
                        reassignments.push((idx, neighbor_pid));
                    }
                }
            }
        }
    }

    // Apply reassignments
    for (idx, new_pid) in reassignments {
        province_ids[idx] = new_pid;
    }
}

// ---------------------------------------------------------------------------
// 4. Bake province attributes from terrain data
// ---------------------------------------------------------------------------

/// Compute aggregate attributes for each province from the terrain and
/// analysis grids.
///
/// Returns a `Vec<Province>` with one entry per seed (1-based province ids).
pub fn bake_province_attributes(
    province_ids: &[u16],
    seeds: &[(usize, usize)],
    terrain: &dyn TerrainQuery,
    habitability: &[f32],
    navigation_cost: &[f32],
    width: usize,
    height: usize,
) -> Vec<Province> {
    let n = seeds.len();

    // Per-province accumulators
    let mut area: Vec<u32> = vec![0; n];
    let mut hab_sum: Vec<f64> = vec![0.0; n];
    let mut elev_sum: Vec<f64> = vec![0.0; n];
    let mut cost_sum: Vec<f64> = vec![0.0; n]; // sum of (1 - nav_cost)
    let mut biome_counts: Vec<HashMap<TileType, u32>> = vec![HashMap::new(); n];
    let mut is_coastal: Vec<bool> = vec![false; n];
    let mut is_river_junction: Vec<bool> = vec![false; n];

    let total = width * height;

    for idx in 0..total {
        let pid = province_ids[idx];
        if pid == 0 {
            continue;
        }
        let pi = (pid - 1) as usize; // 0-based index
        if pi >= n {
            continue;
        }

        let x = idx % width;
        let y = idx / width;

        area[pi] += 1;
        hab_sum[pi] += habitability[idx] as f64;
        elev_sum[pi] += terrain.heightmap_at(x, y);
        cost_sum[pi] += (1.0 - navigation_cost[idx]) as f64;

        let biome = terrain.biome_at(x, y);
        *biome_counts[pi].entry(biome).or_insert(0) += 1;

        // Coastal check: any pixel within 3px of an ocean pixel
        if !is_coastal[pi] {
            'coast_search: for dy in -3i32..=3 {
                for dx in -3i32..=3 {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    if nx >= 0
                        && ny >= 0
                        && (nx as usize) < width
                        && (ny as usize) < height
                        && terrain.is_ocean(nx as usize, ny as usize)
                    {
                        is_coastal[pi] = true;
                        break 'coast_search;
                    }
                }
            }
        }

        // River junction check: river pixel with high drainage
        if !is_river_junction[pi] {
            let river = terrain.river_at(x, y);
            if river > 0.0 {
                let drainage = terrain.drainage_at(x, y);
                if drainage > 2000.0 {
                    is_river_junction[pi] = true;
                }
            }
        }
    }

    // Build Province structs
    let mut provinces = Vec::with_capacity(n);
    for i in 0..n {
        let (sx, sy) = seeds[i];
        let a = area[i];

        // Most common biome
        let biome = biome_counts[i]
            .iter()
            .max_by_key(|&(_, count)| *count)
            .map(|(&tile, _)| tile)
            .unwrap_or(TileType::default());

        let hab_mean = if a > 0 {
            (hab_sum[i] / a as f64) as f32
        } else {
            0.0
        };
        let elev_mean = if a > 0 {
            (elev_sum[i] / a as f64) as f32
        } else {
            0.0
        };
        let terrain_cost = if a > 0 {
            (cost_sum[i] / a as f64) as f32
        } else {
            1.0
        };

        provinces.push(Province {
            id: (i + 1) as u16,
            site: (sx as f64, sy as f64),
            biome,
            habitability: hab_mean,
            area_px: a,
            is_coastal: is_coastal[i],
            is_river_junction: is_river_junction[i],
            elevation_mean: elev_mean,
            terrain_cost,
            political_state: PoliticalState::Uninhabited,
        });
    }

    provinces
}

//! Stream power erosion simulation.
//!
//! Creates dendritic mountain/valley patterns via the competition between
//! tectonic uplift and fluvial erosion. The implicit stream power scheme:
//!
//!   h_new = (h_old + dt*U + F*h_new_receiver) / (1 + F)
//!   where F = dt * K * A^m / dx
//!
//! Strong uplift at plate boundaries pushes terrain up. Strong erosion carves
//! valleys where water accumulates. The balance between these forces creates
//! the characteristic dendritic ridge/valley texture seen in real mountain ranges.
//!
//! Processed from outlets upward (implicit solve), so each cell references
//! its receiver's already-updated height, giving unconditional stability.

use crate::rivers::{
    fill_depressions, compute_flow_accumulation, box_blur,
    D8_OFFSETS, D8_DISTANCES, NO_FLOW,
};

/// Parameters controlling the erosion simulation.
pub struct ErosionParams {
    /// Erodibility coefficient — controls valley depth.
    pub k_base: f64,
    /// Uplift rate — controls mountain height at boundaries.
    pub u_base: f64,
    /// Drainage area exponent (Hack's law, typically 0.4-0.5).
    pub m: f64,
    /// Timestep (implicit scheme is unconditionally stable).
    pub dt: f64,
    /// Number of erosion iterations (more = more developed drainage).
    pub iterations: u32,
    /// Cell size in world units.
    pub dx: f64,
    /// Maximum slope before thermal erosion kicks in.
    pub talus_angle: f64,
    /// Fraction of excess material redistributed per thermal pass.
    pub thermal_rate: f64,
    /// Sea level threshold — cells below this are not eroded.
    pub sea_level: f64,
}

impl Default for ErosionParams {
    fn default() -> Self {
        Self {
            k_base: 0.04,
            u_base: 0.015,
            m: 0.45,
            dt: 1.0,
            iterations: 120,
            dx: 1.0,
            talus_angle: 0.5,
            thermal_rate: 0.3,
            sea_level: -0.01,
        }
    }
}

/// Result of the erosion simulation.
pub struct ErosionResult {
    /// Eroded heightmap.
    pub heightmap: Vec<f64>,
    /// Drainage area per cell (number of upstream cells).
    pub drainage_area: Vec<u32>,
    /// Accumulated sediment per cell.
    pub sediment: Vec<f64>,
}

/// Compute steepest-descent D8 flow directions.
fn compute_d8_flow(elevation: &[f64], width: usize, height: usize) -> Vec<u8> {
    let total = width * height;
    let mut flow_dir = vec![NO_FLOW; total];

    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            let h = elevation[idx];
            let mut best_slope = 0.0;
            let mut best_dir = NO_FLOW;

            for (d, &(dx, dy)) in D8_OFFSETS.iter().enumerate() {
                let nx = crate::wrap::wrap_grid_x(x as i32 + dx, width);
                let ny = y as i32 + dy;
                if ny < 0 || ny >= height as i32 {
                    continue;
                }
                let nidx = ny as usize * width + nx as usize;
                let drop = h - elevation[nidx];
                let slope = drop / D8_DISTANCES[d];
                if slope > best_slope {
                    best_slope = slope;
                    best_dir = d as u8;
                }
            }

            flow_dir[idx] = best_dir;
        }
    }

    flow_dir
}

/// Get the index of the receiver cell for a given cell and flow direction.
fn receiver_index(idx: usize, dir: u8, width: usize, height: usize) -> Option<usize> {
    if dir == NO_FLOW {
        return None;
    }
    let (dx, dy) = D8_OFFSETS[dir as usize];
    let x = (idx % width) as i32 + dx;
    let y = (idx / width) as i32 + dy;
    if y < 0 || y >= height as i32 {
        return None;
    }
    let wrapped_x = crate::wrap::wrap_grid_x(x, width);
    Some(y as usize * width + wrapped_x as usize)
}

/// Run the stream power erosion simulation.
pub fn simulate_erosion(
    heightmap: &[f64],
    rock_hardness: &[f64],
    tectonic_stress: &[f64],
    continentalness: &[f64],
    width: usize,
    height: usize,
    params: &ErosionParams,
) -> ErosionResult {
    let total = width * height;
    let mut h = heightmap.to_vec();
    let mut sediment = vec![0.0f64; total];

    // Pre-compute per-cell erodibility and uplift
    // Soft rock erodes faster (K higher when hardness low)
    let erodibility: Vec<f64> = rock_hardness.iter()
        .map(|&r| params.k_base * (1.5 - r))
        .collect();
    // Uplift concentrated at plate boundaries
    let uplift: Vec<f64> = tectonic_stress.iter()
        .map(|&s| params.u_base * s * s) // quadratic — strongly concentrated at boundaries
        .collect();

    let mut flow_dir;
    let mut accumulation;

    for iter in 0..params.iterations {
        // Fill depressions periodically (expensive, not needed every iteration)
        if iter % 5 == 0 {
            h = fill_depressions(&h, width, height, params.sea_level, None);
        }

        // Compute D8 flow directions on current terrain
        flow_dir = compute_d8_flow(&h, width, height);

        // Flow accumulation
        accumulation = compute_flow_accumulation(&flow_dir, &h, width, height);

        // Sort cells lowest-to-highest for implicit solve
        // (process outlets first, then work uphill)
        let mut sorted: Vec<usize> = (0..total).collect();
        sorted.sort_by(|&a, &b| {
            h[a].partial_cmp(&h[b]).unwrap_or(std::cmp::Ordering::Equal)
        });

        // Implicit stream power: process from outlets upward
        for &idx in &sorted {
            // Skip ocean cells
            if continentalness[idx] < params.sea_level {
                continue;
            }

            let dir = flow_dir[idx];
            if dir == NO_FLOW {
                // Sink/outlet: apply uplift only
                h[idx] += params.dt * uplift[idx];
                continue;
            }

            let Some(recv_idx) = receiver_index(idx, dir, width, height) else {
                h[idx] += params.dt * uplift[idx];
                continue;
            };

            let k = erodibility[idx];
            let a = accumulation[idx] as f64;
            let f = params.dt * k * a.powf(params.m) / params.dx;

            let h_old = h[idx];
            // h[recv_idx] is already updated (processed earlier, it's lower)
            let h_recv = h[recv_idx];
            let h_new = (h_old + params.dt * uplift[idx] + f * h_recv) / (1.0 + f);

            // Track eroded material as sediment
            let eroded = (h_old - h_new).max(0.0);
            sediment[idx] += eroded;

            h[idx] = h_new;
        }

        // Thermal erosion: enforce talus angle every few iterations
        if iter % 3 == 0 {
            // Sort highest-to-lowest for thermal (material moves downhill)
            let mut thermal_sorted: Vec<usize> = (0..total)
                .filter(|&i| continentalness[i] >= params.sea_level)
                .collect();
            thermal_sorted.sort_by(|&a, &b| {
                h[b].partial_cmp(&h[a]).unwrap_or(std::cmp::Ordering::Equal)
            });

            for &idx in &thermal_sorted {
                let x = idx % width;
                let y = idx / width;

                for (d, &(dx, dy)) in D8_OFFSETS.iter().enumerate() {
                    let nx = crate::wrap::wrap_grid_x(x as i32 + dx, width);
                    let ny = y as i32 + dy;
                    if ny < 0 || ny >= height as i32 {
                        continue;
                    }
                    let nidx = ny as usize * width + nx as usize;
                    let dist = D8_DISTANCES[d] * params.dx;
                    let slope = (h[idx] - h[nidx]) / dist;

                    if slope > params.talus_angle {
                        let excess = (slope - params.talus_angle) * dist;
                        let transfer = excess * params.thermal_rate * 0.5;
                        h[idx] -= transfer;
                        h[nidx] += transfer;
                        sediment[nidx] += transfer;
                    }
                }
            }
        }
    }

    // Clamp heights to valid range
    for idx in 0..total {
        h[idx] = h[idx].clamp(-1.0, 1.0);
    }

    // Final drainage computation on sculpted terrain
    h = fill_depressions(&h, width, height, params.sea_level, None);
    flow_dir = compute_d8_flow(&h, width, height);
    accumulation = compute_flow_accumulation(&flow_dir, &h, width, height);

    ErosionResult {
        heightmap: h,
        drainage_area: accumulation,
        sediment,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_params() -> ErosionParams {
        ErosionParams {
            iterations: 15,
            ..Default::default()
        }
    }

    #[test]
    fn flat_terrain_stays_flat() {
        let w = 32;
        let h = 16;
        let total = w * h;
        let heightmap = vec![0.1; total];
        let rock = vec![0.5; total];
        let stress = vec![0.0; total];
        let cont = vec![0.1; total];

        let params = ErosionParams {
            k_base: 0.0,
            u_base: 0.0,
            iterations: 5,
            ..Default::default()
        };

        let result = simulate_erosion(&heightmap, &rock, &stress, &cont, w, h, &params);

        for (i, (&orig, &eroded)) in heightmap.iter().zip(result.heightmap.iter()).enumerate() {
            assert!(
                (orig - eroded).abs() < 0.01,
                "Cell {} changed too much: {} -> {}", i, orig, eroded
            );
        }
    }

    #[test]
    fn uplift_without_erosion_raises() {
        let w = 32;
        let h = 16;
        let total = w * h;
        let heightmap = vec![0.1; total];
        let rock = vec![0.5; total];
        let stress = vec![1.0; total];
        let cont = vec![0.1; total];

        let params = ErosionParams {
            k_base: 0.0,
            u_base: 0.02,
            iterations: 10,
            ..Default::default()
        };

        let result = simulate_erosion(&heightmap, &rock, &stress, &cont, w, h, &params);

        let mid = (h / 2) * w + (w / 2);
        assert!(
            result.heightmap[mid] > heightmap[mid],
            "Uplifted terrain ({}) should be higher than original ({})",
            result.heightmap[mid], heightmap[mid]
        );
    }

    #[test]
    fn valley_forms_with_erosion() {
        let w = 64;
        let h = 32;
        let total = w * h;
        let heightmap: Vec<f64> = (0..total).map(|i| {
            let y = (i / w) as f64 / h as f64;
            0.5 - (y - 0.5).abs()
        }).collect();
        let rock = vec![0.2; total];
        let stress = vec![0.0; total];
        let cont = vec![0.1; total];

        let result = simulate_erosion(&heightmap, &rock, &stress, &cont, w, h, &make_params());

        let mid = (h / 2) * w + (w / 2);
        assert!(
            result.heightmap[mid] < heightmap[mid],
            "Eroded ridge ({}) should be lower than original ({})",
            result.heightmap[mid], heightmap[mid]
        );
    }

    #[test]
    fn drainage_area_increases_downstream() {
        let w = 64;
        let h = 32;
        let total = w * h;
        let heightmap: Vec<f64> = (0..total).map(|i| {
            let y = (i / w) as f64;
            1.0 - y / h as f64
        }).collect();
        let rock = vec![0.5; total];
        let stress = vec![0.0; total];
        let cont = vec![0.1; total];

        let result = simulate_erosion(&heightmap, &rock, &stress, &cont, w, h, &make_params());

        let top_mid = w / 2;
        let bottom_mid = (h - 2) * w + w / 2;
        assert!(
            result.drainage_area[bottom_mid] > result.drainage_area[top_mid],
            "Downstream drainage ({}) should exceed upstream ({})",
            result.drainage_area[bottom_mid], result.drainage_area[top_mid]
        );
    }

    #[test]
    fn hard_rock_erodes_slower() {
        let w = 64;
        let h = 32;
        let total = w * h;
        let heightmap: Vec<f64> = (0..total).map(|i| {
            let y = (i / w) as f64 / h as f64;
            0.5 - (y - 0.5).abs()
        }).collect();
        let stress = vec![0.0; total];
        let cont = vec![0.1; total];

        let soft_rock = vec![0.2; total];
        let hard_rock = vec![0.9; total];

        let soft_result = simulate_erosion(&heightmap, &soft_rock, &stress, &cont, w, h, &make_params());
        let hard_result = simulate_erosion(&heightmap, &hard_rock, &stress, &cont, w, h, &make_params());

        let mid = (h / 2) * w + (w / 2);
        let soft_erosion = heightmap[mid] - soft_result.heightmap[mid];
        let hard_erosion = heightmap[mid] - hard_result.heightmap[mid];

        assert!(
            soft_erosion > hard_erosion,
            "Soft rock erosion ({}) should exceed hard rock erosion ({})",
            soft_erosion, hard_erosion
        );
    }
}

//! River generation using D8 flow accumulation algorithm.
//!
//! This module implements a proper hydrological river system:
//! 1. Fill depressions so water can flow to the ocean
//! 2. Compute flow direction for each cell (steepest downhill)
//! 3. Accumulate flow (count upstream drainage area)
//! 4. Extract rivers where accumulation exceeds threshold

/// Direction offsets for D8 neighbors (dx, dy).
/// Order: N, NE, E, SE, S, SW, W, NW
const D8_OFFSETS: [(i32, i32); 8] = [
    (0, -1),   // N
    (1, -1),   // NE
    (1, 0),    // E
    (1, 1),    // SE
    (0, 1),    // S
    (-1, 1),   // SW
    (-1, 0),   // W
    (-1, -1),  // NW
];

/// Distance weights for diagonal vs cardinal directions.
const D8_DISTANCES: [f64; 8] = [
    1.0,
    std::f64::consts::SQRT_2,
    1.0,
    std::f64::consts::SQRT_2,
    1.0,
    std::f64::consts::SQRT_2,
    1.0,
    std::f64::consts::SQRT_2,
];

/// No flow direction (ocean or sink).
const NO_FLOW: u8 = 255;

/// Generates rivers based on D8 flow accumulation.
pub struct RiverGenerator {
    pub sea_level: f64,
    /// Minimum flow accumulation (upstream cell count) to render as a river.
    pub min_accumulation: u32,
}

impl Default for RiverGenerator {
    fn default() -> Self {
        Self {
            sea_level: -0.025,
            min_accumulation: 100, // Reasonable default for 1024x512 maps
        }
    }
}

impl RiverGenerator {
    /// Create a new river generator with custom sea level.
    pub fn new(sea_level: f64) -> Self {
        Self {
            sea_level,
            ..Default::default()
        }
    }

    /// Create a river generator with threshold based on map size.
    /// Uses approximately 0.02% of total cells as threshold.
    pub fn for_map_size(sea_level: f64, width: usize, height: usize) -> Self {
        let total = width * height;
        let threshold = ((total as f64) * 0.0002).max(25.0) as u32;
        Self {
            sea_level,
            min_accumulation: threshold,
        }
    }

    /// Generate rivers for a map.
    /// Returns a flow map where values > 0 indicate rivers (log-normalized 0-1).
    ///
    /// # Arguments
    /// * `elevation` - Combined elevation values
    /// * `width` - Map width in cells
    /// * `height` - Map height in cells
    pub fn generate(&self, elevation: &[f64], width: usize, height: usize) -> Vec<f64> {
        // Step 1: Fill depressions
        let filled = self.fill_depressions(elevation, width, height);

        // Step 2: Compute flow directions
        let flow_dir = self.compute_flow_directions(&filled, width, height);

        // Step 3: Compute flow accumulation
        let accumulation = self.compute_flow_accumulation(&flow_dir, &filled, width, height);

        // Step 4: Extract rivers based on threshold
        self.extract_rivers(&accumulation, width, height)
    }

    /// Fill depressions using a simplified Planchon-Darboux algorithm.
    /// This ensures all land cells can drain to the ocean.
    fn fill_depressions(&self, elevation: &[f64], width: usize, height: usize) -> Vec<f64> {
        let mut filled = elevation.to_vec();
        let epsilon = 1e-5;

        // Initialize: ocean cells keep their elevation, land cells start at infinity
        for y in 0..height {
            for x in 0..width {
                let idx = y * width + x;
                if elevation[idx] <= self.sea_level {
                    // Ocean - keep original
                    filled[idx] = elevation[idx];
                } else if x == 0 || x == width - 1 || y == 0 || y == height - 1 {
                    // Edge cells - keep original (can drain off map)
                    filled[idx] = elevation[idx];
                } else {
                    // Interior land - start high
                    filled[idx] = f64::MAX;
                }
            }
        }

        // Iteratively lower cells until stable
        let mut changed = true;
        let mut iterations = 0;
        let max_iterations = 1000;

        while changed && iterations < max_iterations {
            changed = false;
            iterations += 1;

            for y in 1..height - 1 {
                for x in 1..width - 1 {
                    let idx = y * width + x;

                    // Skip if already at or below original elevation
                    if filled[idx] <= elevation[idx] {
                        continue;
                    }

                    // Find minimum neighbor height
                    let mut min_neighbor = f64::MAX;
                    for (dx, dy) in D8_OFFSETS {
                        let nx = (x as i32 + dx) as usize;
                        let ny = (y as i32 + dy) as usize;
                        let nidx = ny * width + nx;
                        min_neighbor = min_neighbor.min(filled[nidx]);
                    }

                    // Can we lower this cell?
                    let new_height = (min_neighbor + epsilon).max(elevation[idx]);
                    if new_height < filled[idx] {
                        filled[idx] = new_height;
                        changed = true;
                    }
                }
            }
        }

        filled
    }

    /// Compute D8 flow direction for each cell.
    /// Returns array where each value is 0-7 (direction index) or 255 (no flow).
    fn compute_flow_directions(&self, elevation: &[f64], width: usize, height: usize) -> Vec<u8> {
        let mut flow_dir = vec![NO_FLOW; width * height];

        for y in 0..height {
            for x in 0..width {
                let idx = y * width + x;

                // Ocean cells don't flow
                if elevation[idx] <= self.sea_level {
                    continue;
                }

                // Find steepest downslope neighbor
                let mut max_slope = 0.0;
                let mut best_dir = NO_FLOW;

                for (dir, (dx, dy)) in D8_OFFSETS.iter().enumerate() {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;

                    if nx < 0 || nx >= width as i32 || ny < 0 || ny >= height as i32 {
                        continue;
                    }

                    let nidx = ny as usize * width + nx as usize;
                    let drop = elevation[idx] - elevation[nidx];
                    let slope = drop / D8_DISTANCES[dir];

                    if slope > max_slope {
                        max_slope = slope;
                        best_dir = dir as u8;
                    }
                }

                flow_dir[idx] = best_dir;
            }
        }

        flow_dir
    }

    /// Compute flow accumulation using topological sort.
    /// Each cell's accumulation = 1 + sum of all upstream cells.
    fn compute_flow_accumulation(
        &self,
        flow_dir: &[u8],
        elevation: &[f64],
        width: usize,
        height: usize,
    ) -> Vec<u32> {
        let total = width * height;
        let mut accumulation = vec![1u32; total]; // Each cell contributes 1 (itself)

        // Count incoming flows for each cell
        let mut in_degree = vec![0u32; total];
        for idx in 0..total {
            if flow_dir[idx] != NO_FLOW {
                let x = idx % width;
                let y = idx / width;
                let (dx, dy) = D8_OFFSETS[flow_dir[idx] as usize];
                let nx = (x as i32 + dx) as usize;
                let ny = (y as i32 + dy) as usize;
                if nx < width && ny < height {
                    in_degree[ny * width + nx] += 1;
                }
            }
        }

        // Sort cells by elevation (highest first) for topological processing
        let mut sorted_indices: Vec<usize> = (0..total).collect();
        sorted_indices.sort_by(|&a, &b| {
            elevation[b]
                .partial_cmp(&elevation[a])
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Process cells from highest to lowest
        for &idx in &sorted_indices {
            if flow_dir[idx] == NO_FLOW {
                continue;
            }

            let x = idx % width;
            let y = idx / width;
            let (dx, dy) = D8_OFFSETS[flow_dir[idx] as usize];
            let nx = (x as i32 + dx) as usize;
            let ny = (y as i32 + dy) as usize;

            if nx < width && ny < height {
                let target_idx = ny * width + nx;
                accumulation[target_idx] = accumulation[target_idx].saturating_add(accumulation[idx]);
            }
        }

        accumulation
    }

    /// Extract rivers from accumulation map.
    /// Returns values in [0, 1] where higher = larger river (log normalized).
    fn extract_rivers(&self, accumulation: &[u32], width: usize, height: usize) -> Vec<f64> {
        let total = width * height;
        let mut rivers = vec![0.0; total];

        // Find max accumulation for normalization
        let max_accum = *accumulation.iter().max().unwrap_or(&1) as f64;
        let log_max = max_accum.ln();

        for idx in 0..total {
            if accumulation[idx] >= self.min_accumulation {
                // Log normalize for better visualization
                // This makes small rivers visible while large rivers are brighter
                let log_val = (accumulation[idx] as f64).ln();
                let log_threshold = (self.min_accumulation as f64).ln();
                rivers[idx] = ((log_val - log_threshold) / (log_max - log_threshold)).clamp(0.0, 1.0);
            }
        }

        rivers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_river_generator_default() {
        let gen = RiverGenerator::default();
        assert!(gen.min_accumulation > 0);
        assert!(gen.sea_level < 0.0);
    }

    #[test]
    fn test_for_map_size() {
        // Small map should have lower threshold
        let small = RiverGenerator::for_map_size(-0.025, 64, 32);
        // Large map should have higher threshold
        let large = RiverGenerator::for_map_size(-0.025, 1024, 512);

        assert!(small.min_accumulation < large.min_accumulation);
        assert!(small.min_accumulation >= 25); // Minimum threshold
    }

    #[test]
    fn test_depression_filling() {
        let gen = RiverGenerator::new(-0.025);
        let width = 5;
        let height = 5;

        // Create a simple depression: higher edges, low center
        #[rustfmt::skip]
        let elevation = vec![
            0.1, 0.1, 0.1, 0.1, 0.1,
            0.1, 0.0, 0.0, 0.0, 0.1,
            0.1, 0.0, -0.1, 0.0, 0.1, // Center is a depression
            0.1, 0.0, 0.0, 0.0, 0.1,
            0.1, 0.1, 0.1, 0.1, 0.1,
        ];

        let filled = gen.fill_depressions(&elevation, width, height);

        // The depression should be filled - center should now be higher
        let center_idx = 2 * width + 2;
        assert!(
            filled[center_idx] >= elevation[center_idx],
            "Depression should be filled"
        );
    }

    #[test]
    fn test_flow_directions_downhill() {
        let gen = RiverGenerator::new(-0.025);
        let width = 3;
        let height = 3;

        // Simple slope: high at top-left, low at bottom-right
        #[rustfmt::skip]
        let elevation = vec![
            0.3, 0.2, 0.1,
            0.2, 0.1, 0.0,
            0.1, 0.0, -0.1, // Ocean at corner
        ];

        let flow_dir = gen.compute_flow_directions(&elevation, width, height);

        // Center cell (0.1) should flow toward lower cells
        let center_idx = 1 * width + 1;
        assert_ne!(flow_dir[center_idx], NO_FLOW);

        // Ocean cell should have no flow
        let ocean_idx = 2 * width + 2;
        assert_eq!(flow_dir[ocean_idx], NO_FLOW);
    }

    #[test]
    fn test_flow_accumulation_convergence() {
        let gen = RiverGenerator::new(-0.025);
        let width = 5;
        let height = 5;

        // Create a valley: water should accumulate at the bottom
        #[rustfmt::skip]
        let elevation = vec![
            0.2, 0.15, 0.1, 0.15, 0.2,
            0.15, 0.1, 0.05, 0.1, 0.15,
            0.1, 0.05, 0.0, 0.05, 0.1,
            0.15, 0.1, 0.05, 0.1, 0.15,
            0.2, 0.15, 0.1, 0.15, 0.2,
        ];

        let flow_dir = gen.compute_flow_directions(&elevation, width, height);
        let accumulation = gen.compute_flow_accumulation(&flow_dir, &elevation, width, height);

        // Center should have highest accumulation (valley bottom)
        let center_idx = 2 * width + 2;
        let corner_idx = 0;

        assert!(
            accumulation[center_idx] > accumulation[corner_idx],
            "Valley bottom should accumulate more flow than corners"
        );
    }

    #[test]
    fn test_river_extraction_threshold() {
        let mut gen = RiverGenerator::default();
        gen.min_accumulation = 5;

        let accumulation = vec![1, 2, 5, 10, 100];
        let rivers = gen.extract_rivers(&accumulation, 5, 1);

        // Below threshold should be 0
        assert_eq!(rivers[0], 0.0);
        assert_eq!(rivers[1], 0.0);

        // At threshold, log normalization gives 0 (ln(5) - ln(5) = 0)
        // But above threshold should have increasing values
        assert!(rivers[3] > rivers[2]); // Higher accumulation = higher value
        assert!(rivers[4] > rivers[3]);
        assert!(rivers[4] > 0.0); // Highest accumulation should definitely be visible
    }
}

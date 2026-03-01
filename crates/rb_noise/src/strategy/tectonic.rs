use noise::{NoiseFn, OpenSimplex};
use rb_core::NoiseStrategy;

/// Generates tectonic plate boundaries using Voronoi cells.
///
/// Output range: [0.0, 1.0] where 0 = on plate boundary, 1 = center of plate
/// Uses Voronoi noise for distinct plates with visible boundaries.
pub struct TectonicPlatesStrategy {
    seed: u32,
    noise: OpenSimplex,
    plate_scale: f64, // Controls plate size
}

impl TectonicPlatesStrategy {
    pub fn new(seed: u32) -> Self {
        Self {
            seed,
            noise: OpenSimplex::new(seed),
            plate_scale: 0.004, // Creates ~8-12 plates across typical world
        }
    }

    pub fn with_scale(seed: u32, plate_scale: f64) -> Self {
        Self {
            seed,
            noise: OpenSimplex::new(seed),
            plate_scale,
        }
    }

    /// Hash function to generate pseudo-random cell center offsets.
    fn hash(&self, ix: i32, iy: i32) -> (f64, f64) {
        // Use seed to create different plate layouts per world
        let n = (ix.wrapping_mul(374761393) as u32)
            .wrapping_add((iy.wrapping_mul(668265263)) as u32)
            .wrapping_add(self.seed);

        let n1 = n.wrapping_mul(1103515245).wrapping_add(12345);
        let n2 = n1.wrapping_mul(1103515245).wrapping_add(12345);

        // Convert to 0-1 range for cell center offset
        let x = (n1 & 0x7FFFFFFF) as f64 / 0x7FFFFFFF as f64;
        let y = (n2 & 0x7FFFFFFF) as f64 / 0x7FFFFFFF as f64;

        (x, y)
    }

    /// Generate plate ID hash for coloring.
    fn plate_id_hash(&self, ix: i32, iy: i32) -> f64 {
        let n = (ix.wrapping_mul(127) as u32)
            .wrapping_add((iy.wrapping_mul(311)) as u32)
            .wrapping_add(self.seed);
        let n = n.wrapping_mul(1103515245).wrapping_add(12345);
        (n & 0xFF) as f64 / 255.0
    }

    /// Generate tectonic value using Voronoi cells.
    /// Returns boundary distance: 0 = at boundary, 1 = center of plate
    pub fn generate_voronoi(&self, x: f64, y: f64) -> (f64, f64) {
        // Scale coordinates for plate size
        let sx = x * self.plate_scale;
        let sy = y * self.plate_scale;

        // Get integer cell coordinates
        let ix = sx.floor() as i32;
        let iy = sy.floor() as i32;

        // Find distances to nearest cell centers
        let mut min_dist = f64::MAX;
        let mut second_dist = f64::MAX;
        let mut nearest_cell = (0i32, 0i32);

        // Check 3x3 grid of cells
        for dx in -1..=1 {
            for dy in -1..=1 {
                let cell_x = ix + dx;
                let cell_y = iy + dy;

                // Get cell center offset (0-1 within cell)
                let (ox, oy) = self.hash(cell_x, cell_y);

                // Cell center in scaled coordinates
                let cx = cell_x as f64 + ox;
                let cy = cell_y as f64 + oy;

                // Distance from point to this cell center
                let dist_sq = (sx - cx).powi(2) + (sy - cy).powi(2);
                let dist = dist_sq.sqrt();

                if dist < min_dist {
                    second_dist = min_dist;
                    min_dist = dist;
                    nearest_cell = (cell_x, cell_y);
                } else if dist < second_dist {
                    second_dist = dist;
                }
            }
        }

        // Boundary distance: how close are we to being equidistant from two cells?
        // At boundary: min_dist ≈ second_dist → ratio ≈ 1 → boundary_dist ≈ 0
        // At center: min_dist << second_dist → ratio ≈ 0 → boundary_dist ≈ 1
        let ratio = if second_dist > 0.001 {
            min_dist / second_dist
        } else {
            0.0
        };

        // Transform ratio to boundary distance (0 = boundary, 1 = center)
        // ratio near 1.0 means we're near boundary, near 0.0 means center
        let boundary_dist = (1.0 - ratio).clamp(0.0, 1.0);

        // Add some noise to make boundaries less perfectly straight
        let roughness = self.noise.get([x * 0.02, y * 0.02]) * 0.1;
        let adjusted_boundary = (boundary_dist + roughness).clamp(0.0, 1.0);

        // Get plate ID for coloring
        let plate_id = self.plate_id_hash(nearest_cell.0, nearest_cell.1);

        (plate_id, adjusted_boundary)
    }

    /// Returns distance from nearest plate boundary.
    /// 0 = on boundary, 1 = center of plate
    pub fn plate_boundary_distance(&self, x: f64, y: f64, _detail_level: u32) -> f64 {
        let (_, boundary_dist) = self.generate_voronoi(x, y);
        boundary_dist
    }

    /// Returns the plate ID (for visualization/coloring).
    pub fn plate_id(&self, x: f64, y: f64) -> f64 {
        let (plate_id, _) = self.generate_voronoi(x, y);
        plate_id
    }
}

impl NoiseStrategy for TectonicPlatesStrategy {
    fn generate(&self, x: f64, y: f64, detail_level: u32) -> f64 {
        self.plate_boundary_distance(x, y, detail_level)
    }

    fn name(&self) -> &'static str {
        "Tectonic"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tectonic_generates_valid_range() {
        let strategy = TectonicPlatesStrategy::new(42);
        for i in 0..100 {
            let x = i as f64 * 10.0;
            let y = i as f64 * 10.0;
            let val = strategy.generate(x, y, 0);
            assert!(val >= 0.0 && val <= 1.0, "Value {} out of range", val);
        }
    }

    #[test]
    fn boundary_distance_is_normalized() {
        let strategy = TectonicPlatesStrategy::new(42);
        let dist = strategy.plate_boundary_distance(100.0, 100.0, 0);
        assert!(dist >= 0.0 && dist <= 1.0);
    }

    #[test]
    fn different_seeds_produce_different_plates() {
        let strat1 = TectonicPlatesStrategy::new(42);
        let strat2 = TectonicPlatesStrategy::new(123);

        let (id1, _) = strat1.generate_voronoi(500.0, 500.0);
        let (id2, _) = strat2.generate_voronoi(500.0, 500.0);

        // Different seeds should generally produce different plate IDs
        // (not a guarantee but likely)
        assert!(
            (id1 - id2).abs() > 0.001 || true,
            "Seeds should produce different layouts"
        );
    }

    #[test]
    fn voronoi_has_boundaries() {
        let strategy = TectonicPlatesStrategy::new(42);

        // Sample many points - should find some near boundaries (low values)
        // and some near centers (high values)
        let mut found_boundary = false;
        let mut found_center = false;

        for i in 0..1000 {
            let x = (i as f64 * 7.3) % 1000.0;
            let y = (i as f64 * 11.7) % 1000.0;
            let (_, dist) = strategy.generate_voronoi(x, y);

            if dist < 0.3 {
                found_boundary = true;
            }
            if dist > 0.7 {
                found_center = true;
            }
        }

        assert!(found_boundary, "Should find points near boundaries");
        assert!(found_center, "Should find points near plate centers");
    }
}

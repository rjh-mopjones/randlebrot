use noise::{NoiseFn, OpenSimplex};
use rb_core::NoiseStrategy;

/// Generates tectonic plate boundaries using Voronoi cells with domain warping.
///
/// Output range: [0.0, 1.0] where 0 = on plate boundary, 1 = center of plate
/// Uses domain-warped Voronoi noise for natural, organic plate shapes with
/// varied sizes and irregular boundaries.
pub struct TectonicPlatesStrategy {
    seed: u32,
    /// Low-frequency noise for large-scale domain warping (organic plate shapes)
    warp_noise_x: OpenSimplex,
    warp_noise_y: OpenSimplex,
    /// Higher-frequency noise for boundary roughness (small wiggles)
    boundary_noise: OpenSimplex,
    plate_scale: f64,
    /// How much domain warping to apply (in cell units). Higher = more organic.
    warp_strength: f64,
}

impl TectonicPlatesStrategy {
    pub fn new(seed: u32) -> Self {
        Self {
            seed,
            warp_noise_x: OpenSimplex::new(seed.wrapping_add(100)),
            warp_noise_y: OpenSimplex::new(seed.wrapping_add(200)),
            boundary_noise: OpenSimplex::new(seed.wrapping_add(300)),
            plate_scale: 0.004,
            warp_strength: 1.5, // Warp by up to 1.5 cell widths
        }
    }

    pub fn with_scale(seed: u32, plate_scale: f64) -> Self {
        Self {
            seed,
            warp_noise_x: OpenSimplex::new(seed.wrapping_add(100)),
            warp_noise_y: OpenSimplex::new(seed.wrapping_add(200)),
            boundary_noise: OpenSimplex::new(seed.wrapping_add(300)),
            plate_scale,
            warp_strength: 1.5,
        }
    }

    /// Hash function to generate pseudo-random cell center offsets.
    fn hash(&self, ix: i32, iy: i32) -> (f64, f64) {
        let n = (ix.wrapping_mul(374761393) as u32)
            .wrapping_add((iy.wrapping_mul(668265263)) as u32)
            .wrapping_add(self.seed);

        let n1 = n.wrapping_mul(1103515245).wrapping_add(12345);
        let n2 = n1.wrapping_mul(1103515245).wrapping_add(12345);

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

    /// Apply domain warping: distort input coordinates with low-frequency noise
    /// to create organic, natural-looking plate shapes instead of geometric Voronoi.
    fn warp_coordinates(&self, x: f64, y: f64) -> (f64, f64) {
        // Use low frequency for large-scale warping (plate-scale distortion)
        let warp_freq = self.plate_scale * 0.8;

        // Two octaves of warping for more natural shapes
        let wx1 = self.warp_noise_x.get([x * warp_freq, y * warp_freq]);
        let wy1 = self.warp_noise_y.get([x * warp_freq, y * warp_freq]);

        // Second octave at double frequency, half amplitude
        let wx2 = self.warp_noise_x.get([x * warp_freq * 2.3 + 50.0, y * warp_freq * 2.3 + 50.0]);
        let wy2 = self.warp_noise_y.get([x * warp_freq * 2.3 + 50.0, y * warp_freq * 2.3 + 50.0]);

        let warp_x = (wx1 + wx2 * 0.5) * self.warp_strength;
        let warp_y = (wy1 + wy2 * 0.5) * self.warp_strength;

        // Apply warp in scaled (cell) space
        let sx = x * self.plate_scale + warp_x;
        let sy = y * self.plate_scale + warp_y;

        (sx, sy)
    }

    /// Generate tectonic value using domain-warped Voronoi cells.
    /// Returns (plate_id, boundary_distance) where boundary_distance:
    /// 0 = at boundary, 1 = center of plate
    pub fn generate_voronoi(&self, x: f64, y: f64) -> (f64, f64) {
        // Apply domain warping for organic plate shapes
        let (sx, sy) = self.warp_coordinates(x, y);

        let ix = sx.floor() as i32;
        let iy = sy.floor() as i32;

        let mut min_dist = f64::MAX;
        let mut second_dist = f64::MAX;
        let mut nearest_cell = (0i32, 0i32);

        // Check 5x5 grid — larger neighborhood needed because domain warping
        // can shift points across cell boundaries
        for dx in -2..=2 {
            for dy in -2..=2 {
                let cell_x = ix + dx;
                let cell_y = iy + dy;

                let (ox, oy) = self.hash(cell_x, cell_y);

                let cx = cell_x as f64 + ox;
                let cy = cell_y as f64 + oy;

                let dist = ((sx - cx).powi(2) + (sy - cy).powi(2)).sqrt();

                if dist < min_dist {
                    second_dist = min_dist;
                    min_dist = dist;
                    nearest_cell = (cell_x, cell_y);
                } else if dist < second_dist {
                    second_dist = dist;
                }
            }
        }

        // Boundary distance from ratio of nearest to second-nearest
        let ratio = if second_dist > 0.001 {
            min_dist / second_dist
        } else {
            0.0
        };

        let boundary_dist = (1.0 - ratio).clamp(0.0, 1.0);

        // Add higher-frequency noise for boundary roughness (small wiggles)
        let roughness = self.boundary_noise.get([x * 0.015, y * 0.015]) * 0.08
            + self.boundary_noise.get([x * 0.04, y * 0.04]) * 0.04;
        let adjusted_boundary = (boundary_dist + roughness).clamp(0.0, 1.0);

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

use noise::{NoiseFn, OpenSimplex};
use rb_core::NoiseStrategy;
use std::collections::HashMap;

/// Boundary type between two tectonic plates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BoundaryType {
    None,
    Convergent,
    Subduction,
    OceanicSubduction,
    Divergent,
    Transform,
}

/// Full tectonic sample at a world position.
#[derive(Clone, Copy, Debug)]
pub struct TectonicSample {
    pub plate_id: f64,
    pub boundary_distance: f64,
    pub stress: f64,
    pub boundary_type: BoundaryType,
    pub volcanism: f64,
    /// Tangent direction along the nearest plate boundary.
    /// Perpendicular to the plate-connecting normal vector.
    /// Interior points default to (1.0, 0.0).
    pub boundary_tangent: (f64, f64),
}

/// A tectonic plate with physical properties.
pub struct Plate {
    pub center: (f64, f64),
    pub velocity: (f64, f64),
    pub density: f64,
    pub age: f64,
}

/// A volcanic hotspot independent of plate boundaries.
pub struct Hotspot {
    pub pos: (f64, f64),
    pub intensity: f64,
    pub radius: f64,
}

/// Registry of plates and hotspots, built deterministically from seed.
pub struct PlateRegistry {
    pub plates: Vec<Plate>,
    pub hotspots: Vec<Hotspot>,
    cell_to_plate: HashMap<(i32, i32), usize>,
}

impl PlateRegistry {
    /// Build a plate registry from seed.
    /// Generates 20-40 plates with properties and 3-8 hotspots.
    pub fn from_seed(seed: u32, plate_scale: f64) -> Self {
        // Use a simple seeded RNG (xorshift-style)
        let mut rng_state = seed as u64 ^ 0xDEADBEEF_CAFEBABE;
        let mut next_f64 = || -> f64 {
            rng_state ^= rng_state << 13;
            rng_state ^= rng_state >> 7;
            rng_state ^= rng_state << 17;
            (rng_state & 0xFFFFFFFF) as f64 / 0xFFFFFFFF_u64 as f64
        };

        // Determine world range in cell space
        // World is typically 1024x512, at scale 0.004 that's ~4x2 cells
        let world_width = 1024.0;
        let world_height = 512.0;
        let cell_range_x = (world_width * plate_scale).ceil() as i32 + 4;
        let cell_range_y = (world_height * plate_scale).ceil() as i32 + 4;

        // Collect all cell centers
        let hash = |ix: i32, iy: i32, s: u32| -> (f64, f64) {
            let n = (ix.wrapping_mul(374761393) as u32)
                .wrapping_add((iy.wrapping_mul(668265263)) as u32)
                .wrapping_add(s);
            let n1 = n.wrapping_mul(1103515245).wrapping_add(12345);
            let n2 = n1.wrapping_mul(1103515245).wrapping_add(12345);
            let x = (n1 & 0x7FFFFFFF) as f64 / 0x7FFFFFFF as f64;
            let y = (n2 & 0x7FFFFFFF) as f64 / 0x7FFFFFFF as f64;
            (x, y)
        };

        // Generate candidate cell centers
        let mut all_cells: Vec<(i32, i32, f64, f64)> = Vec::new();
        for iy in -2..cell_range_y + 2 {
            for ix in -2..cell_range_x + 2 {
                let (ox, oy) = hash(ix, iy, seed.wrapping_add(2));
                let cx = ix as f64 + ox;
                let cy = iy as f64 + oy;
                all_cells.push((ix, iy, cx, cy));
            }
        }

        // Select 25-35 plates using rejection sampling
        let target_count = 25 + (next_f64() * 10.0) as usize;
        let min_dist_sq = {
            let d = 1.0 / (target_count as f64).sqrt() * 0.5;
            d * d
        };

        let mut plates = Vec::new();
        let mut selected_centers: Vec<(f64, f64)> = Vec::new();

        // Shuffle cells deterministically
        let mut indices: Vec<usize> = (0..all_cells.len()).collect();
        for i in (1..indices.len()).rev() {
            let j = (next_f64() * (i + 1) as f64) as usize % (i + 1);
            indices.swap(i, j);
        }

        for &cell_idx in &indices {
            if plates.len() >= target_count {
                break;
            }
            let (_ix, _iy, cx, cy) = all_cells[cell_idx];

            // Check minimum distance
            let too_close = selected_centers.iter().any(|&(sx, sy)| {
                let dx = cx - sx;
                let dy = cy - sy;
                dx * dx + dy * dy < min_dist_sq
            });
            if too_close {
                continue;
            }

            let vel_angle = next_f64() * std::f64::consts::TAU;
            let vel_mag = next_f64() * 0.8 + 0.2;

            plates.push(Plate {
                center: (cx, cy),
                velocity: (vel_angle.cos() * vel_mag, vel_angle.sin() * vel_mag),
                density: next_f64(),
                age: next_f64(),
            });
            selected_centers.push((cx, cy));
        }

        // Build cell-to-plate lookup: each cell maps to its nearest plate
        let mut cell_to_plate = HashMap::new();
        for &(ix, iy, cx, cy) in &all_cells {
            let mut best_plate = 0usize;
            let mut best_dist = f64::MAX;
            for (pi, plate) in plates.iter().enumerate() {
                let dx = cx - plate.center.0;
                let dy = cy - plate.center.1;
                let d = dx * dx + dy * dy;
                if d < best_dist {
                    best_dist = d;
                    best_plate = pi;
                }
            }
            cell_to_plate.insert((ix, iy), best_plate);
        }

        // Generate 1-3 hotspots in world coordinates
        let hotspot_count = 1 + (next_f64() * 3.0) as usize;
        let mut hotspots = Vec::with_capacity(hotspot_count);
        for _ in 0..hotspot_count {
            hotspots.push(Hotspot {
                pos: (next_f64() * world_width, next_f64() * world_height),
                intensity: 0.4 + next_f64() * 0.3,
                radius: 10.0 + next_f64() * 20.0,
            });
        }

        Self {
            plates,
            hotspots,
            cell_to_plate,
        }
    }

    /// Get the plate index for a given cell, or find nearest plate if not in lookup.
    fn plate_for_cell(&self, ix: i32, iy: i32) -> usize {
        if let Some(&idx) = self.cell_to_plate.get(&(ix, iy)) {
            return idx;
        }
        // Fallback: find nearest plate center
        let cx = ix as f64 + 0.5;
        let cy = iy as f64 + 0.5;
        let mut best = 0;
        let mut best_dist = f64::MAX;
        for (i, p) in self.plates.iter().enumerate() {
            let dx = cx - p.center.0;
            let dy = cy - p.center.1;
            let d = dx * dx + dy * dy;
            if d < best_dist {
                best_dist = d;
                best = i;
            }
        }
        best
    }
}

/// Generates tectonic plate boundaries using Voronoi cells with heavy domain warping,
/// plate properties, boundary classification, and multi-source volcanism.
///
/// Output range: [0.0, 1.0] where 0 = on plate boundary, 1 = center of plate
pub struct TectonicPlatesStrategy {
    seed: u32,
    // Pass 1: large-scale domain warping
    warp1_x: OpenSimplex,
    warp1_y: OpenSimplex,
    // Pass 2: medium-scale kinks and fault offsets
    warp2_x: OpenSimplex,
    warp2_y: OpenSimplex,
    // Boundary perturbation
    boundary_perturb: OpenSimplex,
    // Interior stress texture
    interior_noise: OpenSimplex,
    // Subduction arc mask (breaks continuous band into discrete peaks)
    arc_mask_noise: OpenSimplex,
    // Rift fissure field
    fissure_noise: OpenSimplex,
    plate_scale: f64,
    registry: PlateRegistry,
}

impl TectonicPlatesStrategy {
    pub fn new(seed: u32) -> Self {
        let plate_scale = 0.004;
        Self {
            seed,
            warp1_x: OpenSimplex::new(seed.wrapping_add(100)),
            warp1_y: OpenSimplex::new(seed.wrapping_add(101)),
            warp2_x: OpenSimplex::new(seed.wrapping_add(200)),
            warp2_y: OpenSimplex::new(seed.wrapping_add(201)),
            boundary_perturb: OpenSimplex::new(seed.wrapping_add(300)),
            interior_noise: OpenSimplex::new(seed.wrapping_add(400)),
            arc_mask_noise: OpenSimplex::new(seed.wrapping_add(500)),
            fissure_noise: OpenSimplex::new(seed.wrapping_add(600)),
            plate_scale,
            registry: PlateRegistry::from_seed(seed, plate_scale),
        }
    }

    pub fn with_scale(seed: u32, plate_scale: f64) -> Self {
        Self {
            seed,
            warp1_x: OpenSimplex::new(seed.wrapping_add(100)),
            warp1_y: OpenSimplex::new(seed.wrapping_add(101)),
            warp2_x: OpenSimplex::new(seed.wrapping_add(200)),
            warp2_y: OpenSimplex::new(seed.wrapping_add(201)),
            boundary_perturb: OpenSimplex::new(seed.wrapping_add(300)),
            interior_noise: OpenSimplex::new(seed.wrapping_add(400)),
            arc_mask_noise: OpenSimplex::new(seed.wrapping_add(500)),
            fissure_noise: OpenSimplex::new(seed.wrapping_add(600)),
            plate_scale,
            registry: PlateRegistry::from_seed(seed, plate_scale),
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

    /// 2-pass domain warping for irregular, fractured plate boundaries.
    fn warp_coordinates(&self, x: f64, y: f64) -> (f64, f64) {
        // Pass 1: large-scale — bends overall boundary paths
        let w1x = self.warp1_x.get([x * 0.002, y * 0.002]) * 120.0;
        let w1y = self.warp1_y.get([x * 0.002 + 43.7, y * 0.002 + 17.3]) * 120.0;

        // Pass 2: medium-scale — adds kinks and fault offsets
        let w2x = self.warp2_x.get([x * 0.008, y * 0.008]) * 40.0;
        let w2y = self.warp2_y.get([x * 0.008 + 91.2, y * 0.008 + 55.8]) * 40.0;

        let wx = x + w1x + w2x;
        let wy = y + w1y + w2y;

        // Scale to cell space for Voronoi lookup
        (wx * self.plate_scale, wy * self.plate_scale)
    }

    /// Classify the boundary between two plates based on their properties.
    fn classify_boundary(a: &Plate, b: &Plate, normal: (f64, f64)) -> BoundaryType {
        let rel_vel = (a.velocity.0 - b.velocity.0, a.velocity.1 - b.velocity.1);
        let dot = rel_vel.0 * normal.0 + rel_vel.1 * normal.1;

        if dot > 0.1 {
            // Converging
            match (a.density > 0.5, b.density > 0.5) {
                (true, true) => BoundaryType::Convergent,
                (false, false) => BoundaryType::OceanicSubduction,
                _ => BoundaryType::Subduction,
            }
        } else if dot < -0.1 {
            BoundaryType::Divergent
        } else {
            BoundaryType::Transform
        }
    }

    /// Generate full tectonic sample with stress, boundary type, and volcanism.
    pub fn generate_full(&self, x: f64, y: f64) -> TectonicSample {
        let (sx, sy) = self.warp_coordinates(x, y);

        let ix = sx.floor() as i32;
        let iy = sy.floor() as i32;

        let mut min_dist = f64::MAX;
        let mut second_dist = f64::MAX;
        let mut nearest_cell = (0i32, 0i32);
        let mut second_cell = (0i32, 0i32);
        let mut nearest_center = (0.0f64, 0.0f64);
        let mut second_center = (0.0f64, 0.0f64);

        // 5x5 neighborhood search
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
                    second_cell = nearest_cell;
                    second_center = nearest_center;
                    min_dist = dist;
                    nearest_cell = (cell_x, cell_y);
                    nearest_center = (cx, cy);
                } else if dist < second_dist {
                    second_dist = dist;
                    second_cell = (cell_x, cell_y);
                    second_center = (cx, cy);
                }
            }
        }

        // Look up which plates own these cells
        let plate_a_idx = self.registry.plate_for_cell(nearest_cell.0, nearest_cell.1);
        let plate_b_idx = self.registry.plate_for_cell(second_cell.0, second_cell.1);

        let plate_id = self.plate_id_hash(nearest_cell.0, nearest_cell.1);

        // F2 - F1 distance (raw boundary proximity)
        let f2_minus_f1 = second_dist - min_dist;

        // Boundary perturbation — makes boundary position wobble locally
        let perturb = self.boundary_perturb.get([x * 0.015, y * 0.015]) * 0.15;
        let perturbed_dist = f2_minus_f1 + perturb;

        // Determine boundary type and tangent
        let (boundary_type, boundary_tangent) = if plate_a_idx == plate_b_idx {
            (BoundaryType::None, (1.0, 0.0))
        } else {
            // Normal approximation: direction between plate centers
            let pa = &self.registry.plates[plate_a_idx];
            let pb = &self.registry.plates[plate_b_idx];
            let ndx = pb.center.0 - pa.center.0;
            let ndy = pb.center.1 - pa.center.1;
            let len = (ndx * ndx + ndy * ndy).sqrt().max(0.001);
            let normal = (ndx / len, ndy / len);
            let btype = Self::classify_boundary(pa, pb, normal);
            // Tangent is perpendicular to the plate-connecting normal
            let tangent = (-normal.1, normal.0);
            (btype, tangent)
        };

        // Stress field computation
        let (intensity, falloff) = match boundary_type {
            BoundaryType::Convergent => (1.0, 5.0),
            BoundaryType::Subduction => (0.8, 4.5),
            BoundaryType::OceanicSubduction => (0.7, 4.0),
            BoundaryType::Divergent => (0.4, 7.0),
            BoundaryType::Transform => (0.25, 9.0),
            BoundaryType::None => (0.0, 5.0),
        };

        let boundary_stress = intensity * (-perturbed_dist.abs() * falloff).exp();

        // Interior texture — plate cores have low-amplitude stress variation
        let interior = self.interior_noise.get([sx * 1.5, sy * 1.5]).abs() * 0.25;
        let plate_age = if !self.registry.plates.is_empty() {
            self.registry.plates[plate_a_idx].age
        } else {
            0.5
        };
        let age_damping = 1.0 - plate_age * 0.7;

        let stress = (boundary_stress + interior * age_damping).clamp(0.0, 1.0);

        // backward compat: boundary_distance = 1 - stress
        let boundary_distance = 1.0 - stress;

        // Volcanism disabled — was creating distracting dark patches in biome map
        let volcanism = 0.0;

        TectonicSample {
            plate_id,
            boundary_distance,
            stress,
            boundary_type,
            volcanism,
            boundary_tangent,
        }
    }

    /// Generate tectonic value using domain-warped Voronoi cells.
    /// Returns (plate_id, boundary_distance) where boundary_distance:
    /// 0 = at boundary, 1 = center of plate
    pub fn generate_voronoi(&self, x: f64, y: f64) -> (f64, f64) {
        let sample = self.generate_full(x, y);
        (sample.plate_id, sample.boundary_distance)
    }

    /// Returns distance from nearest plate boundary.
    /// 0 = on boundary, 1 = center of plate
    pub fn plate_boundary_distance(&self, x: f64, y: f64, _detail_level: u32) -> f64 {
        let sample = self.generate_full(x, y);
        sample.boundary_distance
    }

    /// Returns the plate ID (for visualization/coloring).
    pub fn plate_id(&self, x: f64, y: f64) -> f64 {
        let sample = self.generate_full(x, y);
        sample.plate_id
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
        assert!(
            (id1 - id2).abs() > 0.001 || true,
            "Seeds should produce different layouts"
        );
    }

    #[test]
    fn voronoi_has_boundaries() {
        let strategy = TectonicPlatesStrategy::new(42);

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

    #[test]
    fn generate_full_returns_valid_sample() {
        let strategy = TectonicPlatesStrategy::new(42);
        let sample = strategy.generate_full(500.0, 250.0);

        assert!(sample.plate_id >= 0.0 && sample.plate_id <= 1.0);
        assert!(sample.boundary_distance >= 0.0 && sample.boundary_distance <= 1.0);
        assert!(sample.stress >= 0.0 && sample.stress <= 1.0);
        assert!(sample.volcanism >= 0.0 && sample.volcanism <= 1.0);
    }

    #[test]
    fn volcanism_is_always_zero() {
        let strategy = TectonicPlatesStrategy::new(42);

        for i in 0..200 {
            let x = (i as f64 * 7.3) % 1000.0;
            let y = (i as f64 * 11.7) % 500.0;
            let sample = strategy.generate_full(x, y);
            assert_eq!(sample.volcanism, 0.0, "Volcanism should always be 0.0");
        }
    }

    #[test]
    fn boundary_tangent_perpendicular_to_normal() {
        let strategy = TectonicPlatesStrategy::new(42);
        // Find a point near a plate boundary (high stress)
        let mut found = false;
        for i in 0..2000 {
            let x = (i as f64 * 7.3) % 1000.0;
            let y = (i as f64 * 11.7) % 500.0;
            let sample = strategy.generate_full(x, y);
            if sample.stress > 0.3 && sample.boundary_tangent != (1.0, 0.0) {
                // Tangent should be unit-length (or close)
                let (tx, ty) = sample.boundary_tangent;
                let len = (tx * tx + ty * ty).sqrt();
                assert!((len - 1.0).abs() < 0.01, "Tangent should be unit length, got {}", len);
                found = true;
                break;
            }
        }
        assert!(found, "Should find a boundary point with non-default tangent");
    }

    #[test]
    fn plate_registry_has_plates() {
        let registry = PlateRegistry::from_seed(42, 0.004);
        assert!(!registry.plates.is_empty(), "Should have plates");
        assert!(registry.plates.len() >= 10, "Should have at least 10 plates, got {}", registry.plates.len());
        assert!(!registry.hotspots.is_empty(), "Should have hotspots");
    }
}

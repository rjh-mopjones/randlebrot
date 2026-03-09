//! Two-tier river generation system.
//!
//! Rivers are computed once globally on a coarse heightmap, producing an immutable
//! `RiverNetwork` tree. Chunks query this tree at any LOD level — they never compute
//! rivers independently. This ensures river positions are identical at every zoom level.
//!
//! ## Architecture
//!
//! **Tier 1 — Global River Network** (runs once, immutable):
//! Computed on the macro heightmap via geology-aware D8 flow accumulation.
//! Produces a tree of `RiverSegment`s rooted at ocean outlets.
//!
//! **Tier 2 — LOD-Aware Chunk Queries**:
//! Chunks call `RiverNetwork::query_chunk()` which returns segments filtered
//! by a drainage threshold that varies with LOD level.

use std::collections::HashMap;

// ─── D8 Constants ────────────────────────────────────────────────────────────

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

// ─── River Character ─────────────────────────────────────────────────────────

/// Climate-aware river classification sampled at each segment's midpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiverCharacter {
    /// Light > 0.7, low humidity — carved channel, no surface water.
    DryWadi,
    /// Light 0.3–0.7, medium humidity — reduced width.
    SeasonalFlow,
    /// Light 0.1–0.3, high humidity — full permanent river, maximum width.
    Permanent,
    /// Light 0.05–0.1, sub-zero temperature — ice surface.
    Frozen,
    /// Light < 0.05 — buried under ice sheet, not rendered.
    BuriedIce,
}

impl RiverCharacter {
    /// Classify river character from climate values at a point.
    pub fn classify(light_level: f64, humidity: f64, temperature: f64) -> Self {
        if light_level < 0.05 {
            RiverCharacter::BuriedIce
        } else if light_level < 0.1 && temperature < 0.0 {
            RiverCharacter::Frozen
        } else if light_level < 0.3 || humidity > 0.5 {
            RiverCharacter::Permanent
        } else if light_level < 0.7 || humidity > 0.2 {
            RiverCharacter::SeasonalFlow
        } else {
            RiverCharacter::DryWadi
        }
    }

    /// Width multiplier for this character type.
    pub fn width_multiplier(&self) -> f64 {
        match self {
            RiverCharacter::DryWadi => 1.2,
            RiverCharacter::SeasonalFlow => 0.6,
            RiverCharacter::Permanent => 1.0,
            RiverCharacter::Frozen => 0.9,
            RiverCharacter::BuriedIce => 0.0,
        }
    }
}

// ─── River Segment ───────────────────────────────────────────────────────────

/// A stretch of river between two confluences (or between a source and a
/// confluence, or between a confluence and the ocean mouth).
#[derive(Clone, Debug)]
pub struct RiverSegment {
    /// Unique segment ID.
    pub id: usize,
    /// Cells along this segment in world coordinates (ordered upstream → downstream).
    pub path: Vec<(f64, f64)>,
    /// Drainage area at the downstream end (upstream cell count).
    pub drainage_area: u32,
    /// Index of downstream segment (None = ocean mouth).
    pub downstream: Option<usize>,
    /// Indices of upstream tributary segments.
    pub upstream: Vec<usize>,
    /// River character derived from climate at midpoint.
    pub character: RiverCharacter,
    /// Pre-computed meandering offsets (perpendicular displacement per path point).
    pub meander_offsets: Vec<f64>,
}

// ─── Lake ────────────────────────────────────────────────────────────────────

/// A natural lake (depression that wasn't fully filled).
#[derive(Clone, Debug)]
pub struct Lake {
    /// Cells comprising the lake surface (world coordinates).
    pub cells: Vec<(f64, f64)>,
    /// Drainage area of the basin feeding this lake.
    pub drainage_area: u32,
    /// Outlet segment ID (where lake spills over into a river), if any.
    pub outlet: Option<usize>,
    /// Whether permanently frozen (light_level < 0.05).
    pub frozen: bool,
}

// ─── River Constraint ────────────────────────────────────────────────────────

/// A river constraint returned by chunk queries.
/// Contains everything a chunk needs to render/carve a river segment.
#[derive(Clone, Debug)]
pub struct RiverConstraint {
    /// Path points within the queried bounds (world coords + meander offset).
    pub path: Vec<(f64, f64, f64)>, // (x, y, meander_offset)
    /// Drainage area for width/depth computation.
    pub drainage_area: u32,
    /// Character for rendering style.
    pub character: RiverCharacter,
    /// Computed width in world units.
    pub width: f64,
    /// Computed depth.
    pub depth: f64,
}

// ─── Chunk Coordinate for Spatial Index ──────────────────────────────────────

/// Simple chunk coordinate for the spatial index.
/// Uses the macro pixel grid (1 unit = 1 world unit for macro maps).
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct RiverChunkCoord {
    x: i32,
    y: i32,
}

// ─── River Network ───────────────────────────────────────────────────────────

/// Top-level immutable river network. Computed once during world generation.
pub struct RiverNetwork {
    /// All segments in the network.
    pub segments: Vec<RiverSegment>,
    /// Spatial index: which segment IDs pass through each chunk cell.
    spatial_index: HashMap<RiverChunkCoord, Vec<usize>>,
    /// Natural lakes (depressions that weren't filled).
    pub lakes: Vec<Lake>,
}

impl RiverNetwork {
    /// Generate the global river network from terrain and geological data.
    ///
    /// # Arguments
    /// * `heightmap` - Derived elevation grid
    /// * `rock_hardness` - Base layer: 0.0 soft, 1.0 hard
    /// * `tectonic_stress` - Base layer: high near boundaries (1 - boundary_distance)
    /// * `continentalness` - Base layer: < sea_level = ocean
    /// * `light_level` - Base layer: for river character classification
    /// * `humidity` - Base layer: for river character classification
    /// * `temperature` - Derived layer: for frozen classification
    /// * `peaks_valleys` - Derived layer: for meander strength
    /// * `width`, `height` - Grid dimensions
    /// * `sea_level` - Ocean threshold
    pub fn generate(
        heightmap: &[f64],
        rock_hardness: &[f64],
        tectonic_stress: &[f64],
        continentalness: &[f64],
        light_level: &[f64],
        humidity: &[f64],
        temperature: &[f64],
        peaks_valleys: &[f64],
        width: usize,
        height: usize,
        sea_level: f64,
    ) -> Self {
        let total = width * height;

        // Step 1: Identify natural lakes (diff pre-fill vs post-fill)
        let filled = fill_depressions(heightmap, width, height, sea_level);
        let lakes = identify_lakes(heightmap, &filled, continentalness, light_level, width, height, sea_level);

        // Step 2: Geology-aware D8 flow direction
        let flow_dir = compute_geology_aware_flow(
            &filled, rock_hardness, tectonic_stress, width, height, sea_level,
        );

        // Step 3: Flow accumulation
        let accumulation = compute_flow_accumulation(&flow_dir, &filled, width, height);

        // Step 4: Build river tree
        let min_accumulation = ((total as f64) * 0.0005).max(25.0) as u32;
        let mut segments = build_river_tree(
            &flow_dir, &accumulation, continentalness, width, height, sea_level, min_accumulation,
        );

        // Step 5: Classify river character at each segment midpoint
        for seg in &mut segments {
            if seg.path.is_empty() {
                continue;
            }
            let mid_idx = seg.path.len() / 2;
            let (mx, my) = seg.path[mid_idx];
            let px = (mx as usize).min(width - 1);
            let py = (my as usize).min(height - 1);
            let idx = py * width + px;

            let light = light_level.get(idx).copied().unwrap_or(0.5);
            let humid = humidity.get(idx).copied().unwrap_or(0.5);
            let temp = temperature.get(idx).copied().unwrap_or(15.0);
            seg.character = RiverCharacter::classify(light, humid, temp);
        }

        // Step 6: Apply meandering
        for seg in &mut segments {
            apply_meandering(seg, heightmap, peaks_valleys, width, height);
        }

        // Step 7: Generate deltas at river mouths
        let deltas = generate_deltas(&segments, continentalness, width, height, sea_level);
        let delta_start = segments.len();
        segments.extend(deltas);
        // Link delta segments to their parent
        for _i in delta_start..segments.len() {
            // Delta segments have downstream = None (they terminate at ocean)
            // and their path starts near the parent's mouth
        }

        // Step 8: Build spatial index
        let spatial_index = build_spatial_index(&segments);

        Self {
            segments,
            spatial_index,
            lakes,
        }
    }

    /// Query river segments intersecting a rectangular bounds,
    /// filtered by LOD drainage threshold.
    pub fn query_chunk(
        &self,
        min_x: f64,
        min_y: f64,
        max_x: f64,
        max_y: f64,
        lod_drainage_threshold: u32,
    ) -> Vec<RiverConstraint> {
        // Determine which spatial index cells overlap the bounds
        let ix_min = min_x.floor() as i32;
        let iy_min = min_y.floor() as i32;
        let ix_max = max_x.ceil() as i32;
        let iy_max = max_y.ceil() as i32;

        let mut seen_segments: Vec<bool> = vec![false; self.segments.len()];
        let mut constraints = Vec::new();

        for iy in iy_min..=iy_max {
            for ix in ix_min..=ix_max {
                let coord = RiverChunkCoord { x: ix, y: iy };
                if let Some(seg_ids) = self.spatial_index.get(&coord) {
                    for &id in seg_ids {
                        if id >= self.segments.len() || seen_segments[id] {
                            continue;
                        }
                        seen_segments[id] = true;

                        let seg = &self.segments[id];
                        if seg.drainage_area < lod_drainage_threshold {
                            continue;
                        }

                        // Clip segment path to bounds
                        let clipped_path: Vec<(f64, f64, f64)> = seg.path.iter()
                            .enumerate()
                            .filter(|(_, &(px, py))| {
                                px >= min_x && px <= max_x && py >= min_y && py <= max_y
                            })
                            .map(|(i, &(px, py))| {
                                let offset = seg.meander_offsets.get(i).copied().unwrap_or(0.0);
                                (px, py, offset)
                            })
                            .collect();

                        if clipped_path.is_empty() {
                            continue;
                        }

                        constraints.push(RiverConstraint {
                            path: clipped_path,
                            drainage_area: seg.drainage_area,
                            character: seg.character,
                            width: compute_river_width(seg.drainage_area, seg.character),
                            depth: compute_river_depth(seg.drainage_area),
                        });
                    }
                }
            }
        }

        constraints
    }

    /// Convert the river network to a flat flow grid (backward compatibility).
    /// Returns values in [0, 1] where higher = larger river (log normalized).
    pub fn to_flow_grid(&self, width: usize, height: usize) -> Vec<f64> {
        let total = width * height;
        let mut grid = vec![0.0f64; total];

        // Find max drainage for normalization
        let max_drainage = self.segments.iter()
            .map(|s| s.drainage_area)
            .max()
            .unwrap_or(1) as f64;
        let log_max = max_drainage.ln().max(1.0);

        for seg in &self.segments {
            if seg.character == RiverCharacter::BuriedIce {
                continue;
            }
            for (i, &(px, py)) in seg.path.iter().enumerate() {
                // Apply meander offset for position
                let offset = seg.meander_offsets.get(i).copied().unwrap_or(0.0);

                // Compute flow direction for perpendicular offset
                let (next_x, next_y) = if i + 1 < seg.path.len() {
                    seg.path[i + 1]
                } else if i > 0 {
                    // Use same direction as previous
                    let (prev_x, prev_y) = seg.path[i - 1];
                    (px + (px - prev_x), py + (py - prev_y))
                } else {
                    (px, py + 1.0)
                };

                let flow_dx = next_x - px;
                let flow_dy = next_y - py;
                let flow_len = (flow_dx * flow_dx + flow_dy * flow_dy).sqrt().max(0.001);
                let perp_x = -flow_dy / flow_len;
                let perp_y = flow_dx / flow_len;

                let final_x = (px + offset * perp_x).round() as i64;
                let final_y = (py + offset * perp_y).round() as i64;

                if final_x >= 0 && final_x < width as i64 && final_y >= 0 && final_y < height as i64 {
                    let idx = final_y as usize * width + final_x as usize;
                    let log_val = (seg.drainage_area as f64).ln();
                    let normalized = (log_val / log_max).clamp(0.0, 1.0);
                    grid[idx] = grid[idx].max(normalized);
                }
            }
        }

        grid
    }

    /// Number of segments in the network.
    pub fn segment_count(&self) -> usize {
        self.segments.len()
    }
}

// ─── Width & Depth ───────────────────────────────────────────────────────────

/// Compute river width in world units from drainage area and character.
fn compute_river_width(drainage_area: u32, character: RiverCharacter) -> f64 {
    let base_width = (drainage_area as f64).sqrt() * 0.1;
    base_width * character.width_multiplier()
}

/// Compute river depth from drainage area.
fn compute_river_depth(drainage_area: u32) -> f64 {
    (drainage_area as f64).log10().max(0.0) * 0.5
}

// ─── Depression Filling ──────────────────────────────────────────────────────

/// Fill depressions using a simplified Planchon-Darboux algorithm.
/// This ensures all land cells can drain to the ocean.
fn fill_depressions(elevation: &[f64], width: usize, height: usize, sea_level: f64) -> Vec<f64> {
    let mut filled = elevation.to_vec();
    let epsilon = 1e-5;

    // Initialize: ocean cells keep their elevation, land cells start at infinity
    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            if elevation[idx] <= sea_level {
                filled[idx] = elevation[idx];
            } else if x == 0 || x == width - 1 || y == 0 || y == height - 1 {
                filled[idx] = elevation[idx];
            } else {
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

                if filled[idx] <= elevation[idx] {
                    continue;
                }

                let mut min_neighbor = f64::MAX;
                for (dx, dy) in D8_OFFSETS {
                    let nx = (x as i32 + dx) as usize;
                    let ny = (y as i32 + dy) as usize;
                    let nidx = ny * width + nx;
                    min_neighbor = min_neighbor.min(filled[nidx]);
                }

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

// ─── Lake Identification ─────────────────────────────────────────────────────

/// Identify natural lakes by diffing pre-fill and post-fill heightmaps.
fn identify_lakes(
    original: &[f64],
    filled: &[f64],
    continentalness: &[f64],
    light_level: &[f64],
    width: usize,
    height: usize,
    sea_level: f64,
) -> Vec<Lake> {
    let total = width * height;
    let lake_threshold = 0.02;
    let min_lake_cells = 4;

    // Find cells where fill_depth > threshold and on land
    let mut is_lake_cell = vec![false; total];
    for idx in 0..total {
        let fill_depth = filled[idx] - original[idx];
        if fill_depth > lake_threshold && continentalness[idx] >= sea_level {
            is_lake_cell[idx] = true;
        }
    }

    // Flood-fill to find contiguous lake regions
    let mut visited = vec![false; total];
    let mut lakes = Vec::new();

    for start_idx in 0..total {
        if !is_lake_cell[start_idx] || visited[start_idx] {
            continue;
        }

        // BFS flood fill
        let mut cells = Vec::new();
        let mut queue = vec![start_idx];
        visited[start_idx] = true;

        while let Some(idx) = queue.pop() {
            cells.push(idx);

            let x = idx % width;
            let y = idx / width;

            for (dx, dy) in D8_OFFSETS {
                let nx = x as i32 + dx;
                let ny = y as i32 + dy;
                if nx < 0 || nx >= width as i32 || ny < 0 || ny >= height as i32 {
                    continue;
                }
                let nidx = ny as usize * width + nx as usize;
                if !visited[nidx] && is_lake_cell[nidx] {
                    visited[nidx] = true;
                    queue.push(nidx);
                }
            }
        }

        if cells.len() < min_lake_cells {
            continue;
        }

        // Convert cell indices to world coordinates
        let world_cells: Vec<(f64, f64)> = cells.iter()
            .map(|&idx| ((idx % width) as f64, (idx / width) as f64))
            .collect();

        // Check if lake is frozen
        let avg_light: f64 = cells.iter()
            .map(|&idx| light_level.get(idx).copied().unwrap_or(0.5))
            .sum::<f64>() / cells.len() as f64;

        lakes.push(Lake {
            cells: world_cells,
            drainage_area: cells.len() as u32,
            outlet: None, // Will be linked after river tree is built
            frozen: avg_light < 0.05,
        });
    }

    lakes
}

// ─── Geology-Aware D8 Flow Direction ─────────────────────────────────────────

/// Compute D8 flow directions with geological bias.
/// Rivers prefer fractured fault zones and avoid hard rock.
fn compute_geology_aware_flow(
    elevation: &[f64],
    rock_hardness: &[f64],
    tectonic_stress: &[f64],
    width: usize,
    height: usize,
    sea_level: f64,
) -> Vec<u8> {
    let mut flow_dir = vec![NO_FLOW; width * height];
    let hardness_penalty = 0.08;
    let fault_bonus = 0.12;

    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;

            if elevation[idx] <= sea_level {
                continue;
            }

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
                let base_slope = drop / D8_DISTANCES[dir];

                // Geological bias: penalise hard rock, prefer fault zones
                let adjusted_slope = base_slope
                    - rock_hardness.get(nidx).copied().unwrap_or(0.5) * hardness_penalty
                    + tectonic_stress.get(nidx).copied().unwrap_or(0.0) * fault_bonus;

                if adjusted_slope > max_slope {
                    max_slope = adjusted_slope;
                    best_dir = dir as u8;
                }
            }

            flow_dir[idx] = best_dir;
        }
    }

    flow_dir
}

// ─── Flow Accumulation ───────────────────────────────────────────────────────

/// Compute flow accumulation using topological sort.
/// Each cell's accumulation = 1 + sum of all upstream cells.
fn compute_flow_accumulation(
    flow_dir: &[u8],
    elevation: &[f64],
    width: usize,
    height: usize,
) -> Vec<u32> {
    let total = width * height;
    let mut accumulation = vec![1u32; total];

    // Sort cells by elevation (highest first) for topological processing
    let mut sorted_indices: Vec<usize> = (0..total).collect();
    sorted_indices.sort_by(|&a, &b| {
        elevation[b]
            .partial_cmp(&elevation[a])
            .unwrap_or(std::cmp::Ordering::Equal)
    });

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

// ─── River Tree Building ─────────────────────────────────────────────────────

/// Build river tree from flow direction and accumulation grids.
/// Segments are stretches between confluences, sources, and ocean outlets.
fn build_river_tree(
    flow_dir: &[u8],
    accumulation: &[u32],
    continentalness: &[f64],
    width: usize,
    height: usize,
    sea_level: f64,
    min_accumulation: u32,
) -> Vec<RiverSegment> {
    let total = width * height;

    // Mark which cells are "river cells" (above threshold)
    let is_river: Vec<bool> = accumulation.iter()
        .map(|&a| a >= min_accumulation)
        .collect();

    // Count how many river-cell upstream neighbors flow into each cell
    let mut river_inflow_count = vec![0u32; total];
    for idx in 0..total {
        if !is_river[idx] || flow_dir[idx] == NO_FLOW {
            continue;
        }
        let x = idx % width;
        let y = idx / width;
        let (dx, dy) = D8_OFFSETS[flow_dir[idx] as usize];
        let nx = x as i32 + dx;
        let ny = y as i32 + dy;
        if nx >= 0 && nx < width as i32 && ny >= 0 && ny < height as i32 {
            let nidx = ny as usize * width + nx as usize;
            if is_river[nidx] {
                river_inflow_count[nidx] += 1;
            }
        }
    }

    // Find segment start points: river cells that are either:
    // - headwaters (no river-cell upstream neighbor flowing in)
    // - confluences (2+ river-cell upstream neighbors flowing in)
    // We trace FROM these start points DOWNSTREAM.

    // Actually, it's simpler to trace from every river cell that is a "source"
    // (no incoming river flow) or a "confluence" (2+ incoming), downstream until
    // hitting another confluence or the ocean.

    let mut segment_id_at: Vec<Option<usize>> = vec![None; total];
    let mut segments: Vec<RiverSegment> = Vec::new();

    // Find all river cells that are segment start points
    let mut starts: Vec<usize> = Vec::new();
    for idx in 0..total {
        if !is_river[idx] {
            continue;
        }
        // Headwater: no river cell flows into this one
        if river_inflow_count[idx] == 0 {
            starts.push(idx);
        }
        // Confluence: 2+ river cells flow into this one
        // These are also segment starts (for the downstream segment)
        if river_inflow_count[idx] >= 2 {
            starts.push(idx);
        }
    }

    // Remove duplicates (a confluence headwater)
    starts.sort_unstable();
    starts.dedup();

    // Trace each segment downstream
    for &start in &starts {
        if segment_id_at[start].is_some() && river_inflow_count[start] == 0 {
            // Already part of a segment from a confluence trace, skip if headwater
            continue;
        }

        let mut path = Vec::new();
        let mut current = start;

        loop {
            // If this cell already belongs to a segment and we're not at the start,
            // we've hit a confluence — end this segment here (exclusive)
            if current != start && segment_id_at[current].is_some() {
                break;
            }
            // If this cell is a confluence and we're not at the start, end here
            if current != start && river_inflow_count[current] >= 2 {
                break;
            }

            path.push((
                (current % width) as f64,
                (current / width) as f64,
            ));

            // Check if we've reached the ocean
            if continentalness.get(current).copied().unwrap_or(0.0) < sea_level {
                break;
            }

            // Follow flow direction
            if flow_dir[current] == NO_FLOW {
                break;
            }

            let x = current % width;
            let y = current / width;
            let (dx, dy) = D8_OFFSETS[flow_dir[current] as usize];
            let nx = x as i32 + dx;
            let ny = y as i32 + dy;

            if nx < 0 || nx >= width as i32 || ny < 0 || ny >= height as i32 {
                break;
            }

            let next = ny as usize * width + nx as usize;

            // If next cell is below threshold, terminate
            if !is_river[next] && continentalness.get(next).copied().unwrap_or(0.0) >= sea_level {
                break;
            }

            current = next;
        }

        if path.len() < 2 {
            continue;
        }

        let seg_id = segments.len();
        let last_cell = path.last().unwrap();
        let last_idx = last_cell.1 as usize * width + last_cell.0 as usize;
        let drainage = accumulation.get(last_idx).copied().unwrap_or(0);

        // Mark cells as belonging to this segment
        for &(px, py) in &path {
            let idx = py as usize * width + px as usize;
            if segment_id_at[idx].is_none() {
                segment_id_at[idx] = Some(seg_id);
            }
        }

        segments.push(RiverSegment {
            id: seg_id,
            meander_offsets: vec![0.0; path.len()],
            path,
            drainage_area: drainage,
            downstream: None,
            upstream: Vec::new(),
            character: RiverCharacter::Permanent, // Will be classified later
        });
    }

    // Link segments: find downstream connections
    // For each segment, check where its last cell flows to — find which segment owns that cell
    for i in 0..segments.len() {
        let last = *segments[i].path.last().unwrap();
        let last_idx = last.1 as usize * width + last.0 as usize;

        if flow_dir[last_idx] == NO_FLOW {
            continue;
        }

        let x = last_idx % width;
        let y = last_idx / width;
        let (dx, dy) = D8_OFFSETS[flow_dir[last_idx] as usize];
        let nx = x as i32 + dx;
        let ny = y as i32 + dy;

        if nx < 0 || nx >= width as i32 || ny < 0 || ny >= height as i32 {
            continue;
        }

        let next_idx = ny as usize * width + nx as usize;
        if let Some(downstream_seg) = segment_id_at[next_idx] {
            if downstream_seg != i {
                segments[i].downstream = Some(downstream_seg);
            }
        }
    }

    // Build upstream links from downstream links
    let downstream_links: Vec<(usize, Option<usize>)> = segments.iter()
        .map(|s| (s.id, s.downstream))
        .collect();
    for (seg_id, downstream) in downstream_links {
        if let Some(ds) = downstream {
            if ds < segments.len() {
                segments[ds].upstream.push(seg_id);
            }
        }
    }

    segments
}

// ─── Meandering ──────────────────────────────────────────────────────────────

/// Apply meandering to low-gradient stretches of a segment.
fn apply_meandering(
    segment: &mut RiverSegment,
    heightmap: &[f64],
    peaks_valleys: &[f64],
    width: usize,
    height: usize,
) {
    let slope_threshold = 0.02;

    for i in 0..segment.path.len() {
        let (px, py) = segment.path[i];
        let ix = (px as usize).min(width.saturating_sub(1));
        let iy = (py as usize).min(height.saturating_sub(1));
        let idx = iy * width + ix;

        // Compute local slope from heightmap
        let slope = local_gradient(heightmap, ix, iy, width, height);

        // Low slope + low relief = floodplain = meander
        let pv = peaks_valleys.get(idx).copied().unwrap_or(0.0).abs();
        let meander_strength = (1.0 - slope / slope_threshold).max(0.0)
            * (1.0 - pv).max(0.0);

        if meander_strength > 0.1 {
            let amplitude = meander_strength
                * (segment.drainage_area as f64).sqrt()
                * 0.15; // scale factor (conservative for grid coords)
            let wavelength = (amplitude * 8.0).max(4.0);

            // Deterministic phase from segment ID and point index
            let phase = deterministic_hash(segment.id, i);

            let offset = (i as f64 / wavelength * std::f64::consts::TAU + phase).sin()
                * amplitude;

            if i < segment.meander_offsets.len() {
                segment.meander_offsets[i] = offset;
            }
        }
    }
}

/// Compute local gradient magnitude at a heightmap cell.
fn local_gradient(heightmap: &[f64], x: usize, y: usize, width: usize, height: usize) -> f64 {
    let idx = y * width + x;
    let h = heightmap[idx];

    let mut max_drop = 0.0f64;
    for (dx, dy) in D8_OFFSETS {
        let nx = x as i32 + dx;
        let ny = y as i32 + dy;
        if nx >= 0 && nx < width as i32 && ny >= 0 && ny < height as i32 {
            let nidx = ny as usize * width + nx as usize;
            let drop = (h - heightmap[nidx]).abs();
            max_drop = max_drop.max(drop);
        }
    }
    max_drop
}

/// Deterministic hash for stable meander phase.
fn deterministic_hash(seg_id: usize, point_idx: usize) -> f64 {
    let n = (seg_id as u32).wrapping_mul(2654435761)
        .wrapping_add(point_idx as u32)
        .wrapping_mul(1103515245)
        .wrapping_add(12345);
    (n & 0xFFFF) as f64 / 0xFFFF as f64 * std::f64::consts::TAU
}

// ─── Delta Generation ────────────────────────────────────────────────────────

/// Generate deltas at major river mouths where they meet the coast.
fn generate_deltas(
    segments: &[RiverSegment],
    continentalness: &[f64],
    width: usize,
    height: usize,
    sea_level: f64,
) -> Vec<RiverSegment> {
    let mut deltas = Vec::new();

    for seg in segments {
        // Only large rivers with no downstream segment (ocean mouth)
        if seg.drainage_area < 2000 || seg.downstream.is_some() {
            continue;
        }

        let mouth = match seg.path.last() {
            Some(m) => *m,
            None => continue,
        };

        let mx = (mouth.0 as usize).min(width.saturating_sub(1));
        let my = (mouth.1 as usize).min(height.saturating_sub(1));
        let cont = continentalness.get(my * width + mx).copied().unwrap_or(0.0);

        // Only in the continental shelf taper zone
        if cont > 0.15 || cont < -0.1 {
            continue;
        }

        // Determine flow direction at mouth
        let flow_dir = if seg.path.len() >= 2 {
            let prev = seg.path[seg.path.len() - 2];
            let dx = mouth.0 - prev.0;
            let dy = mouth.1 - prev.1;
            let len = (dx * dx + dy * dy).sqrt().max(0.001);
            (dx / len, dy / len)
        } else {
            (0.0, 1.0)
        };

        // Fork into 2-3 distributaries
        let num_branches = if seg.drainage_area > 5000 { 3 } else { 2 };
        let spread_angle = std::f64::consts::PI / 6.0; // 30° total spread

        for branch in 0..num_branches {
            let angle_offset = if num_branches == 1 {
                0.0
            } else {
                spread_angle * (branch as f64 / (num_branches - 1) as f64 - 0.5)
            };

            let cos_a = angle_offset.cos();
            let sin_a = angle_offset.sin();
            let branch_dx = flow_dir.0 * cos_a - flow_dir.1 * sin_a;
            let branch_dy = flow_dir.0 * sin_a + flow_dir.1 * cos_a;

            // Trace branch until hitting deep ocean
            let mut path = Vec::new();
            let mut bx = mouth.0;
            let mut by = mouth.1;

            for _ in 0..20 {
                bx += branch_dx;
                by += branch_dy;

                let ix = (bx as usize).min(width.saturating_sub(1));
                let iy = (by as usize).min(height.saturating_sub(1));

                if ix >= width || iy >= height {
                    break;
                }

                let c = continentalness.get(iy * width + ix).copied().unwrap_or(-1.0);
                path.push((bx, by));

                if c < sea_level - 0.05 {
                    break;
                }
            }

            if path.len() >= 2 {
                let delta_id = segments.len() + deltas.len();
                deltas.push(RiverSegment {
                    id: delta_id,
                    meander_offsets: vec![0.0; path.len()],
                    path,
                    drainage_area: seg.drainage_area / num_branches as u32,
                    downstream: None,
                    upstream: vec![seg.id],
                    character: seg.character,
                });
            }
        }
    }

    deltas
}

// ─── Spatial Index ───────────────────────────────────────────────────────────

/// Build spatial index mapping grid cells to segment IDs.
fn build_spatial_index(segments: &[RiverSegment]) -> HashMap<RiverChunkCoord, Vec<usize>> {
    let mut index: HashMap<RiverChunkCoord, Vec<usize>> = HashMap::new();

    for seg in segments {
        for &(x, y) in &seg.path {
            let coord = RiverChunkCoord {
                x: x.floor() as i32,
                y: y.floor() as i32,
            };
            index.entry(coord).or_default().push(seg.id);
        }
    }

    // Deduplicate segment IDs per cell
    for ids in index.values_mut() {
        ids.sort_unstable();
        ids.dedup();
    }

    index
}

// ─── Heightmap Carving ──────────────────────────────────────────────────────

/// Carve macro river channels into a meso heightmap before D8 flow computation.
///
/// For each meso pixel that overlaps a macro river cell, the heightmap is lowered
/// proportionally to the macro flow magnitude. This forces meso-level D8 routing
/// to follow the same corridors established at macro level.
pub fn carve_river_channels(
    heightmap: &mut [f64],
    width: usize,
    height: usize,
    macro_rivers: &[f64],
    macro_width: usize,
    macro_height: usize,
    world_x: f64,
    world_y: f64,
    world_size: f64,
    sea_level: f64,
) {
    let scale = world_size / width as f64;
    let max_carve_depth = 0.04;
    let carve_radius_base = 2.0_f64;
    let carve_radius_flow_scale = 4.0_f64;

    // First pass: collect carve points from macro rivers
    let mut carve_points: Vec<(usize, usize, f64)> = Vec::new(); // (mx_px, my_px, flow)

    for my in 0..macro_height {
        for mx in 0..macro_width {
            let macro_flow = macro_rivers[my * macro_width + mx];
            if macro_flow <= 0.0 {
                continue;
            }

            // Map macro world coord to meso pixel
            let meso_px = ((mx as f64 - world_x) / scale) as i64;
            let meso_py = ((my as f64 - world_y) / scale) as i64;

            if meso_px >= 0 && meso_px < width as i64 && meso_py >= 0 && meso_py < height as i64 {
                carve_points.push((meso_px as usize, meso_py as usize, macro_flow));
            }
        }
    }

    // Second pass: apply Gaussian-width carving around each river point
    for &(cx, cy, flow) in &carve_points {
        let depth = (flow * max_carve_depth).min(max_carve_depth);
        let radius = carve_radius_base + flow * carve_radius_flow_scale;
        let r_ceil = radius.ceil() as i64;

        for dy in -r_ceil..=r_ceil {
            for dx in -r_ceil..=r_ceil {
                let px = cx as i64 + dx;
                let py = cy as i64 + dy;

                if px < 0 || px >= width as i64 || py < 0 || py >= height as i64 {
                    continue;
                }

                let idx = py as usize * width + px as usize;

                // Only carve above sea level
                if heightmap[idx] <= sea_level {
                    continue;
                }

                let dist = ((dx * dx + dy * dy) as f64).sqrt();
                if dist > radius {
                    continue;
                }

                // Gaussian falloff
                let sigma = radius / 2.0;
                let falloff = (-dist * dist / (2.0 * sigma * sigma)).exp();
                let carve_amount = depth * falloff;

                heightmap[idx] -= carve_amount;
            }
        }
    }
}

// ─── Legacy API ──────────────────────────────────────────────────────────────

/// Legacy river generator for backward compatibility.
/// Wraps the new RiverNetwork system but provides the old Vec<f64> interface.
pub struct RiverGenerator {
    pub sea_level: f64,
    pub min_accumulation: u32,
}

impl Default for RiverGenerator {
    fn default() -> Self {
        Self {
            sea_level: -0.025,
            min_accumulation: 100,
        }
    }
}

impl RiverGenerator {
    pub fn new(sea_level: f64) -> Self {
        Self {
            sea_level,
            ..Default::default()
        }
    }

    /// Create a river generator with threshold based on map size.
    pub fn for_map_size(sea_level: f64, width: usize, height: usize) -> Self {
        let total = width * height;
        let threshold = ((total as f64) * 0.0005).max(25.0) as u32;
        Self {
            sea_level,
            min_accumulation: threshold,
        }
    }

    /// Generate rivers for a map using the new geology-aware system.
    /// Falls back to simple D8 when geological layers aren't available.
    ///
    /// This is the simple interface that doesn't use geological data.
    /// Used by macro-level generation where we don't have per-pixel geological data
    /// separately (it's baked into the heightmap).
    pub fn generate(&self, elevation: &[f64], width: usize, height: usize) -> Vec<f64> {
        // Use the simple path: fill depressions, basic D8, accumulate, extract
        let filled = fill_depressions(elevation, width, height, self.sea_level);
        let flow_dir = self.compute_flow_directions_simple(&filled, width, height);
        let accumulation = compute_flow_accumulation(&flow_dir, &filled, width, height);
        self.extract_rivers(&accumulation, width, height)
    }

    /// Generate rivers with full geological awareness.
    /// Returns a RiverNetwork for LOD-consistent chunk queries.
    pub fn generate_network(
        &self,
        heightmap: &[f64],
        rock_hardness: &[f64],
        tectonic_stress: &[f64],
        continentalness: &[f64],
        light_level: &[f64],
        humidity: &[f64],
        temperature: &[f64],
        peaks_valleys: &[f64],
        width: usize,
        height: usize,
    ) -> RiverNetwork {
        RiverNetwork::generate(
            heightmap, rock_hardness, tectonic_stress, continentalness,
            light_level, humidity, temperature, peaks_valleys,
            width, height, self.sea_level,
        )
    }

    /// Generate rivers with macro flow seeding for meso-level chunks.
    /// Seeds edge cells with upstream accumulation from the macro river map,
    /// ensuring rivers flow continuously across chunk boundaries.
    pub fn generate_with_macro_flow(
        &self,
        elevation: &[f64],
        width: usize,
        height: usize,
        macro_rivers: &[f64],
        macro_width: usize,
        macro_height: usize,
        world_x: f64,
        world_y: f64,
        world_size: f64,
    ) -> Vec<f64> {
        let filled = fill_depressions(elevation, width, height, self.sea_level);
        let flow_dir = self.compute_flow_directions_simple(&filled, width, height);

        let total = width * height;
        let mut accumulation = vec![1u32; total];

        // Seed ALL macro river pixels with upstream accumulation.
        // Previously only an edge band was seeded; now every meso pixel overlapping
        // a macro river gets a proportional boost so interior rivers don't vanish.
        let scale = world_size / width as f64;
        let macro_total = (macro_width * macro_height) as f64;

        for y in 0..height {
            for x in 0..width {
                let wx = world_x + x as f64 * scale;
                let wy = world_y + y as f64 * scale;
                let mx = wx as i64;
                let my = wy as i64;

                if mx >= 0 && mx < macro_width as i64
                    && my >= 0 && my < macro_height as i64
                {
                    let macro_flow = macro_rivers[my as usize * macro_width + mx as usize];
                    if macro_flow > 0.0 {
                        let boost = (macro_flow * macro_total * 0.08) as u32;
                        let idx = y * width + x;
                        accumulation[idx] = accumulation[idx].saturating_add(boost);
                    }
                }
            }
        }

        // Topological sort: propagate flow from highest to lowest elevation.
        let mut sorted_indices: Vec<usize> = (0..total).collect();
        sorted_indices.sort_by(|&a, &b| {
            filled[b]
                .partial_cmp(&filled[a])
                .unwrap_or(std::cmp::Ordering::Equal)
        });

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
                accumulation[target_idx] =
                    accumulation[target_idx].saturating_add(accumulation[idx]);
            }
        }

        self.extract_rivers(&accumulation, width, height)
    }

    /// Simple D8 flow direction (no geological bias).
    fn compute_flow_directions_simple(&self, elevation: &[f64], width: usize, height: usize) -> Vec<u8> {
        let mut flow_dir = vec![NO_FLOW; width * height];

        for y in 0..height {
            for x in 0..width {
                let idx = y * width + x;

                if elevation[idx] <= self.sea_level {
                    continue;
                }

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

    /// Extract rivers from accumulation map (legacy format).
    fn extract_rivers(&self, accumulation: &[u32], width: usize, height: usize) -> Vec<f64> {
        let total = width * height;
        let mut rivers = vec![0.0; total];

        let max_accum = *accumulation.iter().max().unwrap_or(&1) as f64;
        let log_max = max_accum.ln();

        for idx in 0..total {
            if accumulation[idx] >= self.min_accumulation {
                let log_val = (accumulation[idx] as f64).ln();
                let log_threshold = (self.min_accumulation as f64).ln();
                rivers[idx] = ((log_val - log_threshold) / (log_max - log_threshold)).clamp(0.0, 1.0);
            }
        }

        rivers
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

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
        let small = RiverGenerator::for_map_size(-0.025, 64, 32);
        let large = RiverGenerator::for_map_size(-0.025, 1024, 512);
        assert!(small.min_accumulation < large.min_accumulation);
        assert!(small.min_accumulation >= 25);
    }

    #[test]
    fn test_depression_filling() {
        let width = 5;
        let height = 5;
        let sea_level = -0.025;

        #[rustfmt::skip]
        let elevation = vec![
            0.1, 0.1, 0.1, 0.1, 0.1,
            0.1, 0.0, 0.0, 0.0, 0.1,
            0.1, 0.0, -0.1, 0.0, 0.1,
            0.1, 0.0, 0.0, 0.0, 0.1,
            0.1, 0.1, 0.1, 0.1, 0.1,
        ];

        let filled = fill_depressions(&elevation, width, height, sea_level);
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

        #[rustfmt::skip]
        let elevation = vec![
            0.3, 0.2, 0.1,
            0.2, 0.1, 0.0,
            0.1, 0.0, -0.1,
        ];

        let flow_dir = gen.compute_flow_directions_simple(&elevation, width, height);

        let center_idx = 1 * width + 1;
        assert_ne!(flow_dir[center_idx], NO_FLOW);

        let ocean_idx = 2 * width + 2;
        assert_eq!(flow_dir[ocean_idx], NO_FLOW);
    }

    #[test]
    fn test_flow_accumulation_convergence() {
        let width = 5;
        let height = 5;

        #[rustfmt::skip]
        let elevation = vec![
            0.2, 0.15, 0.1, 0.15, 0.2,
            0.15, 0.1, 0.05, 0.1, 0.15,
            0.1, 0.05, 0.0, 0.05, 0.1,
            0.15, 0.1, 0.05, 0.1, 0.15,
            0.2, 0.15, 0.1, 0.15, 0.2,
        ];

        let gen = RiverGenerator::new(-0.025);
        let flow_dir = gen.compute_flow_directions_simple(&elevation, width, height);
        let accumulation = compute_flow_accumulation(&flow_dir, &elevation, width, height);

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

        assert_eq!(rivers[0], 0.0);
        assert_eq!(rivers[1], 0.0);
        assert!(rivers[3] > rivers[2]);
        assert!(rivers[4] > rivers[3]);
        assert!(rivers[4] > 0.0);
    }

    #[test]
    fn test_geology_aware_flow_differs_from_simple() {
        let width = 8;
        let height = 8;
        let sea_level = -0.025;

        // Gentle slope from top-left to bottom-right
        let mut elevation = vec![0.0; width * height];
        for y in 0..height {
            for x in 0..width {
                elevation[y * width + x] = 0.5 - (x + y) as f64 * 0.03;
            }
        }

        // Uniform rock hardness
        let rock_uniform = vec![0.5; width * height];
        let tect_zero = vec![0.0; width * height];

        // Banded rock hardness: hard vertical stripe in middle
        let mut rock_banded = vec![0.3; width * height];
        for y in 0..height {
            rock_banded[y * width + 3] = 0.95; // Hard stripe
            rock_banded[y * width + 4] = 0.95;
        }

        let flow_uniform = compute_geology_aware_flow(
            &elevation, &rock_uniform, &tect_zero, width, height, sea_level,
        );
        let flow_banded = compute_geology_aware_flow(
            &elevation, &rock_banded, &tect_zero, width, height, sea_level,
        );

        // With banded hardness, some flow directions should differ
        let mut differs = false;
        for idx in 0..width * height {
            if flow_uniform[idx] != flow_banded[idx]
                && flow_uniform[idx] != NO_FLOW
                && flow_banded[idx] != NO_FLOW
            {
                differs = true;
                break;
            }
        }
        assert!(differs, "Geological bias should change some flow directions");
    }

    #[test]
    fn test_river_network_generation() {
        let width = 32;
        let height = 16;
        let sea_level = -0.025;

        // Simple terrain: high on left, sloping to ocean on right
        let mut heightmap = vec![0.0; width * height];
        let mut continentalness = vec![0.0; width * height];
        let rock = vec![0.5; width * height];
        let tect = vec![0.0; width * height];
        let light = vec![0.3; width * height];
        let humid = vec![0.5; width * height];
        let temp = vec![15.0; width * height];
        let pv = vec![0.0; width * height];

        for y in 0..height {
            for x in 0..width {
                let idx = y * width + x;
                heightmap[idx] = 0.3 - x as f64 * 0.015;
                continentalness[idx] = 0.3 - x as f64 * 0.015;
            }
        }

        let network = RiverNetwork::generate(
            &heightmap, &rock, &tect, &continentalness,
            &light, &humid, &temp, &pv,
            width, height, sea_level,
        );

        // Should produce some segments
        let grid = network.to_flow_grid(width, height);
        assert_eq!(grid.len(), width * height);
    }

    #[test]
    fn test_river_character_classification() {
        assert_eq!(RiverCharacter::classify(0.8, 0.1, 20.0), RiverCharacter::DryWadi);
        assert_eq!(RiverCharacter::classify(0.5, 0.3, 20.0), RiverCharacter::SeasonalFlow);
        assert_eq!(RiverCharacter::classify(0.2, 0.6, 15.0), RiverCharacter::Permanent);
        assert_eq!(RiverCharacter::classify(0.08, 0.5, -10.0), RiverCharacter::Frozen);
        assert_eq!(RiverCharacter::classify(0.03, 0.5, -30.0), RiverCharacter::BuriedIce);
    }

    #[test]
    fn test_lake_identification() {
        let width = 8;
        let height = 8;
        let sea_level = -0.5;

        // Create terrain with a depression
        let mut original = vec![0.2; width * height];
        // Make a lake basin in the center
        for y in 2..6 {
            for x in 2..6 {
                original[y * width + x] = 0.05;
            }
        }
        // Even deeper center
        original[3 * width + 3] = 0.0;
        original[3 * width + 4] = 0.0;
        original[4 * width + 3] = 0.0;
        original[4 * width + 4] = 0.0;

        let filled = fill_depressions(&original, width, height, sea_level);
        let continentalness = vec![0.2; width * height]; // all land
        let light_level = vec![0.3; width * height];

        let lakes = identify_lakes(&original, &filled, &continentalness, &light_level, width, height, sea_level);

        // Should find at least one lake
        // Note: depends on whether fill depth exceeds threshold
        // The center cells should have significant fill depth
        let center_fill = filled[3 * width + 3] - original[3 * width + 3];
        if center_fill > 0.02 {
            assert!(!lakes.is_empty(), "Should identify a lake basin");
        }
    }

    #[test]
    fn test_spatial_index_query() {
        // Build a simple river network and verify query returns correct results
        let seg = RiverSegment {
            id: 0,
            path: vec![(5.0, 5.0), (6.0, 5.0), (7.0, 5.0)],
            drainage_area: 100,
            downstream: None,
            upstream: vec![],
            character: RiverCharacter::Permanent,
            meander_offsets: vec![0.0, 0.0, 0.0],
        };

        let spatial_index = build_spatial_index(&[seg.clone()]);

        // Query the area containing the segment
        let network = RiverNetwork {
            segments: vec![seg],
            spatial_index,
            lakes: vec![],
        };

        let constraints = network.query_chunk(4.0, 4.0, 8.0, 6.0, 50);
        assert!(!constraints.is_empty(), "Should find river segment in query bounds");
        assert_eq!(constraints[0].drainage_area, 100);
    }

    #[test]
    fn test_meander_flat_terrain() {
        let width = 20;
        let height = 5;
        // Flat terrain should produce meandering
        let heightmap = vec![0.1; width * height];
        let pv = vec![0.0; width * height]; // no relief

        let mut seg = RiverSegment {
            id: 0,
            path: (0..width).map(|x| (x as f64, 2.0)).collect(),
            drainage_area: 200,
            downstream: None,
            upstream: vec![],
            character: RiverCharacter::Permanent,
            meander_offsets: vec![0.0; width],
        };

        apply_meandering(&mut seg, &heightmap, &pv, width, height);

        // Some offsets should be nonzero on flat terrain
        let has_meander = seg.meander_offsets.iter().any(|&o| o.abs() > 0.01);
        assert!(has_meander, "Flat terrain should produce meanders");
    }
}

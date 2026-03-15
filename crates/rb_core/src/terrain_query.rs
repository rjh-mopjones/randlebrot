use crate::biome::TileType;

/// Read-only terrain query interface at meso pixel resolution (8192×4096).
///
/// The terrain is pre-generated as 128 macro chunks (16×8 grid), each 512×512
/// pixels, stitched into an 8192×4096 grid. All coordinates are in this meso
/// pixel space.
///
/// Coordinate mapping:
///   world_x * 8.0 = meso_x    (world is 1024 wide, meso is 8192)
///   meso_x / 512 = chunk_x    (16 chunks across)
///   meso_x % 512 = local_x    (within 512×512 tile)
pub trait TerrainQuery: Send + Sync {
    /// Width in meso pixels (8192).
    fn width(&self) -> usize;
    /// Height in meso pixels (4096).
    fn height(&self) -> usize;

    // Per-pixel queries at meso resolution.
    // All return 0.0 / TileType default for out-of-bounds coords.
    fn heightmap_at(&self, x: usize, y: usize) -> f64;
    fn biome_at(&self, x: usize, y: usize) -> TileType;
    fn temperature_at(&self, x: usize, y: usize) -> f64;
    fn humidity_at(&self, x: usize, y: usize) -> f64;
    fn continentalness_at(&self, x: usize, y: usize) -> f64;
    fn erosion_at(&self, x: usize, y: usize) -> f64;
    fn light_level_at(&self, x: usize, y: usize) -> f64;
    fn rock_hardness_at(&self, x: usize, y: usize) -> f64;
    fn river_at(&self, x: usize, y: usize) -> f64;
    fn drainage_at(&self, x: usize, y: usize) -> f64;
    fn tectonic_at(&self, x: usize, y: usize) -> f64;
    fn peaks_valleys_at(&self, x: usize, y: usize) -> f64;
    fn aridity_at(&self, x: usize, y: usize) -> f64;
    fn slope_at(&self, x: usize, y: usize) -> f64;

    // Classification queries
    fn is_ocean(&self, x: usize, y: usize) -> bool;
    fn is_river(&self, x: usize, y: usize) -> bool;
}

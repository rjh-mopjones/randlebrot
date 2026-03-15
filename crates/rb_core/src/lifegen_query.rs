/// Read-only query interface for civilisation data at meso pixel resolution (8192×4096).
///
/// Grid queries use meso pixel coordinates. Indexed queries use IDs.
/// Spatial queries use world-space f64 coordinates for SceneGen micro chunks.
pub trait LifeGenQuery: Send + Sync {
    /// Width in meso pixels (8192).
    fn width(&self) -> usize;
    /// Height in meso pixels (4096).
    fn height(&self) -> usize;

    // Grid queries (meso pixel coordinates)
    fn province_at(&self, x: usize, y: usize) -> Option<u16>;
    fn habitability_at(&self, x: usize, y: usize) -> f32;
    fn navigation_cost_at(&self, x: usize, y: usize) -> f32;
    fn is_province_border(&self, x: usize, y: usize) -> bool;
    fn is_faction_border(&self, x: usize, y: usize) -> bool;
}

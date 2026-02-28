/// Trait for noise generation strategies.
///
/// Each strategy generates a specific type of noise (continentalness, temperature, etc.)
/// using fractal Brownian motion (fBm) or other techniques.
///
/// The trait is object-safe and designed for use in the chunk hierarchy system.
pub trait NoiseStrategy: Send + Sync {
    /// Generate a noise value at the given world coordinates and detail level.
    ///
    /// # Arguments
    /// * `x` - World X coordinate (f64 for precision)
    /// * `y` - World Y coordinate (f64 for precision)
    /// * `detail_level` - The detail level (0=Macro, 1=Meso, 2=Micro)
    ///
    /// # Returns
    /// A noise value, typically in the range [-1.0, 1.0] or [0.0, 1.0]
    /// depending on the specific strategy.
    fn generate(&self, x: f64, y: f64, detail_level: u32) -> f64;

    /// Returns the name of this noise strategy for debugging.
    fn name(&self) -> &'static str {
        "NoiseStrategy"
    }
}

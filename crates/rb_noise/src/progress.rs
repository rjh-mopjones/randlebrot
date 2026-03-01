use std::sync::atomic::{AtomicUsize, Ordering};

/// Layer identifiers for progress tracking during parallel generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LayerId {
    Continentalness,
    Temperature,
    Tectonic,
    PeaksValleys,
    Erosion,
    Humidity,
    Resources,
}

impl LayerId {
    /// Returns all layer IDs in order.
    pub fn all() -> &'static [LayerId] {
        &[
            Self::Continentalness,
            Self::Temperature,
            Self::Tectonic,
            Self::PeaksValleys,
            Self::Erosion,
            Self::Humidity,
            Self::Resources,
        ]
    }

    /// Display name for UI.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Continentalness => "Continentalness",
            Self::Temperature => "Temperature",
            Self::Tectonic => "Tectonic",
            Self::PeaksValleys => "Peaks/Valleys",
            Self::Erosion => "Erosion",
            Self::Humidity => "Humidity",
            Self::Resources => "Resources",
        }
    }

    /// Index for array access (0-6).
    pub fn index(&self) -> usize {
        match self {
            Self::Continentalness => 0,
            Self::Temperature => 1,
            Self::Tectonic => 2,
            Self::PeaksValleys => 3,
            Self::Erosion => 4,
            Self::Humidity => 5,
            Self::Resources => 6,
        }
    }
}

/// Thread-safe progress tracker for parallel layer generation.
/// Uses atomic counters for lock-free progress updates from multiple threads.
pub struct LayerProgress {
    counters: [AtomicUsize; 7],
    total_pixels: usize,
}

impl LayerProgress {
    /// Create a new progress tracker for the given total pixel count.
    pub fn new(total_pixels: usize) -> Self {
        Self {
            counters: Default::default(),
            total_pixels,
        }
    }

    /// Increment the counter for a specific layer.
    /// Safe to call from multiple threads concurrently.
    pub fn increment(&self, layer: LayerId, amount: usize) {
        self.counters[layer.index()].fetch_add(amount, Ordering::Relaxed);
    }

    /// Get the current count for a specific layer.
    pub fn get(&self, layer: LayerId) -> usize {
        self.counters[layer.index()].load(Ordering::Relaxed)
    }

    /// Get the progress fraction (0.0 to 1.0) for a specific layer.
    pub fn fraction(&self, layer: LayerId) -> f32 {
        if self.total_pixels == 0 {
            return 0.0;
        }
        self.get(layer) as f32 / self.total_pixels as f32
    }

    /// Get the total pixel count being tracked.
    pub fn total_pixels(&self) -> usize {
        self.total_pixels
    }

    /// Reset all counters to zero.
    pub fn reset(&self) {
        for counter in &self.counters {
            counter.store(0, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_id_indices_are_unique() {
        let mut seen = [false; 7];
        for layer in LayerId::all() {
            let idx = layer.index();
            assert!(!seen[idx], "Duplicate index {} for {:?}", idx, layer);
            seen[idx] = true;
        }
    }

    #[test]
    fn progress_tracking() {
        let progress = LayerProgress::new(100);

        progress.increment(LayerId::Continentalness, 50);
        assert_eq!(progress.get(LayerId::Continentalness), 50);
        assert!((progress.fraction(LayerId::Continentalness) - 0.5).abs() < 0.001);

        progress.increment(LayerId::Continentalness, 50);
        assert_eq!(progress.get(LayerId::Continentalness), 100);
        assert!((progress.fraction(LayerId::Continentalness) - 1.0).abs() < 0.001);
    }

    #[test]
    fn reset_clears_all() {
        let progress = LayerProgress::new(100);
        for layer in LayerId::all() {
            progress.increment(*layer, 42);
        }
        progress.reset();
        for layer in LayerId::all() {
            assert_eq!(progress.get(*layer), 0);
        }
    }
}

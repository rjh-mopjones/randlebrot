use rb_core::ResourceType;
use smallvec::SmallVec;
use std::collections::HashMap;

/// Sparse resource map - only stores cells with resources above threshold.
/// This avoids storing 12 dense vectors for resource types (would be ~50MB at 1024x512).
pub struct ResourceMap {
    pub width: usize,
    pub height: usize,
    /// Map from pixel index (y * width + x) to resource abundances.
    /// Uses SmallVec since most cells have 0-3 resources.
    resources: HashMap<usize, SmallVec<[(ResourceType, f32); 3]>>,
}

impl ResourceMap {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            resources: HashMap::new(),
        }
    }

    /// Set the abundance of a resource at a location.
    /// Values below 0.01 are ignored to save memory.
    pub fn set(&mut self, x: usize, y: usize, resource: ResourceType, abundance: f32) {
        if abundance < 0.01 {
            return; // Don't store negligible amounts
        }

        let idx = y * self.width + x;
        let entry = self.resources.entry(idx).or_insert_with(SmallVec::new);

        // Update existing or add new
        if let Some(existing) = entry.iter_mut().find(|(r, _)| *r == resource) {
            existing.1 = abundance;
        } else {
            entry.push((resource, abundance));
        }
    }

    /// Get the abundance of a specific resource at a location.
    /// Returns 0.0 if no resource is present.
    pub fn get(&self, x: usize, y: usize, resource: ResourceType) -> f32 {
        let idx = y * self.width + x;
        self.resources
            .get(&idx)
            .and_then(|v| v.iter().find(|(r, _)| *r == resource))
            .map(|(_, a)| *a)
            .unwrap_or(0.0)
    }

    /// Get all resources at a location.
    pub fn get_all(&self, x: usize, y: usize) -> &[(ResourceType, f32)] {
        let idx = y * self.width + x;
        self.resources
            .get(&idx)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Check if any resources exist at a location.
    pub fn has_resources(&self, x: usize, y: usize) -> bool {
        let idx = y * self.width + x;
        self.resources.contains_key(&idx)
    }

    /// Get the total number of cells with resources.
    pub fn cells_with_resources(&self) -> usize {
        self.resources.len()
    }

    /// Get all locations with a specific resource type.
    pub fn locations_with_resource(&self, resource: ResourceType) -> Vec<(usize, usize, f32)> {
        self.resources
            .iter()
            .filter_map(|(idx, resources)| {
                resources
                    .iter()
                    .find(|(r, _)| *r == resource)
                    .map(|(_, abundance)| {
                        let x = idx % self.width;
                        let y = idx / self.width;
                        (x, y, *abundance)
                    })
            })
            .collect()
    }

    /// Clear all resources.
    pub fn clear(&mut self) {
        self.resources.clear();
    }

    /// Get approximate memory usage in bytes.
    pub fn memory_usage(&self) -> usize {
        let base = std::mem::size_of::<Self>();
        let entries: usize = self.resources.values().map(|v| v.len()).sum();
        base + self.resources.capacity() * std::mem::size_of::<(usize, SmallVec<[(ResourceType, f32); 3]>)>()
            + entries * std::mem::size_of::<(ResourceType, f32)>()
    }
}

impl Default for ResourceMap {
    fn default() -> Self {
        Self::new(0, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparse_storage_works() {
        let mut map = ResourceMap::new(100, 100);
        map.set(50, 50, ResourceType::Gold, 0.8);
        map.set(50, 50, ResourceType::Iron, 0.5);

        assert!((map.get(50, 50, ResourceType::Gold) - 0.8).abs() < 0.001);
        assert!((map.get(50, 50, ResourceType::Iron) - 0.5).abs() < 0.001);
        assert_eq!(map.get(50, 50, ResourceType::Silver), 0.0);
        assert_eq!(map.get(0, 0, ResourceType::Gold), 0.0);
    }

    #[test]
    fn ignores_small_values() {
        let mut map = ResourceMap::new(100, 100);
        map.set(10, 10, ResourceType::Coal, 0.005);

        assert_eq!(map.get(10, 10, ResourceType::Coal), 0.0);
        assert!(!map.has_resources(10, 10));
    }

    #[test]
    fn get_all_returns_all_resources() {
        let mut map = ResourceMap::new(100, 100);
        map.set(25, 25, ResourceType::Gold, 0.9);
        map.set(25, 25, ResourceType::Gems, 0.7);
        map.set(25, 25, ResourceType::Silver, 0.5);

        let resources = map.get_all(25, 25);
        assert_eq!(resources.len(), 3);
    }

    #[test]
    fn locations_with_resource() {
        let mut map = ResourceMap::new(100, 100);
        map.set(10, 20, ResourceType::Iron, 0.8);
        map.set(30, 40, ResourceType::Iron, 0.6);
        map.set(50, 60, ResourceType::Gold, 0.9);

        let iron_locations = map.locations_with_resource(ResourceType::Iron);
        assert_eq!(iron_locations.len(), 2);

        let gold_locations = map.locations_with_resource(ResourceType::Gold);
        assert_eq!(gold_locations.len(), 1);
    }

    #[test]
    fn update_existing_resource() {
        let mut map = ResourceMap::new(100, 100);
        map.set(15, 15, ResourceType::Copper, 0.5);
        map.set(15, 15, ResourceType::Copper, 0.9);

        assert!((map.get(15, 15, ResourceType::Copper) - 0.9).abs() < 0.001);
        assert_eq!(map.get_all(15, 15).len(), 1);
    }
}

/// A volume's index in the AWS real-time NEXRAD bucket. These indexes are rotated-through as chunks
/// are accumulated and finally combined into full volumes to be archived.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct VolumeIndex(usize);

impl VolumeIndex {
    /// Creates a new volume index with the specified value.
    pub fn new(index: usize) -> Self {
        debug_assert!(index <= 999, "Volume index must be <= 999");
        Self(index)
    }

    /// Returns the volume index as a number.
    pub fn as_number(&self) -> usize {
        self.0
    }

    /// Returns the next volume index.
    pub fn next(&self) -> Self {
        if self.0 == 999 {
            Self::new(1)
        } else {
            Self::new(self.0 + 1)
        }
    }
}

/// Processing status flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProcessingStatus {
    /// Receiver noise (RxR Noise) processing.
    RxRNoise,
    /// Clutter Bypass Thresholding (CBT) processing.
    CBT,
    /// Other processing status value.
    Other(u16),
}

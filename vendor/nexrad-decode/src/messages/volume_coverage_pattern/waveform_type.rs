/// Possible values for waveform type.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum WaveformType {
    /// Contiguous Surveillance.
    CS,
    /// Contiguous Doppler with Ambiguity Resolution.
    CDW,
    /// Contiguous Doppler without Ambiguity Resolution.
    CDWO,
    /// Batch mode.
    B,
    /// Staggered Pulse Pair.
    SPP,
    /// Unknown waveform type.
    Unknown,
}

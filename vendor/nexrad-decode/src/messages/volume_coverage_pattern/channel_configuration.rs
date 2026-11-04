/// Possible values for channel configuration.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum ChannelConfiguration {
    /// Constant phase processing.
    ConstantPhase,
    /// Random phase processing.
    RandomPhase,
    /// SZ-2 phase coding for range dealiasing.
    SZ2Phase,
    /// Unknown phase configuration.
    UnknownPhase,
}

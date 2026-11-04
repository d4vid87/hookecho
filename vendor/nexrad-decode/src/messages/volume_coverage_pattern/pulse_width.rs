/// Possible values for pulse width.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum PulseWidth {
    /// Short pulse width for higher resolution.
    Short,
    /// Long pulse width for extended range.
    Long,
    /// Unknown pulse width value.
    Unknown,
}

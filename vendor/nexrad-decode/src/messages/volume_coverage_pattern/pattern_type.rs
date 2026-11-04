/// Possible values for the VCP pattern type.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum PatternType {
    /// Constant elevation scan pattern.
    Constant,
    /// Unknown pattern type.
    Unknown,
}

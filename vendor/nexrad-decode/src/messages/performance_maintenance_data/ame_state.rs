/// AME operational state.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum AmeState {
    /// AME is starting up.
    Start,
    /// AME is running normally.
    Running,
    /// AME is in flash programming mode.
    Flash,
    /// AME is in an error state.
    Error,
    /// Unknown state value for forward compatibility.
    Unknown(u16),
}

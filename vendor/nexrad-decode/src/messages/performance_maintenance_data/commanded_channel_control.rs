/// Commanded channel control setting.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum CommandedChannelControl {
    /// Not applicable.
    NotApplicable,
    /// Channel 1.
    Channel1,
    /// Channel 2.
    Channel2,
    /// Unknown value for forward compatibility.
    Unknown(u16),
}

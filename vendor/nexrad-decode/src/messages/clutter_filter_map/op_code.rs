/// Control codes indicating behavior of the clutter filter map for a range segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OpCode {
    /// The clutter filter is bypassed for the range segment.
    BypassFilter,
    /// The bypass map is in control for the range segment.
    BypassMapInControl,
    /// The clutter filter is being forced for the range segment.
    ForceFilter,
    /// Unknown opcode value for forward compatibility.
    Unknown(u16),
}

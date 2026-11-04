/// Receiver-to-antenna connection status.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum ReceiverConnectedToAntenna {
    /// Receiver is connected to the antenna.
    Connected,
    /// Receiver is not connected to the antenna.
    NotConnected,
    /// Not applicable.
    NotApplicable,
    /// Unknown status value for forward compatibility.
    Unknown(u16),
}

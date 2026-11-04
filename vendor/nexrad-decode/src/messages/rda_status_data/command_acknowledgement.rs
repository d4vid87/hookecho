/// Acknowledgement of command receipt by RDA system.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum CommandAcknowledgement {
    /// Remote VCP command was received.
    RemoteVCPReceived,
    /// Clutter bypass map was received.
    ClutterBypassMapReceived,
    /// Clutter censor zones were received.
    ClutterCensorZonesReceived,
    /// Redundant channel control command was accepted.
    RedundantChannelControlCommandAccepted,
}

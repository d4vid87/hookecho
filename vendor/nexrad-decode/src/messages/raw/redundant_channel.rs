/// The possible RDA redundant channels.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum RedundantChannel {
    /// Legacy RDA single channel configuration.
    LegacySingleChannel,
    /// Legacy RDA redundant channel 1.
    LegacyRedundantChannel1,
    /// Legacy RDA redundant channel 2.
    LegacyRedundantChannel2,
    /// ORDA single channel configuration.
    ORDASingleChannel,
    /// ORDA redundant channel 1.
    ORDARedundantChannel1,
    /// ORDA redundant channel 2.
    ORDARedundantChannel2,
    /// Unknown redundant channel value for forward compatibility.
    Unknown(u8),
}

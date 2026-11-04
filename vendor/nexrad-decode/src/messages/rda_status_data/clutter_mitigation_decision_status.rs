/// The possible RDA system clutter mitigation decision statuses.
#[derive(Debug, PartialEq, Eq, Clone, Hash)]
pub enum ClutterMitigationDecisionStatus {
    /// Clutter mitigation is disabled.
    Disabled,
    /// Clutter mitigation is enabled.
    Enabled,
    /// Which elevation segments of the bypass map are applied.
    BypassMapElevationSegments(Vec<u8>),
}

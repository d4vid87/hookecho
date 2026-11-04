/// Antenna polarization mode.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum Polarization {
    /// Horizontal polarization only.
    HorizontalOnly,
    /// Simultaneous horizontal and vertical polarization.
    HorizontalAndVertical,
    /// Vertical polarization only.
    VerticalOnly,
    /// Unknown value for forward compatibility.
    Unknown(u16),
}

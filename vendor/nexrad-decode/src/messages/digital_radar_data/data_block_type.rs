/// The type category of a data block.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash)]
pub enum DataBlockType {
    /// Radial-type data block containing volume, elevation, or radial metadata ('R').
    Radial,
    /// Data moment block containing measurement data ('D').
    DataMoment,
    /// Unknown data block type for forward compatibility.
    Unknown(u8),
}

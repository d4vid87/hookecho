use crate::messages::clutter_censor_zones::raw;
use crate::result::Result;
use crate::segmented_slice_reader::SegmentedSliceReader;
use std::borrow::Cow;
use std::fmt::Debug;

/// The Clutter Censor Zones message defines override regions for clutter filtering behavior.
/// Each region specifies a range, azimuth, and elevation zone along with an operator select code
/// that controls whether the bypass filter is forced, the bypass map is in control, or clutter
/// filtering is forced.
///
/// This message's contents correspond to ICD 2620002AA section 3.2.4.8 Table XII.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Message<'a> {
    header: Cow<'a, raw::Header>,
    regions: Cow<'a, [raw::Region]>,
}

impl<'a> Message<'a> {
    /// Parse a clutter censor zones message from segmented input.
    pub(crate) fn parse(reader: &mut SegmentedSliceReader<'a, '_>) -> Result<Self> {
        let header = reader.take_ref::<raw::Header>()?;
        let region_count = header.override_region_count.get() as usize;
        let regions = reader.take_slice::<raw::Region>(region_count)?;

        Ok(Self {
            header: Cow::Borrowed(header),
            regions: Cow::Borrowed(regions),
        })
    }

    /// Number of clutter map override regions (0 to 25).
    pub fn override_region_count(&self) -> u16 {
        self.header.override_region_count.get()
    }

    /// The override regions defined in this message.
    pub fn regions(&self) -> &[raw::Region] {
        &self.regions
    }

    /// Convert this message to an owned version with `'static` lifetime.
    pub fn into_owned(self) -> Message<'static> {
        Message {
            header: Cow::Owned(self.header.into_owned()),
            regions: Cow::Owned(self.regions.into_owned()),
        }
    }
}

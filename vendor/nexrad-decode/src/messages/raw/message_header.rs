use super::primitive_aliases::{Integer1, Integer2, Integer4};
use super::{MessageType, RedundantChannel};
use crate::binary_data::BinaryData;
use crate::util::get_datetime;
use chrono::{DateTime, Duration, Utc};
use std::fmt::Debug;
use zerocopy::{FromBytes, Immutable, KnownLayout};

#[cfg(feature = "uom")]
use uom::si::f64::Information;
#[cfg(feature = "uom")]
use uom::si::information::byte;

/// Sentinel value for the `segment_size` field indicating variable-length framing.
pub const VARIABLE_LENGTH_MESSAGE_SIZE: u16 = 65535;

/// Message and system configuration information appended to the beginning of all messages.
///
/// Use [`segmented()`](Self::segmented) to determine the framing mode, and
/// [`message_size_bytes()`](Self::message_size_bytes) for the message size.
#[repr(C)]
#[derive(Clone, PartialEq, Eq, Hash, Debug, FromBytes, Immutable, KnownLayout)]
pub struct MessageHeader {
    /// Unknown/reserved bytes from RPG.
    pub(crate) rpg_unknown: BinaryData<[u8; 12]>,

    /// Size of this segment in half-words. Note that this only describes this segment's size,
    /// though there could be multiple segments. In the case of a variable-length message (indicated
    /// by this field being set to 65535/0xFFFF), the full message's size is determined differently.
    /// See [MessageHeader::message_size_bytes] and [MessageHeader::segment_count] for more
    /// information.
    pub segment_size: Integer2,

    /// Whether the RDA is operating on a redundant channel.
    ///
    /// Legacy:
    ///  0 = Single Channel (no bits set)
    ///  1 = Redundant Channel 1 (bit 0 set)
    ///  2 = Redundant Channel 2 (bit 1 set)
    /// ORDA
    ///  8 = Single Channel (bit 3 set)
    ///  9 = Redundant Channel 1 (bits 0 and 3 set)
    /// 10 = Redundant Channel 2 (bits 1 and 3 set)
    pub redundant_channel: Integer1,

    /// Type discriminator.
    pub message_type: Integer1,

    /// Message sequence number.
    pub sequence_number: Integer2,

    /// This message's date represented as a count of days since 1 January 1970 00:00 GMT. It is
    /// also referred-to as a "modified Julian date" where it is the Julian date - 2440586.5.
    pub date: Integer2,

    /// Milliseconds past midnight, GMT.
    pub time: Integer4,

    /// Number of segments in this message. If the message is segmented, this field is meaningful,
    /// otherwise bytes 12-15 (this field and [MessageHeader::segment_number]) specify the size of
    /// the message in bytes.
    pub segment_count: Integer2,

    /// This message segment's number. If the message is segmented, this field is meaningful,
    /// otherwise bytes 12-15 (this field and [MessageHeader::segment_count]) specify the size of
    /// the message in bytes.
    pub segment_number: Integer2,
}

impl MessageHeader {
    /// If this message is [MessageHeader::segmented], this indicates this message segment's size.
    /// Otherwise, this returns [None] and [MessageHeader::message_size] should be used to determine
    /// the message's full size.
    #[cfg(feature = "uom")]
    pub fn segment_size(&self) -> Option<Information> {
        let segment_size = self.segment_size.get();
        if segment_size < VARIABLE_LENGTH_MESSAGE_SIZE {
            Some(Information::new::<byte>((segment_size * 2) as f64))
        } else {
            None
        }
    }

    /// Whether the RDA is operating on a redundant channel.
    pub fn rda_redundant_channel(&self) -> RedundantChannel {
        match self.redundant_channel {
            0 => RedundantChannel::LegacySingleChannel,
            1 => RedundantChannel::LegacyRedundantChannel1,
            2 => RedundantChannel::LegacyRedundantChannel2,
            8 => RedundantChannel::ORDASingleChannel,
            9 => RedundantChannel::ORDARedundantChannel1,
            10 => RedundantChannel::ORDARedundantChannel2,
            other => RedundantChannel::Unknown(other),
        }
    }

    /// Message type discriminator.
    pub fn message_type(&self) -> MessageType {
        match self.message_type {
            1 => MessageType::RDADigitalRadarData,
            2 => MessageType::RDAStatusData,
            3 => MessageType::RDAPerformanceMaintenanceData,
            4 => MessageType::RDAConsoleMessage,
            5 => MessageType::RDAVolumeCoveragePattern,
            6 => MessageType::RDAControlCommands,
            7 => MessageType::RPGVolumeCoveragePattern,
            8 => MessageType::RPGClutterCensorZones,
            9 => MessageType::RPGRequestForData,
            10 => MessageType::RPGConsoleMessage,
            11 => MessageType::RDALoopBackTest,
            12 => MessageType::RPGLoopBackTest,
            13 => MessageType::RDAClutterFilterBypassMap,
            14 => MessageType::Spare1,
            15 => MessageType::RDAClutterFilterMap,
            16 => MessageType::ReservedFAARMSOnly1,
            17 => MessageType::ReservedFAARMSOnly2,
            18 => MessageType::RDAAdaptationData,
            20 => MessageType::Reserved1,
            21 => MessageType::Reserved2,
            22 => MessageType::Reserved3,
            23 => MessageType::Reserved4,
            24 => MessageType::ReservedFAARMSOnly3,
            25 => MessageType::ReservedFAARMSOnly4,
            26 => MessageType::ReservedFAARMSOnly5,
            29 => MessageType::Reserved5,
            31 => MessageType::RDADigitalRadarDataGenericFormat,
            32 => MessageType::RDAPRFData,
            33 => MessageType::RDALogData,
            _ => MessageType::Unknown(self.message_type),
        }
    }

    /// This message's date and time in UTC.
    pub fn date_time(&self) -> Option<DateTime<Utc>> {
        get_datetime(
            self.date.get(),
            Duration::milliseconds(self.time.get() as i64),
        )
    }

    /// Whether this message uses fixed-segment framing (2432-byte frames).
    ///
    /// Segmented messages occupy one or more fixed-size frames. Variable-length messages
    /// (where this returns `false`) carry their own size and are not padded to frame
    /// boundaries. Digital Radar Data (Type 31) is always variable-length.
    pub fn segmented(&self) -> bool {
        // ICD Table II: segment_size == 0xFFFF indicates variable-length framing.
        //
        // Type 31 (Digital Radar Data) is also variable-length, but in practice its
        // segment_size field contains the actual content halfwords rather than the
        // ICD-specified 0xFFFF sentinel. We treat Type 31 as variable-length regardless.
        self.segment_size < VARIABLE_LENGTH_MESSAGE_SIZE
            && self.message_type() != MessageType::RDADigitalRadarDataGenericFormat
    }

    /// If the message is [MessageHeader::segmented], this indicates the number of segments in the
    /// full message, otherwise this returns [None]. [MessageHeader::message_size_bytes] can be used
    /// to determine the message's full size.
    pub fn segment_count(&self) -> Option<u16> {
        if self.segmented() {
            Some(self.segment_count.get())
        } else {
            None
        }
    }

    /// If the message is [MessageHeader::segmented], this indicates this segment's number/sequence
    /// in the message, otherwise this returns [None]. [MessageHeader::message_size_bytes] can be
    /// used to determine the message's full size.
    pub fn segment_number(&self) -> Option<u16> {
        if self.segmented() {
            Some(self.segment_number.get())
        } else {
            None
        }
    }

    /// The full size of the message in bytes. If the message is [MessageHeader::segmented] then
    /// this is the segment size, otherwise this is the full variable-length message size.
    ///
    /// For variable-length messages (segment size = 0xFFFF), the segment count and segment number
    /// fields are repurposed as a 32-bit message size in bytes per ICD Table II.
    ///
    /// Note: this uses the raw `segment_size` field rather than [`segmented()`](Self::segmented)
    /// because Type 31 messages store their halfword count in `segment_size` even though they
    /// use variable-length framing.
    pub fn message_size_bytes(&self) -> u32 {
        if self.segment_size.get() < VARIABLE_LENGTH_MESSAGE_SIZE {
            // segment_size field is in halfwords
            self.segment_size.get() as u32 * 2
        } else {
            // Variable-length: segment_count and segment_number fields are repurposed
            // as a 32-bit message size in bytes (ICD Table II)
            let high = self.segment_count.get() as u32;
            let low = self.segment_number.get() as u32;
            (high << 16) | low
        }
    }

    /// The full size of the message. If the message is [MessageHeader::segmented] then this is the
    /// segment size, otherwise this is the full variable-length message size.
    ///
    /// For variable-length messages (segment size = 0xFFFF), the segment count and segment number
    /// fields are repurposed as a 32-bit message size in bytes per ICD Table II.
    #[cfg(feature = "uom")]
    pub fn message_size(&self) -> Information {
        Information::new::<byte>(self.message_size_bytes() as f64)
    }
}

use crate::messages::primitive_aliases::{Integer4, Real4};
use crate::messages::rda_adaptation_data::raw;
use crate::result::Result;
use crate::segmented_slice_reader::SegmentedSliceReader;
use std::borrow::Cow;
use std::fmt::Debug;
use zerocopy::FromBytes;

/// The RDA Adaptation Data message (Type 18) contains site-specific configuration parameters
/// for the radar system, including antenna parameters, site location, RF path losses,
/// calibration values, and operational thresholds.
///
/// This message's contents correspond to ICD 2620002Y section 3.2.4.16 Table XV. The full
/// message spans approximately 9468 bytes across multiple fixed-length segments. The identity
/// header (44 bytes) is parsed as a structured type, while the remaining data is stored as raw
/// bytes with typed accessor methods for individual fields at known ICD byte offsets.
#[derive(Clone, PartialEq, Debug)]
pub struct Message<'a> {
    /// The 44-byte identity header (bytes 0-43).
    header: Cow<'a, raw::Header>,

    /// Raw adaptation data bytes after the header (bytes 44 onward).
    /// Field accessors index into this data using (ICD_byte_offset - 44).
    data: Vec<u8>,
}

/// Helper to read a big-endian f32 (Real*4) from a byte slice at a given offset.
fn read_real4(data: &[u8], offset: usize) -> Option<f32> {
    Real4::read_from_bytes(data.get(offset..offset + 4)?)
        .ok()
        .map(|v| v.get())
}

/// Helper to read a big-endian u32 (Integer*4) from a byte slice at a given offset.
fn read_integer4(data: &[u8], offset: usize) -> Option<u32> {
    Integer4::read_from_bytes(data.get(offset..offset + 4)?)
        .ok()
        .map(|v| v.get())
}

/// Helper to read a big-endian i32 (SInteger*4) from a byte slice at a given offset.
fn read_sinteger4(data: &[u8], offset: usize) -> Option<i32> {
    let bytes = data.get(offset..offset + 4)?;
    Some(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// Helper to extract a null-terminated (or fixed-length) ASCII string from a byte slice.
fn read_string(data: &[u8], offset: usize, len: usize) -> Option<String> {
    let bytes = data.get(offset..offset + len)?;
    let s = std::str::from_utf8(bytes).ok()?;
    // Trim trailing nulls and whitespace
    Some(s.trim_end_matches('\0').trim().to_string())
}

impl<'a> Message<'a> {
    /// Parse an RDA Adaptation Data message from segmented input.
    ///
    /// The adaptation data message spans multiple fixed-length segments. The 44-byte identity
    /// header is parsed as a structured type, and the remaining bytes are collected across
    /// all segments into a contiguous buffer for offset-based field access.
    pub(crate) fn parse(reader: &mut SegmentedSliceReader<'a, '_>) -> Result<Self> {
        let header = reader.take_ref::<raw::Header>()?;

        // Collect all remaining data across segments into a contiguous buffer.
        let data = reader.read_bytes_owned(reader.remaining_total())?;

        Ok(Self {
            header: Cow::Borrowed(header),
            data,
        })
    }

    // -----------------------------------------------------------------------
    // Identity fields (bytes 0-43, from the parsed header)
    // -----------------------------------------------------------------------

    /// Name of the adaptation data file (bytes 0-11).
    pub fn adap_file_name(&self) -> Option<String> {
        let s = std::str::from_utf8(&self.header.adap_file_name).ok()?;
        Some(s.trim_end_matches('\0').trim().to_string())
    }

    /// Format of the adaptation data file (bytes 12-15).
    pub fn adap_format(&self) -> Option<String> {
        let s = std::str::from_utf8(&self.header.adap_format).ok()?;
        Some(s.trim_end_matches('\0').trim().to_string())
    }

    /// Revision number of the adaptation data file (bytes 16-19).
    pub fn adap_revision(&self) -> Option<String> {
        let s = std::str::from_utf8(&self.header.adap_revision).ok()?;
        Some(s.trim_end_matches('\0').trim().to_string())
    }

    /// Last modified date of the adaptation data file in "mm/dd/yy" format (bytes 20-31).
    pub fn adap_date(&self) -> Option<String> {
        let s = std::str::from_utf8(&self.header.adap_date).ok()?;
        Some(s.trim_end_matches('\0').trim().to_string())
    }

    /// Last modified time of the adaptation data file in "hh:mm:ss" format (bytes 32-43).
    pub fn adap_time(&self) -> Option<String> {
        let s = std::str::from_utf8(&self.header.adap_time).ok()?;
        Some(s.trim_end_matches('\0').trim().to_string())
    }

    // -----------------------------------------------------------------------
    // Antenna/pedestal parameters (bytes 44-67)
    // All offsets into self.data are (ICD_byte - 44).
    // -----------------------------------------------------------------------

    /// Angle of the lower pre-limit switch in degrees (bytes 44-47).
    pub fn lower_pre_limit(&self) -> Option<f32> {
        read_real4(&self.data, 0)
    }

    /// Latency of azimuth encoder measurement in seconds (bytes 48-51).
    pub fn az_lat(&self) -> Option<f32> {
        read_real4(&self.data, 4)
    }

    /// Angle of the upper pre-limit switch in degrees (bytes 52-55).
    pub fn upper_pre_limit(&self) -> Option<f32> {
        read_real4(&self.data, 8)
    }

    /// Latency of elevation encoder measurement in seconds (bytes 56-59).
    pub fn el_lat(&self) -> Option<f32> {
        read_real4(&self.data, 12)
    }

    /// Pedestal park position in azimuth in degrees (bytes 60-63).
    pub fn parkaz(&self) -> Option<f32> {
        read_real4(&self.data, 16)
    }

    /// Pedestal park position in elevation in degrees (bytes 64-67).
    pub fn parkel(&self) -> Option<f32> {
        read_real4(&self.data, 20)
    }

    // -----------------------------------------------------------------------
    // Fuel level conversion values (bytes 68-111)
    // -----------------------------------------------------------------------

    /// Generator fuel level height/capacity conversion values (bytes 68-111).
    /// Returns up to 11 values for 0% through 100% in 10% increments.
    pub fn a_fuel_conv(&self) -> Vec<f32> {
        (0..11)
            .filter_map(|i| read_real4(&self.data, 24 + i * 4))
            .collect()
    }

    // -----------------------------------------------------------------------
    // Temperature limits (bytes 112-155)
    // -----------------------------------------------------------------------

    /// Minimum equipment shelter alarm temperature in deg C (bytes 112-115).
    pub fn a_min_shelter_temp(&self) -> Option<f32> {
        read_real4(&self.data, 68)
    }

    /// Maximum equipment shelter alarm temperature in deg C (bytes 116-119).
    pub fn a_max_shelter_temp(&self) -> Option<f32> {
        read_real4(&self.data, 72)
    }

    /// Minimum A/C discharge air temperature differential in deg C (bytes 120-123).
    pub fn a_min_shelter_ac_temp_diff(&self) -> Option<f32> {
        read_real4(&self.data, 76)
    }

    /// Maximum transmitter leaving air alarm temperature in deg C (bytes 124-127).
    pub fn a_max_xmtr_air_temp(&self) -> Option<f32> {
        read_real4(&self.data, 80)
    }

    /// Maximum radome alarm temperature in deg C (bytes 128-131).
    pub fn a_max_rad_temp(&self) -> Option<f32> {
        read_real4(&self.data, 84)
    }

    /// Maximum radome minus ambient temperature difference in deg C (bytes 132-135).
    pub fn a_max_rad_temp_rise(&self) -> Option<f32> {
        read_real4(&self.data, 88)
    }

    // -----------------------------------------------------------------------
    // Dead limits (bytes 136-143)
    // -----------------------------------------------------------------------

    /// Angle of lower dead limit switch in degrees (bytes 136-139).
    pub fn lower_dead_limit(&self) -> Option<f32> {
        read_real4(&self.data, 92)
    }

    /// Angle of upper dead limit switch in degrees (bytes 140-143).
    pub fn upper_dead_limit(&self) -> Option<f32> {
        read_real4(&self.data, 96)
    }

    // -----------------------------------------------------------------------
    // Generator room temperature (bytes 148-155)
    // -----------------------------------------------------------------------

    /// Minimum generator shelter alarm temperature in deg C (bytes 148-151).
    pub fn a_min_gen_room_temp(&self) -> Option<f32> {
        read_real4(&self.data, 104)
    }

    /// Maximum generator shelter alarm temperature in deg C (bytes 152-155).
    pub fn a_max_gen_room_temp(&self) -> Option<f32> {
        read_real4(&self.data, 108)
    }

    // -----------------------------------------------------------------------
    // Power supply tolerances (bytes 156-163)
    // -----------------------------------------------------------------------

    /// SPIP +5 volt power supply tolerance in % (bytes 156-159).
    pub fn spip_5v_reg_lim(&self) -> Option<f32> {
        read_real4(&self.data, 112)
    }

    /// SPIP +/- 15 volt power supply tolerance in % (bytes 160-163).
    pub fn spip_15v_reg_lim(&self) -> Option<f32> {
        read_real4(&self.data, 116)
    }

    // -----------------------------------------------------------------------
    // RPG co-located and installed equipment flags (bytes 176-191)
    // -----------------------------------------------------------------------

    /// Whether the RPG is co-located with the RDA, as a "T" or "F" string (bytes 176-179).
    pub fn rpg_co_located(&self) -> Option<String> {
        read_string(&self.data, 132, 4)
    }

    /// Whether a transmitter spectrum filter is installed (bytes 180-183).
    pub fn spec_filter_installed(&self) -> Option<String> {
        read_string(&self.data, 136, 4)
    }

    /// Whether a transition power source is installed (bytes 184-187).
    pub fn tps_installed(&self) -> Option<String> {
        read_string(&self.data, 140, 4)
    }

    /// Whether FAA RMS is installed (bytes 188-191).
    pub fn rms_installed(&self) -> Option<String> {
        read_string(&self.data, 144, 4)
    }

    // -----------------------------------------------------------------------
    // Performance test and power parameters (bytes 192-227)
    // -----------------------------------------------------------------------

    /// Performance/HVDL test interval in hours (bytes 192-195).
    pub fn a_hvdl_tst_int(&self) -> Option<u32> {
        read_integer4(&self.data, 148)
    }

    /// RPG loop test interval in minutes (bytes 196-199).
    pub fn a_rpg_lt_int(&self) -> Option<u32> {
        read_integer4(&self.data, 152)
    }

    /// Required interval time for stable utility power in minutes (bytes 200-203).
    pub fn a_min_stab_util_pwr_time(&self) -> Option<u32> {
        read_integer4(&self.data, 156)
    }

    /// Maximum generator automatic exercise interval in hours (bytes 204-207).
    pub fn a_gen_auto_exer_interval(&self) -> Option<u32> {
        read_integer4(&self.data, 160)
    }

    /// Recommended switch to utility power time interval in minutes (bytes 208-211).
    pub fn a_util_pwr_sw_req_interval(&self) -> Option<u32> {
        read_integer4(&self.data, 164)
    }

    /// Low fuel tank warning level in % (bytes 212-215).
    pub fn a_low_fuel_level(&self) -> Option<f32> {
        read_real4(&self.data, 168)
    }

    /// Configuration channel number: 1 or 2 (bytes 216-219).
    pub fn config_chan_number(&self) -> Option<u32> {
        read_integer4(&self.data, 172)
    }

    /// Redundant channel configuration: 1=Single, 2=FAA, 3=NWS Redundant (bytes 224-227).
    pub fn redundant_chan_config(&self) -> Option<u32> {
        read_integer4(&self.data, 180)
    }

    // -----------------------------------------------------------------------
    // Test signal attenuator insertion losses (bytes 228-643)
    // 104 Real*4 values, ATTEN_TABLE(0) through ATTEN_TABLE(103)
    // -----------------------------------------------------------------------

    /// Test signal attenuator insertion loss for a given attenuation index (0-103) in dB.
    /// Bytes 228-643, each is a Real*4.
    pub fn atten_table(&self, index: usize) -> Option<f32> {
        if index > 103 {
            return None;
        }
        // ATTEN_TABLE(0) is at byte 228 = data offset 184
        read_real4(&self.data, 184 + index * 4)
    }

    // -----------------------------------------------------------------------
    // Path losses and RF parameters (bytes 644-927)
    // -----------------------------------------------------------------------

    /// Path loss - vertical IF heliax to 4AT16 in dB (bytes 668-671).
    pub fn path_loss_vertical_if_heliax(&self) -> Option<f32> {
        read_real4(&self.data, 624)
    }

    /// Path loss - 2A9A9 RF delay line in dB (bytes 692-695).
    pub fn path_loss_2a9a9(&self) -> Option<f32> {
        read_real4(&self.data, 648)
    }

    /// Path loss - horizontal IF heliax to 4AT17 in dB (bytes 752-755).
    pub fn path_loss_horizontal_if_heliax(&self) -> Option<f32> {
        read_real4(&self.data, 708)
    }

    /// RF pallet horizontal coupler transmitter loss in dB (bytes 756-759).
    pub fn h_coupler_xmt_loss(&self) -> Option<f32> {
        read_real4(&self.data, 712)
    }

    /// Path loss - WG02 harmonic filter in dB (bytes 768-771).
    pub fn path_loss_wg02(&self) -> Option<f32> {
        read_real4(&self.data, 724)
    }

    /// Path loss - waveguide klystron T0 switch in dB (bytes 772-775).
    pub fn path_loss_waveguide_klystron(&self) -> Option<f32> {
        read_real4(&self.data, 728)
    }

    /// Path loss - WG06 spectrum filter in dB (bytes 780-783).
    pub fn path_loss_wg06(&self) -> Option<f32> {
        read_real4(&self.data, 736)
    }

    /// Path loss - WG04 circulator in dB (bytes 796-799).
    pub fn path_loss_wg04(&self) -> Option<f32> {
        read_real4(&self.data, 752)
    }

    /// Path loss - A6 arc detector in dB (bytes 800-803).
    pub fn path_loss_a6(&self) -> Option<f32> {
        read_real4(&self.data, 756)
    }

    // -----------------------------------------------------------------------
    // Coupler losses (bytes 832-855)
    // -----------------------------------------------------------------------

    /// RF pallet horizontal coupler test signal loss in dB (bytes 832-835).
    pub fn h_coupler_cw_loss(&self) -> Option<f32> {
        read_real4(&self.data, 788)
    }

    /// RF pallet vertical coupler transmitter loss in dB (bytes 836-839).
    pub fn v_coupler_xmt_loss(&self) -> Option<f32> {
        read_real4(&self.data, 792)
    }

    /// RF pallet vertical coupler test signal loss in dB (bytes 852-855).
    pub fn v_coupler_cw_loss(&self) -> Option<f32> {
        read_real4(&self.data, 808)
    }

    // -----------------------------------------------------------------------
    // Power sense and AME parameters (bytes 864-871)
    // -----------------------------------------------------------------------

    /// Power sense calibration offset bias in dB (bytes 864-867).
    pub fn pwr_sense_bias(&self) -> Option<f32> {
        read_real4(&self.data, 820)
    }

    /// AME noise source excess noise ratio in dB (bytes 868-871).
    pub fn ame_v_noise_enr(&self) -> Option<f32> {
        read_real4(&self.data, 824)
    }

    // -----------------------------------------------------------------------
    // Test signal power levels (bytes 936-939)
    // -----------------------------------------------------------------------

    /// AME vertical test signal power in dBm (bytes 936-939).
    pub fn v_ts_cw(&self) -> Option<f32> {
        read_real4(&self.data, 892)
    }

    // -----------------------------------------------------------------------
    // Horizontal receiver noise normalization (bytes 940-991)
    // H_RNSCALE(0) through H_RNSCALE(12)
    // -----------------------------------------------------------------------

    /// Horizontal receiver noise normalization value for a given elevation sector (0-12).
    /// Bytes 940-991, each is a Real*4.
    pub fn h_rnscale(&self, index: usize) -> Option<f32> {
        if index > 12 {
            return None;
        }
        // H_RNSCALE(0) is at byte 940 = data offset 896
        read_real4(&self.data, 896 + index * 4)
    }

    // -----------------------------------------------------------------------
    // Atmospheric loss values (bytes 992-1043)
    // ATMOS(0) through ATMOS(12)
    // -----------------------------------------------------------------------

    /// Two-way atmospheric loss per km for a given elevation sector (0-12) in dB/km.
    /// Bytes 992-1043, each is a Real*4.
    pub fn atmos(&self, index: usize) -> Option<f32> {
        if index > 12 {
            return None;
        }
        // ATMOS(0) is at byte 992 = data offset 948
        read_real4(&self.data, 948 + index * 4)
    }

    // -----------------------------------------------------------------------
    // Bypass map elevation angles (bytes 1044-1091)
    // EL_INDEX(0) through EL_INDEX(11)
    // -----------------------------------------------------------------------

    /// Bypass map generation elevation angle for a given index (0-11) in degrees.
    /// Bytes 1044-1091, each is a Real*4.
    pub fn el_index(&self, index: usize) -> Option<f32> {
        if index > 11 {
            return None;
        }
        // EL_INDEX(0) is at byte 1044 = data offset 1000
        read_real4(&self.data, 1000 + index * 4)
    }

    // -----------------------------------------------------------------------
    // Transmitter and calibration parameters (bytes 1092-1243)
    // -----------------------------------------------------------------------

    /// Transmitter frequency in MHz (bytes 1092-1095).
    pub fn tfreq_mhz(&self) -> Option<u32> {
        read_integer4(&self.data, 1048)
    }

    /// Base data point clutter suppression threshold (TCN) in dB (bytes 1096-1099).
    pub fn base_data_tcn(&self) -> Option<f32> {
        read_real4(&self.data, 1052)
    }

    /// Range unfolding overlay threshold (TOVER) in dB (bytes 1100-1103).
    pub fn refl_data_tover(&self) -> Option<f32> {
        read_real4(&self.data, 1056)
    }

    /// Horizontal target system calibration (dBZ0) for long pulse in dBZ (bytes 1104-1107).
    pub fn tar_h_dbz0_lp(&self) -> Option<f32> {
        read_real4(&self.data, 1060)
    }

    /// Vertical target system calibration (dBZ0) for long pulse in dBZ (bytes 1108-1111).
    pub fn tar_v_dbz0_lp(&self) -> Option<f32> {
        read_real4(&self.data, 1064)
    }

    /// Initial system differential phase in degrees (bytes 1112-1115).
    pub fn init_phi_dp(&self) -> Option<u32> {
        read_integer4(&self.data, 1068)
    }

    /// Normalized initial system differential phase in degrees (bytes 1116-1119).
    pub fn norm_init_phi_dp(&self) -> Option<u32> {
        read_integer4(&self.data, 1072)
    }

    /// Matched filter loss for long pulse in dB (bytes 1120-1123).
    pub fn lx_lp(&self) -> Option<f32> {
        read_real4(&self.data, 1076)
    }

    /// Matched filter loss for short pulse in dB (bytes 1124-1127).
    pub fn lx_sp(&self) -> Option<f32> {
        read_real4(&self.data, 1080)
    }

    /// Hydrometeor refractivity factor (K**2) (bytes 1128-1131).
    pub fn meteor_param(&self) -> Option<f32> {
        read_real4(&self.data, 1084)
    }

    /// Antenna gain including radome in dB (bytes 1136-1139).
    pub fn antenna_gain(&self) -> Option<f32> {
        read_real4(&self.data, 1092)
    }

    /// Velocity check delta degrade limit in m/s (bytes 1152-1155).
    pub fn vel_degrad_limit(&self) -> Option<f32> {
        read_real4(&self.data, 1108)
    }

    /// Spectrum width check delta degrade limit in m/s (bytes 1156-1159).
    pub fn wth_degrad_limit(&self) -> Option<f32> {
        read_real4(&self.data, 1112)
    }

    /// Horizontal system noise temp degrade limit in K (bytes 1160-1163).
    pub fn h_noisetemp_dgrad_limit(&self) -> Option<f32> {
        read_real4(&self.data, 1116)
    }

    /// Horizontal system noise temp too low limit in K (bytes 1164-1167).
    pub fn h_min_noisetemp(&self) -> Option<u32> {
        read_integer4(&self.data, 1120)
    }

    /// Vertical system noise temp degrade limit in K (bytes 1168-1171).
    pub fn v_noisetemp_dgrad_limit(&self) -> Option<f32> {
        read_real4(&self.data, 1124)
    }

    /// Vertical system noise temp too low limit in K (bytes 1172-1175).
    pub fn v_min_noisetemp(&self) -> Option<u32> {
        read_integer4(&self.data, 1128)
    }

    /// Klystron output target consistency degrade limit in dB (bytes 1176-1179).
    pub fn kly_degrade_limit(&self) -> Option<f32> {
        read_real4(&self.data, 1132)
    }

    /// COHO power at A1J4 in dBm (bytes 1180-1183).
    pub fn ts_coho(&self) -> Option<f32> {
        read_real4(&self.data, 1136)
    }

    /// AME horizontal test signal power in dBm (bytes 1184-1187).
    pub fn h_ts_cw(&self) -> Option<f32> {
        read_real4(&self.data, 1140)
    }

    /// STALO power at A1J2 in dBm (bytes 1196-1199).
    pub fn ts_stalo(&self) -> Option<f32> {
        read_real4(&self.data, 1152)
    }

    /// AME horizontal noise source excess noise ratio in dB (bytes 1200-1203).
    pub fn ame_h_noise_enr(&self) -> Option<f32> {
        read_real4(&self.data, 1156)
    }

    /// Maximum transmitter peak power alarm level in kW (bytes 1204-1207).
    pub fn xmtr_peak_pwr_high_limit(&self) -> Option<f32> {
        read_real4(&self.data, 1160)
    }

    /// Minimum transmitter peak power alarm level in kW (bytes 1208-1211).
    pub fn xmtr_peak_pwr_low_limit(&self) -> Option<f32> {
        read_real4(&self.data, 1164)
    }

    /// Difference between computed and target horizontal dBZ0 limit in dB (bytes 1212-1215).
    pub fn h_dbz0_delta_limit(&self) -> Option<f32> {
        read_real4(&self.data, 1168)
    }

    /// Bypass map generator noise threshold in dB (bytes 1216-1219).
    pub fn threshold1(&self) -> Option<f32> {
        read_real4(&self.data, 1172)
    }

    /// Bypass map generator rejection ratio threshold in dB (bytes 1220-1223).
    pub fn threshold2(&self) -> Option<f32> {
        read_real4(&self.data, 1176)
    }

    /// Clutter suppression degrade limit in dB (bytes 1224-1227).
    pub fn clut_supp_dgrad_lim(&self) -> Option<f32> {
        read_real4(&self.data, 1180)
    }

    /// True range at start of first range bin in km (bytes 1232-1235).
    pub fn range0_value(&self) -> Option<f32> {
        read_real4(&self.data, 1188)
    }

    /// Scale factor used to convert transmitter power byte data to watts (bytes 1236-1239).
    pub fn xmtr_pwr_mtr_scale(&self) -> Option<f32> {
        read_real4(&self.data, 1192)
    }

    /// Difference between computed and target vertical dBZ0 limit in dB (bytes 1240-1243).
    pub fn v_dbz0_delta_limit(&self) -> Option<f32> {
        read_real4(&self.data, 1196)
    }

    /// Horizontal target system calibration (dBZ0) for short pulse in dBZ (bytes 1244-1247).
    pub fn tar_h_dbz0_sp(&self) -> Option<f32> {
        read_real4(&self.data, 1200)
    }

    /// Vertical target system calibration (dBZ0) for short pulse in dBZ (bytes 1248-1251).
    pub fn tar_v_dbz0_sp(&self) -> Option<f32> {
        read_real4(&self.data, 1204)
    }

    // -----------------------------------------------------------------------
    // PRF and pulse parameters (bytes 1252-1283)
    // -----------------------------------------------------------------------

    /// Site PRF set (A=1, B=2, C=3, D=4, E=5) (bytes 1252-1255).
    pub fn deltaprf(&self) -> Option<u32> {
        read_integer4(&self.data, 1208)
    }

    /// Pulse width of transmitter output in short pulse mode in nsec (bytes 1264-1267).
    pub fn tau_sp(&self) -> Option<u32> {
        read_integer4(&self.data, 1220)
    }

    /// Pulse width of transmitter output in long pulse mode in nsec (bytes 1268-1271).
    pub fn tau_lp(&self) -> Option<u32> {
        read_integer4(&self.data, 1224)
    }

    /// Number of 1/4 km bins of corrupted data at end of sweep (bytes 1272-1275).
    pub fn nc_dead_value(&self) -> Option<u32> {
        read_integer4(&self.data, 1228)
    }

    /// RF drive pulse width in short pulse mode in nsec (bytes 1276-1279).
    pub fn tau_rf_sp(&self) -> Option<u32> {
        read_integer4(&self.data, 1232)
    }

    /// RF drive pulse width in long pulse mode in nsec (bytes 1280-1283).
    pub fn tau_rf_lp(&self) -> Option<u32> {
        read_integer4(&self.data, 1236)
    }

    // -----------------------------------------------------------------------
    // Clutter map boundary and site location (bytes 1284-1323)
    // -----------------------------------------------------------------------

    /// Clutter map boundary elevation between segments 1 and 2 in degrees (bytes 1284-1287).
    pub fn seg1lim(&self) -> Option<f32> {
        read_real4(&self.data, 1240)
    }

    /// Site latitude in seconds (bytes 1288-1291).
    pub fn slatsec(&self) -> Option<f32> {
        read_real4(&self.data, 1244)
    }

    /// Site longitude in seconds (bytes 1292-1295).
    pub fn slonsec(&self) -> Option<f32> {
        read_real4(&self.data, 1248)
    }

    /// Site latitude in degrees (bytes 1300-1303).
    pub fn slatdeg(&self) -> Option<u32> {
        read_integer4(&self.data, 1256)
    }

    /// Site latitude in minutes (bytes 1304-1307).
    pub fn slatmin(&self) -> Option<u32> {
        read_integer4(&self.data, 1260)
    }

    /// Site longitude in degrees (bytes 1308-1311).
    pub fn slondeg(&self) -> Option<u32> {
        read_integer4(&self.data, 1264)
    }

    /// Site longitude in minutes (bytes 1312-1315).
    pub fn slonmin(&self) -> Option<u32> {
        read_integer4(&self.data, 1268)
    }

    /// Site latitude direction ("N" or "S") (bytes 1316-1319).
    pub fn slatdir(&self) -> Option<String> {
        read_string(&self.data, 1272, 4)
    }

    /// Site longitude direction ("E" or "W") (bytes 1320-1323).
    pub fn slondir(&self) -> Option<String> {
        read_string(&self.data, 1276, 4)
    }

    /// Compute the site latitude as a signed decimal degrees value, combining the degree,
    /// minute, second, and direction fields per the ICD.
    pub fn site_latitude(&self) -> Option<f64> {
        let deg = self.slatdeg()? as f64;
        let min = self.slatmin()? as f64;
        let sec = self.slatsec()? as f64;
        let dir = self.slatdir()?;
        let sign = if dir.starts_with('S') { -1.0 } else { 1.0 };
        Some(sign * (deg + min / 60.0 + sec / 3600.0))
    }

    /// Compute the site longitude as a signed decimal degrees value, combining the degree,
    /// minute, second, and direction fields per the ICD.
    pub fn site_longitude(&self) -> Option<f64> {
        let deg = self.slondeg()? as f64;
        let min = self.slonmin()? as f64;
        let sec = self.slonsec()? as f64;
        let dir = self.slondir()?;
        let sign = if dir.starts_with('W') { -1.0 } else { 1.0 };
        Some(sign * (deg + min / 60.0 + sec / 3600.0))
    }

    // -----------------------------------------------------------------------
    // Receiver clock and COHO frequencies (bytes 2500-2515)
    // Note: there is a large spare gap from bytes 1324-2499
    // -----------------------------------------------------------------------

    /// Receiver digital clock frequency in MHz (bytes 2500-2507). Real*8.
    pub fn dig_rcvr_clock_freq(&self) -> Option<f64> {
        let offset = 2500 - 44;
        let bytes = self.data.get(offset..offset + 8)?;
        Some(f64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    /// COHO frequency in MHz (bytes 2508-2515). Real*8.
    pub fn coho_freq(&self) -> Option<f64> {
        let offset = 2508 - 44;
        let bytes = self.data.get(offset..offset + 8)?;
        Some(f64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    // -----------------------------------------------------------------------
    // Site name and elevation (bytes 8360-8395)
    // -----------------------------------------------------------------------

    /// Azimuth boresight correction factor in degrees (bytes 8360-8363).
    pub fn az_correction_factor(&self) -> Option<f32> {
        read_real4(&self.data, 8360 - 44)
    }

    /// Elevation boresight correction factor in degrees (bytes 8364-8367).
    pub fn el_correction_factor(&self) -> Option<f32> {
        read_real4(&self.data, 8364 - 44)
    }

    /// Site name designation (bytes 8368-8371). 4-byte string.
    pub fn site_name(&self) -> Option<String> {
        read_string(&self.data, 8368 - 44, 4)
    }

    /// Minimum elevation angle for antenna manual setup in degrees (bytes 8372-8375).
    /// Two's complement integer value, multiply by 360/2^16 for degrees.
    pub fn ant_manual_setup_iel_min(&self) -> Option<i32> {
        read_sinteger4(&self.data, 8372 - 44)
    }

    /// Maximum elevation angle for antenna manual setup in degrees (bytes 8376-8379).
    pub fn ant_manual_setup_iel_max(&self) -> Option<u32> {
        read_integer4(&self.data, 8376 - 44)
    }

    /// Maximum azimuth velocity for antenna manual setup in deg/s (bytes 8380-8383).
    pub fn ant_manual_setup_fazvelmax(&self) -> Option<u32> {
        read_integer4(&self.data, 8380 - 44)
    }

    /// Maximum elevation velocity for antenna manual setup in deg/s (bytes 8384-8387).
    pub fn ant_manual_setup_felvelmax(&self) -> Option<u32> {
        read_integer4(&self.data, 8384 - 44)
    }

    /// Site ground height above sea level in meters (bytes 8388-8391).
    pub fn site_ground_height(&self) -> Option<u32> {
        read_integer4(&self.data, 8388 - 44)
    }

    /// Site radar height above ground in meters (bytes 8392-8395).
    pub fn site_radar_height(&self) -> Option<u32> {
        read_integer4(&self.data, 8392 - 44)
    }

    // -----------------------------------------------------------------------
    // Additional antenna drive parameters (bytes 8396-8463)
    // -----------------------------------------------------------------------

    /// Azimuth motor positive sustaining drive (bytes 8396-8399).
    pub fn az_pos_sustain_drive(&self) -> Option<f32> {
        read_real4(&self.data, 8396 - 44)
    }

    /// Azimuth motor negative sustaining drive (bytes 8400-8403).
    pub fn az_neg_sustain_drive(&self) -> Option<f32> {
        read_real4(&self.data, 8400 - 44)
    }

    /// Azimuth moment of inertia (bytes 8456-8459).
    pub fn az_inertia(&self) -> Option<f32> {
        read_real4(&self.data, 8456 - 44)
    }

    /// Elevation moment of inertia (bytes 8460-8463).
    pub fn el_inertia(&self) -> Option<f32> {
        read_real4(&self.data, 8460 - 44)
    }

    // -----------------------------------------------------------------------
    // Encoder alignment (bytes 8496-8511)
    // -----------------------------------------------------------------------

    /// Azimuth stow angle for encoder alignment in degrees (bytes 8496-8499).
    pub fn az_stow_angle(&self) -> Option<f32> {
        read_real4(&self.data, 8496 - 44)
    }

    /// Elevation stow angle for encoder alignment in degrees (bytes 8500-8503).
    pub fn el_stow_angle(&self) -> Option<f32> {
        read_real4(&self.data, 8500 - 44)
    }

    // -----------------------------------------------------------------------
    // Vertical receiver noise normalization (bytes 8700-8759)
    // V_RNSCALE(0) through V_RNSCALE(12)
    // -----------------------------------------------------------------------

    /// Vertical receiver noise normalization value for a given elevation sector (0-12).
    /// Bytes 8700-8759, each is a Real*4. Note: V_RNSCALE(11) at bytes 8752-8755 and
    /// V_RNSCALE(12) at bytes 8756-8759.
    pub fn v_rnscale(&self, index: usize) -> Option<f32> {
        if index > 12 {
            return None;
        }
        // V_RNSCALE(0) is at byte 8700 = data offset 8656
        read_real4(&self.data, 8700 - 44 + index * 4)
    }

    // -----------------------------------------------------------------------
    // Doppler and clutter map parameters (bytes 8764-8787)
    // -----------------------------------------------------------------------

    /// Start range for first Doppler radial in km (bytes 8764-8767).
    pub fn doppler_range_start(&self) -> Option<f32> {
        read_real4(&self.data, 8764 - 44)
    }

    /// Maximum index for EL_INDEX parameters (bytes 8768-8771).
    pub fn max_el_index(&self) -> Option<u32> {
        read_integer4(&self.data, 8768 - 44)
    }

    /// Clutter map boundary elevation between segments 2 and 3 in degrees (bytes 8772-8775).
    pub fn seg2lim(&self) -> Option<f32> {
        read_real4(&self.data, 8772 - 44)
    }

    /// Clutter map boundary elevation between segments 3 and 4 in degrees (bytes 8776-8779).
    pub fn seg3lim(&self) -> Option<f32> {
        read_real4(&self.data, 8776 - 44)
    }

    /// Clutter map boundary elevation between segments 4 and 5 in degrees (bytes 8780-8783).
    pub fn seg4lim(&self) -> Option<f32> {
        read_real4(&self.data, 8780 - 44)
    }

    /// Number of elevation segments in ORDA clutter map (bytes 8784-8787).
    pub fn nbr_el_segments(&self) -> Option<u32> {
        read_integer4(&self.data, 8784 - 44)
    }

    // -----------------------------------------------------------------------
    // Noise parameters (bytes 8788-8827)
    // -----------------------------------------------------------------------

    /// Horizontal receiver noise for long pulse in dBm (bytes 8788-8791).
    pub fn h_noise_long(&self) -> Option<f32> {
        read_real4(&self.data, 8788 - 44)
    }

    /// Antenna noise temperature in K (bytes 8792-8795).
    pub fn ant_noise_temp(&self) -> Option<f32> {
        read_real4(&self.data, 8792 - 44)
    }

    /// Horizontal receiver noise for short pulse in dBm (bytes 8796-8799).
    pub fn h_noise_short(&self) -> Option<f32> {
        read_real4(&self.data, 8796 - 44)
    }

    /// Horizontal receiver noise tolerance in dB (bytes 8800-8803).
    pub fn h_noise_tolerance(&self) -> Option<f32> {
        read_real4(&self.data, 8800 - 44)
    }

    /// Minimum horizontal dynamic range in dB (bytes 8804-8807).
    pub fn min_h_dyn_range(&self) -> Option<f32> {
        read_real4(&self.data, 8804 - 44)
    }

    /// Vertical receiver noise tolerance in dB (bytes 8816-8819).
    pub fn v_noise_tolerance(&self) -> Option<f32> {
        read_real4(&self.data, 8816 - 44)
    }

    /// Minimum vertical dynamic range in dB (bytes 8820-8823).
    pub fn min_v_dyn_range(&self) -> Option<f32> {
        read_real4(&self.data, 8820 - 44)
    }

    // -----------------------------------------------------------------------
    // ZDR and dual-pol parameters (bytes 8824-8863)
    // -----------------------------------------------------------------------

    /// System differential reflectivity offset degrade limit in dB (bytes 8824-8827).
    pub fn zdr_offset_dgrad_lim(&self) -> Option<f32> {
        read_real4(&self.data, 8824 - 44)
    }

    /// Baseline system differential reflectivity offset in dB (bytes 8828-8843). Real*4 (16 bytes).
    pub fn baseline_zdr_offset(&self) -> Option<f32> {
        read_real4(&self.data, 8828 - 44)
    }

    /// Vertical receiver noise for long pulse in dBm (bytes 8844-8847).
    pub fn v_noise_long(&self) -> Option<f32> {
        read_real4(&self.data, 8844 - 44)
    }

    /// Vertical receiver noise for short pulse in dBm (bytes 8848-8851).
    pub fn v_noise_short(&self) -> Option<f32> {
        read_real4(&self.data, 8848 - 44)
    }

    /// ZDR unfolding overlay threshold in dB (bytes 8852-8855).
    pub fn zdr_data_tover(&self) -> Option<f32> {
        read_real4(&self.data, 8852 - 44)
    }

    /// PHI unfolding overlay threshold in dB (bytes 8856-8859).
    pub fn phi_data_tover(&self) -> Option<f32> {
        read_real4(&self.data, 8856 - 44)
    }

    /// RHO unfolding overlay threshold in dB (bytes 8860-8863).
    pub fn rho_data_tover(&self) -> Option<f32> {
        read_real4(&self.data, 8860 - 44)
    }

    // -----------------------------------------------------------------------
    // STALO power and power sense (bytes 8864-8895)
    // -----------------------------------------------------------------------

    /// STALO power degrade limit in V (bytes 8864-8867).
    pub fn stalo_power_dgrad_limit(&self) -> Option<f32> {
        read_real4(&self.data, 8864 - 44)
    }

    /// STALO power maintenance limit in V (bytes 8868-8871).
    pub fn stalo_power_maint_limit(&self) -> Option<f32> {
        read_real4(&self.data, 8868 - 44)
    }

    /// Minimum horizontal power sense in dBm (bytes 8872-8875).
    pub fn min_h_pwr_sense(&self) -> Option<f32> {
        read_real4(&self.data, 8872 - 44)
    }

    /// Minimum vertical power sense in dBm (bytes 8876-8879).
    pub fn min_v_pwr_sense(&self) -> Option<f32> {
        read_real4(&self.data, 8876 - 44)
    }

    /// Horizontal power sense calibration offset in dB (bytes 8880-8883).
    pub fn h_pwr_sense_offset(&self) -> Option<f32> {
        read_real4(&self.data, 8880 - 44)
    }

    /// Vertical power sense calibration offset in dB (bytes 8884-8887).
    pub fn v_pwr_sense_offset(&self) -> Option<f32> {
        read_real4(&self.data, 8884 - 44)
    }

    /// Power sense gain reference value in dB (bytes 8888-8891).
    pub fn ps_gain_ref(&self) -> Option<f32> {
        read_real4(&self.data, 8888 - 44)
    }

    /// RF pallet broadband loss in dB (bytes 8892-8895).
    pub fn rf_pallet_broad_loss(&self) -> Option<f32> {
        read_real4(&self.data, 8892 - 44)
    }

    // -----------------------------------------------------------------------
    // AME tolerance and temperature limits (bytes 8960-9047)
    // -----------------------------------------------------------------------

    /// AME power supply tolerance in % (bytes 8960-8963).
    pub fn ame_ps_tolerance(&self) -> Option<f32> {
        read_real4(&self.data, 8960 - 44)
    }

    /// Maximum AME internal alarm temperature in deg C (bytes 8964-8967).
    pub fn ame_max_temp(&self) -> Option<f32> {
        read_real4(&self.data, 8964 - 44)
    }

    /// Minimum AME internal alarm temperature in deg C (bytes 8968-8971).
    pub fn ame_min_temp(&self) -> Option<f32> {
        read_real4(&self.data, 8968 - 44)
    }

    /// Maximum AME receiver module alarm temperature in deg C (bytes 8972-8975).
    pub fn rcvr_mod_max_temp(&self) -> Option<f32> {
        read_real4(&self.data, 8972 - 44)
    }

    /// Minimum AME receiver module alarm temperature in deg C (bytes 8976-8979).
    pub fn rcvr_mod_min_temp(&self) -> Option<f32> {
        read_real4(&self.data, 8976 - 44)
    }

    /// Maximum AME BITE module alarm temperature in deg C (bytes 8980-8983).
    pub fn bite_mod_max_temp(&self) -> Option<f32> {
        read_real4(&self.data, 8980 - 44)
    }

    /// Minimum AME BITE module alarm temperature in deg C (bytes 8984-8987).
    pub fn bite_mod_min_temp(&self) -> Option<f32> {
        read_real4(&self.data, 8984 - 44)
    }

    /// Default (H+V) microwave assembly phase shifter position (bytes 8988-8991).
    pub fn default_polarization(&self) -> Option<u32> {
        read_integer4(&self.data, 8988 - 44)
    }

    // -----------------------------------------------------------------------
    // TR limiter and stepper motor (bytes 8992-9003)
    // -----------------------------------------------------------------------

    /// TR limiter degrade limit in V (bytes 8992-8995).
    pub fn tr_limit_dgrad_limit(&self) -> Option<f32> {
        read_real4(&self.data, 8992 - 44)
    }

    /// TR limiter failure limit in V (bytes 8996-8999).
    pub fn tr_limit_fail_limit(&self) -> Option<f32> {
        read_real4(&self.data, 8996 - 44)
    }

    /// Whether RF pallets stepper motor is enabled, "T" or "F" (bytes 9000-9003).
    pub fn rfp_stepper_enabled(&self) -> Option<String> {
        read_string(&self.data, 9000 - 44, 4)
    }

    // -----------------------------------------------------------------------
    // AME current tolerance (bytes 9008-9011)
    // -----------------------------------------------------------------------

    /// AME peltier current tolerance in % (bytes 9008-9011).
    pub fn ame_current_tolerance(&self) -> Option<f32> {
        read_real4(&self.data, 9008 - 44)
    }

    // -----------------------------------------------------------------------
    // Polarization positions (bytes 9012-9019)
    // -----------------------------------------------------------------------

    /// Horizontal (H only) microwave assembly phase shifter position (bytes 9012-9015).
    pub fn h_only_polarization(&self) -> Option<u32> {
        read_integer4(&self.data, 9012 - 44)
    }

    /// Vertical (V only) microwave assembly phase shifter position (bytes 9016-9019).
    pub fn v_only_polarization(&self) -> Option<u32> {
        read_integer4(&self.data, 9016 - 44)
    }

    // -----------------------------------------------------------------------
    // Sun bias and shelter warning (bytes 9028-9035)
    // -----------------------------------------------------------------------

    /// Sun measurement bias in dB (bytes 9028-9031).
    pub fn sun_bias(&self) -> Option<f32> {
        read_real4(&self.data, 9028 - 44)
    }

    /// Low equipment shelter temperature warning limit in deg C (bytes 9032-9035).
    pub fn a_min_shelter_temp_warn(&self) -> Option<f32> {
        read_real4(&self.data, 9032 - 44)
    }

    // -----------------------------------------------------------------------
    // Power meter and TXB (bytes 9036-9047)
    // -----------------------------------------------------------------------

    /// Power meter 0 bias voltage in V (bytes 9036-9039).
    pub fn power_meter_zero(&self) -> Option<f32> {
        read_real4(&self.data, 9036 - 44)
    }

    /// Expected TXB (transmit bias) between H and V channels in dB (bytes 9040-9043).
    pub fn txb_baseline(&self) -> Option<f32> {
        read_real4(&self.data, 9040 - 44)
    }

    /// TXB alarm threshold in dB (bytes 9044-9047).
    pub fn txb_alarm_thresh(&self) -> Option<f32> {
        read_real4(&self.data, 9044 - 44)
    }

    // -----------------------------------------------------------------------
    // Raw data access
    // -----------------------------------------------------------------------

    /// Returns the total number of bytes in the adaptation data (header + data).
    pub fn total_size(&self) -> usize {
        44 + self.data.len()
    }

    /// Returns the raw data bytes after the 44-byte header.
    pub fn raw_data(&self) -> &[u8] {
        &self.data
    }

    /// Read a Raw Real*4 (big-endian f32) at a given absolute ICD byte offset.
    /// Returns None if the offset is out of bounds or within the header (< 44).
    pub fn read_real4_at(&self, icd_byte_offset: usize) -> Option<f32> {
        if icd_byte_offset < 44 {
            return None;
        }
        read_real4(&self.data, icd_byte_offset - 44)
    }

    /// Read a Raw Integer*4 (big-endian u32) at a given absolute ICD byte offset.
    /// Returns None if the offset is out of bounds or within the header (< 44).
    pub fn read_integer4_at(&self, icd_byte_offset: usize) -> Option<u32> {
        if icd_byte_offset < 44 {
            return None;
        }
        read_integer4(&self.data, icd_byte_offset - 44)
    }

    /// Read a null-terminated string at a given absolute ICD byte offset with specified length.
    /// Returns None if the offset is out of bounds or within the header (< 44).
    pub fn read_string_at(&self, icd_byte_offset: usize, len: usize) -> Option<String> {
        if icd_byte_offset < 44 {
            return None;
        }
        read_string(&self.data, icd_byte_offset - 44, len)
    }

    /// Convert this message to an owned version with `'static` lifetime.
    pub fn into_owned(self) -> Message<'static> {
        Message {
            header: Cow::Owned(self.header.into_owned()),
            data: self.data,
        }
    }
}

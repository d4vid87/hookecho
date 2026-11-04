use crate::messages::primitive_aliases::{Code2, Integer2, Integer4, Real4, SInteger2};
use zerocopy::{FromBytes, Immutable, KnownLayout};

/// 480 halfwords * 2 bytes = 960 bytes.
const _: () = assert!(size_of::<Message>() == 960);

/// The raw Performance/Maintenance Data message (Message Type 3).
///
/// This is a fixed-size message of 480 halfwords (960 bytes) containing detailed
/// performance and maintenance information about the RDA system, organized into
/// sections covering communications, AME, power, transmitter, tower/utilities,
/// antenna/pedestal, RF generator/receiver, calibration, file status, RSP/CPU
/// status, and device status.
#[repr(C)]
#[derive(Clone, PartialEq, Debug, FromBytes, Immutable, KnownLayout)]
pub struct Message {
    // ==================== Communications (HW 1-57) ====================
    /// HW 1: Spare
    pub comms_spare_1: Integer2,

    /// HW 2: Loop back test status.
    /// Values: 0=Pass, 1=Fail, 2=Timeout, 3=Not Tested
    pub loop_back_test_status: Integer2,

    /// HW 3-4: T1 output frames.
    pub t1_output_frames: Integer4,

    /// HW 5-6: T1 input frames.
    pub t1_input_frames: Integer4,

    /// HW 7-8: Router memory used (bytes).
    pub router_memory_used: Integer4,

    /// HW 9-10: Router memory free (bytes).
    pub router_memory_free: Integer4,

    /// HW 11: Router memory utilization (%).
    pub router_memory_utilization: Integer2,

    /// HW 12: Route to RPG.
    /// Values: 0=Normal, 1=Backup in Use, 2=Down Failure, 3=Backup Commanded Down,
    /// 4=Not Installed
    pub route_to_rpg: Integer2,

    /// HW 13: T1 port status.
    /// Values: 1=Up, 2=Down, 3=Test
    pub t1_port_status: Integer2,

    /// HW 14: Router dedicated Ethernet port status.
    /// Values: 1=Up, 2=Down, 3=Test
    pub router_dedicated_ethernet_port_status: Integer2,

    /// HW 15: Router commercial Ethernet port status.
    /// Values: 1=Up, 2=Down, 3=Test
    pub router_commercial_ethernet_port_status: Integer2,

    /// HW 16-20: Spare.
    pub comms_spare_16_20: [Integer2; 5],

    /// HW 21-22: CSU 24-hour errored seconds.
    pub csu_24hr_errored_seconds: Integer4,

    /// HW 23-24: CSU 24-hour severely errored seconds.
    pub csu_24hr_severely_errored_seconds: Integer4,

    /// HW 25-26: CSU 24-hour severely errored framing seconds.
    pub csu_24hr_severely_errored_framing_seconds: Integer4,

    /// HW 27-28: CSU 24-hour unavailable seconds.
    pub csu_24hr_unavailable_seconds: Integer4,

    /// HW 29-30: CSU 24-hour controlled slip seconds.
    pub csu_24hr_controlled_slip_seconds: Integer4,

    /// HW 31-32: CSU 24-hour path coding violations.
    pub csu_24hr_path_coding_violations: Integer4,

    /// HW 33-34: CSU 24-hour line errored seconds.
    pub csu_24hr_line_errored_seconds: Integer4,

    /// HW 35-36: CSU 24-hour bursty errored seconds.
    pub csu_24hr_bursty_errored_seconds: Integer4,

    /// HW 37-38: CSU 24-hour degraded minutes.
    pub csu_24hr_degraded_minutes: Integer4,

    /// HW 39-40: Spare.
    pub comms_spare_39_40: Integer4,

    /// HW 41-42: LAN switch CPU utilization (%).
    pub lan_switch_cpu_utilization: Integer4,

    /// HW 43: LAN switch memory utilization (%).
    pub lan_switch_memory_utilization: Integer2,

    /// HW 44: Spare.
    pub comms_spare_44: Integer2,

    /// HW 45: IFDR chassis temperature (deg C).
    pub ifdr_chassis_temperature: SInteger2,

    /// HW 46: IFDR FPGA temperature (deg C).
    pub ifdr_fpga_temperature: SInteger2,

    /// HW 47: NTP status.
    /// Values: 0=OK, 1=Fail
    pub ntp_status: Integer2,

    /// HW 48-52: Spare.
    pub comms_spare_48_52: [Integer2; 5],

    /// HW 53: IPC status.
    /// Values: 0=OK, 1=Fail, 2=N/A
    pub ipc_status: Integer2,

    /// HW 54: Commanded channel control.
    /// Values: 0=N/A, 1=Channel 1, 2=Channel 2
    pub commanded_channel_control: Integer2,

    /// HW 55-57: Spare.
    pub comms_spare_55_57: [Integer2; 3],

    // ==================== AME (HW 58-110) ====================
    /// HW 58: Polarization.
    /// Values: 0=H Only, 1=H+V, 2=V Only
    pub polarization: Integer2,

    /// HW 59-60: AME internal temperature (deg C).
    pub ame_internal_temperature: Real4,

    /// HW 61-62: AME receiver module temperature (deg C).
    pub ame_receiver_module_temperature: Real4,

    /// HW 63-64: AME BITE/CAL module temperature (deg C).
    pub ame_bite_cal_module_temperature: Real4,

    /// HW 65: AME Peltier pulse width modulation (%).
    pub ame_peltier_pulse_width_modulation: Integer2,

    /// HW 66: AME Peltier status.
    /// Values: 0=OFF, 1=ON
    pub ame_peltier_status: Integer2,

    /// HW 67: AME A/D converter status.
    /// Values: 0=OK, 1=FAIL
    pub ame_ad_converter_status: Integer2,

    /// HW 68: AME state.
    /// Values: 0=START, 1=RUNNING, 2=FLASH, 3=ERROR
    pub ame_state: Integer2,

    /// HW 69-70: AME 3.3V PS voltage (V).
    pub ame_3_3v_ps_voltage: Real4,

    /// HW 71-72: AME 5V PS voltage (V).
    pub ame_5v_ps_voltage: Real4,

    /// HW 73-74: AME 6.5V PS voltage (V).
    pub ame_6_5v_ps_voltage: Real4,

    /// HW 75-76: AME 15V PS voltage (V).
    pub ame_15v_ps_voltage: Real4,

    /// HW 77-78: AME 48V PS voltage (V).
    pub ame_48v_ps_voltage: Real4,

    /// HW 79-80: AME STALO power (V).
    pub ame_stalo_power: Real4,

    /// HW 81-82: Peltier current (A).
    pub peltier_current: Real4,

    /// HW 83-84: ADC calibration reference voltage (V).
    pub adc_calibration_reference_voltage: Real4,

    /// HW 85: AME mode.
    /// Values: 0=READY, 1=MAINTENANCE
    pub ame_mode: Integer2,

    /// HW 86: AME Peltier mode.
    /// Values: 0=COOL, 1=HEAT
    pub ame_peltier_mode: Integer2,

    /// HW 87-88: AME Peltier inside fan current (A).
    pub ame_peltier_inside_fan_current: Real4,

    /// HW 89-90: AME Peltier outside fan current (A).
    pub ame_peltier_outside_fan_current: Real4,

    /// HW 91-92: Horizontal TR limiter voltage (V).
    pub horizontal_tr_limiter_voltage: Real4,

    /// HW 93-94: Vertical TR limiter voltage (V).
    pub vertical_tr_limiter_voltage: Real4,

    /// HW 95-96: ADC calibration offset voltage (mV).
    pub adc_calibration_offset_voltage: Real4,

    /// HW 97-98: ADC calibration gain correction.
    pub adc_calibration_gain_correction: Real4,

    /// HW 99: RCP status.
    /// Values: 0=RCP OK, 1=NOT OK
    pub rcp_status: Integer2,

    /// HW 100-107: RCP string (16 bytes).
    pub rcp_string: [u8; 16],

    /// HW 108: SPIP power buttons (bitfield).
    pub spip_power_buttons: Code2,

    /// HW 109-110: Spare.
    pub ame_spare_109_110: [Integer2; 2],

    // ==================== Power (HW 111-136) ====================
    /// HW 111-112: Master power administrator load (A).
    pub master_power_administrator_load: Real4,

    /// HW 113-114: Expansion power administrator load (A).
    pub expansion_power_administrator_load: Real4,

    /// HW 115-136: Spare.
    pub power_spare_115_136: [Integer2; 22],

    // ==================== Transmitter (HW 137-228) ====================
    /// HW 137: +5 VDC PS.
    pub plus_5_vdc_ps: Integer2,

    /// HW 138: +15 VDC PS.
    pub plus_15_vdc_ps: Integer2,

    /// HW 139: +28 VDC PS.
    pub plus_28_vdc_ps: Integer2,

    /// HW 140: -15 VDC PS.
    pub minus_15_vdc_ps: Integer2,

    /// HW 141: +45 VDC PS.
    pub plus_45_vdc_ps: Integer2,

    /// HW 142: Filament PS voltage.
    pub filament_ps_voltage: Integer2,

    /// HW 143: Vacuum pump PS voltage.
    pub vacuum_pump_ps_voltage: Integer2,

    /// HW 144: Focus coil PS voltage.
    pub focus_coil_ps_voltage: Integer2,

    /// HW 145: Filament PS.
    /// Values: 0=On, 1=Off
    pub filament_ps: Integer2,

    /// HW 146: Klystron warmup.
    /// Values: 0=Normal, 1=Preheat
    pub klystron_warmup: Integer2,

    /// HW 147: Transmitter available.
    /// Values: 0=Yes, 1=No
    pub transmitter_available: Integer2,

    /// HW 148: WG switch position.
    /// Values: 0=Antenna, 1=Dummy Load
    pub wg_switch_position: Integer2,

    /// HW 149: WG/PFN transfer interlock.
    /// Values: 0=OK, 1=Open
    pub wg_pfn_transfer_interlock: Integer2,

    /// HW 150: Maintenance mode.
    /// Values: 0=No, 1=Yes
    pub maintenance_mode: Integer2,

    /// HW 151: Maintenance required.
    /// Values: 0=No, 1=Required
    pub maintenance_required: Integer2,

    /// HW 152: PFN switch position.
    /// Values: 0=Short Pulse, 1=Long Pulse
    pub pfn_switch_position: Integer2,

    /// HW 153: Modulator overload.
    pub modulator_overload: Integer2,

    /// HW 154: Modulator inverter current.
    pub modulator_inv_current: Integer2,

    /// HW 155: Modulator switch fail.
    pub modulator_switch_fail: Integer2,

    /// HW 156: Main power voltage.
    pub main_power_voltage: Integer2,

    /// HW 157: Charging system fail.
    pub charging_system_fail: Integer2,

    /// HW 158: Inverse diode current.
    pub inverse_diode_current: Integer2,

    /// HW 159: Trigger amplifier.
    pub trigger_amplifier: Integer2,

    /// HW 160: Circulator temperature.
    pub circulator_temperature: Integer2,

    /// HW 161: Spectrum filter pressure.
    pub spectrum_filter_pressure: Integer2,

    /// HW 162: WG arc/VSWR.
    pub wg_arc_vswr: Integer2,

    /// HW 163: Cabinet interlock.
    pub cabinet_interlock: Integer2,

    /// HW 164: Cabinet air temperature.
    pub cabinet_air_temperature: Integer2,

    /// HW 165: Cabinet airflow.
    pub cabinet_airflow: Integer2,

    /// HW 166: Klystron current.
    pub klystron_current: Integer2,

    /// HW 167: Klystron filament current.
    pub klystron_filament_current: Integer2,

    /// HW 168: Klystron vacion current.
    pub klystron_vacion_current: Integer2,

    /// HW 169: Klystron air temperature.
    pub klystron_air_temperature: Integer2,

    /// HW 170: Klystron airflow.
    pub klystron_airflow: Integer2,

    /// HW 171: Modulator switch maintenance.
    pub modulator_switch_maintenance: Integer2,

    /// HW 172: Post charge regulator maintenance.
    pub post_charge_regulator_maintenance: Integer2,

    /// HW 173: WG pressure/humidity.
    pub wg_pressure_humidity: Integer2,

    /// HW 174: Transmitter overvoltage.
    pub transmitter_overvoltage: Integer2,

    /// HW 175: Transmitter overcurrent.
    pub transmitter_overcurrent: Integer2,

    /// HW 176: Focus coil current.
    pub focus_coil_current: Integer2,

    /// HW 177: Focus coil airflow.
    pub focus_coil_airflow: Integer2,

    /// HW 178: Oil temperature.
    pub oil_temperature: Integer2,

    /// HW 179: PRF limit.
    pub prf_limit: Integer2,

    /// HW 180: Transmitter oil level.
    pub transmitter_oil_level: Integer2,

    /// HW 181: Transmitter battery charging.
    /// Values: 0=Yes, 1=No
    pub transmitter_battery_charging: Integer2,

    /// HW 182: High voltage status.
    /// Values: 0=On, 1=Off
    pub high_voltage_status: Integer2,

    /// HW 183: Transmitter recycling summary.
    /// Values: 0=Normal, 1=Recycling
    pub transmitter_recycling_summary: Integer2,

    /// HW 184: Transmitter inoperable.
    pub transmitter_inoperable: Integer2,

    /// HW 185: Transmitter air filter.
    /// Values: 0=Dirty, 1=OK
    pub transmitter_air_filter: Integer2,

    /// HW 186-193: Zero test bits 0-7.
    pub zero_test_bits: [Integer2; 8],

    /// HW 194-201: One test bits 0-7.
    pub one_test_bits: [Integer2; 8],

    /// HW 202: Transmitter SPIP interface.
    pub xmtr_spip_interface: Integer2,

    /// HW 203: Transmitter summary status.
    /// Values: 0=Ready, 1=Alarm, 2=Maintenance, 3=Recycle, 4=Preheat
    pub transmitter_summary_status: Integer2,

    /// HW 204: Spare.
    pub xmtr_spare_204: Integer2,

    /// HW 205-206: Transmitter RF power sensor (mW).
    pub transmitter_rf_power_sensor: Real4,

    /// HW 207-208: Horizontal transmitter peak power (kW).
    pub horizontal_xmtr_peak_power: Real4,

    /// HW 209-210: Transmitter peak power (kW).
    pub xmtr_peak_power: Real4,

    /// HW 211-212: Vertical transmitter peak power (kW).
    pub vertical_xmtr_peak_power: Real4,

    /// HW 213-214: Transmitter RF average power (W).
    pub xmtr_rf_avg_power: Real4,

    /// HW 215-216: Spare.
    pub xmtr_spare_215_216: Integer4,

    /// HW 217-218: Transmitter recycle count.
    pub xmtr_recycle_count: Integer4,

    /// HW 219-220: Receiver bias measurement (dB).
    pub receiver_bias_measurement: Real4,

    /// HW 221-222: Transmit imbalance (dB).
    pub transmit_imbalance: Real4,

    /// HW 223-224: Transmitter power meter zero (V).
    pub xmtr_power_meter_zero: Real4,

    /// HW 225-228: Spare.
    pub xmtr_spare_225_228: [Integer2; 4],

    // ==================== Tower/Utilities (HW 229-299) ====================
    /// HW 229: AC unit 1 compressor shut off.
    pub ac_unit_1_compressor_shut_off: Integer2,

    /// HW 230: AC unit 2 compressor shut off.
    pub ac_unit_2_compressor_shut_off: Integer2,

    /// HW 231: Generator maintenance required.
    pub generator_maintenance_required: Integer2,

    /// HW 232: Generator battery voltage.
    pub generator_battery_voltage: Integer2,

    /// HW 233: Generator engine.
    pub generator_engine: Integer2,

    /// HW 234: Generator volt/frequency.
    pub generator_volt_frequency: Integer2,

    /// HW 235: Power source.
    /// Values: 0=Utility, 1=Generator
    pub power_source: Integer2,

    /// HW 236: Transitional power source.
    /// Values: 0=OK, 1=Off
    pub transitional_power_source: Integer2,

    /// HW 237: Generator auto run off switch.
    /// Values: 0=Manual, 1=Auto
    pub generator_auto_run_off_switch: Integer2,

    /// HW 238: Aircraft hazard lighting.
    pub aircraft_hazard_lighting: Integer2,

    /// HW 239-249: Spare.
    pub tower_spare_239_249: [Integer2; 11],

    /// HW 250: Equipment shelter fire detection system.
    pub equipment_shelter_fire_detection_system: Integer2,

    /// HW 251: Equipment shelter fire/smoke.
    pub equipment_shelter_fire_smoke: Integer2,

    /// HW 252: Generator shelter fire/smoke.
    pub generator_shelter_fire_smoke: Integer2,

    /// HW 253: Utility voltage/frequency.
    pub utility_voltage_frequency: Integer2,

    /// HW 254: Site security alarm.
    pub site_security_alarm: Integer2,

    /// HW 255: Security equipment.
    pub security_equipment: Integer2,

    /// HW 256: Security system.
    pub security_system: Integer2,

    /// HW 257: Receiver connected to antenna.
    /// Values: 0=Connected, 1=Not Connected, 2=N/A
    pub receiver_connected_to_antenna: Integer2,

    /// HW 258: Radome hatch.
    /// Values: 0=Open, 1=Closed
    pub radome_hatch: Integer2,

    /// HW 259: AC unit 1 filter dirty.
    pub ac_unit_1_filter_dirty: Integer2,

    /// HW 260: AC unit 2 filter dirty.
    pub ac_unit_2_filter_dirty: Integer2,

    /// HW 261-262: Equipment shelter temperature (deg C).
    pub equipment_shelter_temperature: Real4,

    /// HW 263-264: Outside ambient temperature (deg C).
    pub outside_ambient_temperature: Real4,

    /// HW 265-266: Transmitter leaving air temperature (deg C).
    pub transmitter_leaving_air_temp: Real4,

    /// HW 267-268: AC unit 1 discharge air temperature (deg C).
    pub ac_unit_1_discharge_air_temp: Real4,

    /// HW 269-270: Generator shelter temperature (deg C).
    pub generator_shelter_temperature: Real4,

    /// HW 271-272: Radome air temperature (deg C).
    pub radome_air_temperature: Real4,

    /// HW 273-274: AC unit 2 discharge air temperature (deg C).
    pub ac_unit_2_discharge_air_temp: Real4,

    /// HW 275-276: SPIP +15V PS (V).
    pub spip_15v_ps: Real4,

    /// HW 277-278: SPIP -15V PS (V).
    pub spip_neg_15v_ps: Real4,

    /// HW 279: SPIP 28V PS status.
    /// Values: 0=Fail, 1=OK
    pub spip_28v_ps_status: Integer2,

    /// HW 280: Spare.
    pub tower_spare_280: Integer2,

    /// HW 281-282: SPIP 5V PS (V).
    pub spip_5v_ps: Real4,

    /// HW 283: Converted generator fuel level (%).
    pub converted_generator_fuel_level: Integer2,

    /// HW 284-299: Spare.
    pub tower_spare_284_299: [Integer2; 16],

    // ==================== Antenna/Pedestal (HW 300-340) ====================
    /// HW 300: Elevation +dead limit.
    pub elevation_plus_dead_limit: Integer2,

    /// HW 301: +150V overvoltage.
    pub plus_150v_overvoltage: Integer2,

    /// HW 302: +150V undervoltage.
    pub plus_150v_undervoltage: Integer2,

    /// HW 303: Elevation servo amp inhibit.
    pub elevation_servo_amp_inhibit: Integer2,

    /// HW 304: Elevation servo amp short circuit.
    pub elevation_servo_amp_short_circuit: Integer2,

    /// HW 305: Elevation servo amp overtemp.
    pub elevation_servo_amp_overtemp: Integer2,

    /// HW 306: Elevation motor overtemp.
    pub elevation_motor_overtemp: Integer2,

    /// HW 307: Elevation stow pin.
    pub elevation_stow_pin: Integer2,

    /// HW 308: Elevation housing 5V PS.
    pub elevation_housing_5v_ps: Integer2,

    /// HW 309: Elevation -dead limit.
    pub elevation_minus_dead_limit: Integer2,

    /// HW 310: Elevation +normal limit.
    pub elevation_plus_normal_limit: Integer2,

    /// HW 311: Elevation -normal limit.
    pub elevation_minus_normal_limit: Integer2,

    /// HW 312: Elevation encoder light.
    pub elevation_encoder_light: Integer2,

    /// HW 313: Elevation gearbox oil.
    pub elevation_gearbox_oil: Integer2,

    /// HW 314: Elevation handwheel.
    pub elevation_handwheel: Integer2,

    /// HW 315: Elevation amp PS.
    pub elevation_amp_ps: Integer2,

    /// HW 316: Azimuth servo amp inhibit.
    pub azimuth_servo_amp_inhibit: Integer2,

    /// HW 317: Azimuth servo amp short circuit.
    pub azimuth_servo_amp_short_circuit: Integer2,

    /// HW 318: Azimuth servo amp overtemp.
    pub azimuth_servo_amp_overtemp: Integer2,

    /// HW 319: Azimuth motor overtemp.
    pub azimuth_motor_overtemp: Integer2,

    /// HW 320: Azimuth stow pin.
    pub azimuth_stow_pin: Integer2,

    /// HW 321: Azimuth housing 5V PS.
    pub azimuth_housing_5v_ps: Integer2,

    /// HW 322: Azimuth encoder light.
    pub azimuth_encoder_light: Integer2,

    /// HW 323: Azimuth gearbox oil.
    pub azimuth_gearbox_oil: Integer2,

    /// HW 324: Azimuth bull gear oil.
    pub azimuth_bull_gear_oil: Integer2,

    /// HW 325: Azimuth handwheel.
    pub azimuth_handwheel: Integer2,

    /// HW 326: Azimuth servo amp PS.
    pub azimuth_servo_amp_ps: Integer2,

    /// HW 327: Servo.
    /// Values: 0=On, 1=Off
    pub servo: Integer2,

    /// HW 328: Pedestal interlock switch.
    /// Values: 0=Operational, 1=Safe
    pub pedestal_interlock_switch: Integer2,

    /// HW 329-340: Spare.
    pub pedestal_spare_329_340: [Integer2; 12],

    // ==================== RF Generator/Receiver (HW 341-362) ====================
    /// HW 341: COHO/clock.
    pub coho_clock: Integer2,

    /// HW 342: RF generator frequency select oscillator.
    pub rf_generator_frequency_select_oscillator: Integer2,

    /// HW 343: RF generator RF STALO.
    pub rf_generator_rf_stalo: Integer2,

    /// HW 344: RF generator phase-shifted COHO.
    pub rf_generator_phase_shifted_coho: Integer2,

    /// HW 345: +9V receiver PS.
    pub plus_9v_receiver_ps: Integer2,

    /// HW 346: +5V receiver PS.
    pub plus_5v_receiver_ps: Integer2,

    /// HW 347: +/-18V receiver PS.
    pub plus_or_minus_18v_receiver_ps: Integer2,

    /// HW 348: -9V receiver PS.
    pub minus_9v_receiver_ps: Integer2,

    /// HW 349: +5V single channel RDAIU PS.
    pub plus_5v_single_channel_rdaiu_ps: Integer2,

    /// HW 350: Spare.
    pub rf_spare_350: Integer2,

    /// HW 351-352: Horizontal short pulse noise (dBm).
    pub horizontal_short_pulse_noise: Real4,

    /// HW 353-354: Horizontal long pulse noise (dBm).
    pub horizontal_long_pulse_noise: Real4,

    /// HW 355-356: Horizontal noise temperature (K).
    pub horizontal_noise_temperature: Real4,

    /// HW 357-358: Vertical short pulse noise (dBm).
    pub vertical_short_pulse_noise: Real4,

    /// HW 359-360: Vertical long pulse noise (dBm).
    pub vertical_long_pulse_noise: Real4,

    /// HW 361-362: Vertical noise temperature (K).
    pub vertical_noise_temperature: Real4,

    // ==================== Calibration (HW 363-430) ====================
    /// HW 363-364: Horizontal linearity.
    pub horizontal_linearity: Real4,

    /// HW 365-366: Horizontal dynamic range (dB).
    pub horizontal_dynamic_range: Real4,

    /// HW 367-368: Horizontal delta dBZ0 (dB).
    pub horizontal_delta_dbz0: Real4,

    /// HW 369-370: Vertical delta dBZ0 (dB).
    pub vertical_delta_dbz0: Real4,

    /// HW 371-372: KD peak measured (dBm).
    pub kd_peak_measured: Real4,

    /// HW 373-374: Spare.
    pub cal_spare_373_374: Integer4,

    /// HW 375-376: Short pulse horizontal dBZ0 (dBZ).
    pub short_pulse_horizontal_dbz0: Real4,

    /// HW 377-378: Long pulse horizontal dBZ0 (dBZ).
    pub long_pulse_horizontal_dbz0: Real4,

    /// HW 379: Velocity processed.
    /// Values: 0=Good, 1=Fail
    pub velocity_processed: Integer2,

    /// HW 380: Width processed.
    pub width_processed: Integer2,

    /// HW 381: Velocity RF gen.
    pub velocity_rf_gen: Integer2,

    /// HW 382: Width RF gen.
    pub width_rf_gen: Integer2,

    /// HW 383-384: Horizontal I0 (dBm).
    pub horizontal_i0: Real4,

    /// HW 385-386: Vertical I0 (dBm).
    pub vertical_i0: Real4,

    /// HW 387-388: Vertical dynamic range (dB).
    pub vertical_dynamic_range: Real4,

    /// HW 389-390: Short pulse vertical dBZ0 (dBZ).
    pub short_pulse_vertical_dbz0: Real4,

    /// HW 391-392: Long pulse vertical dBZ0 (dBZ).
    pub long_pulse_vertical_dbz0: Real4,

    /// HW 393-394: Spare.
    pub cal_spare_393_394: Integer4,

    /// HW 395-396: Spare.
    pub cal_spare_395_396: Integer4,

    /// HW 397-398: Horizontal power sense (dBm).
    pub horizontal_power_sense: Real4,

    /// HW 399-400: Vertical power sense (dBm).
    pub vertical_power_sense: Real4,

    /// HW 401-402: ZDR offset (dB).
    pub zdr_offset: Real4,

    /// HW 403-408: Spare.
    pub cal_spare_403_408: [Integer2; 6],

    /// HW 409-410: Clutter suppression delta (dB).
    pub clutter_suppression_delta: Real4,

    /// HW 411-412: Clutter suppression unfiltered power (dBZ).
    pub clutter_suppression_unfiltered_power: Real4,

    /// HW 413-414: Clutter suppression filtered power (dBZ).
    pub clutter_suppression_filtered_power: Real4,

    /// HW 415-416: Spare.
    pub cal_spare_415_416: Integer4,

    /// HW 417-418: Spare.
    pub cal_spare_417_418: Integer4,

    /// HW 419-422: Spare.
    pub cal_spare_419_422: [Integer2; 4],

    /// HW 423-424: Spare.
    pub cal_spare_423_424: Integer4,

    /// HW 425-426: Vertical linearity.
    pub vertical_linearity: Real4,

    /// HW 427-430: Spare.
    pub cal_spare_427_430: [Integer2; 4],

    // ==================== File Status (HW 431-447) ====================
    /// HW 431: State file read status.
    pub state_file_read_status: Integer2,

    /// HW 432: State file write status.
    pub state_file_write_status: Integer2,

    /// HW 433: Bypass map file read status.
    pub bypass_map_file_read_status: Integer2,

    /// HW 434: Bypass map file write status.
    pub bypass_map_file_write_status: Integer2,

    /// HW 435: Spare.
    pub file_spare_435: Integer2,

    /// HW 436: Spare.
    pub file_spare_436: Integer2,

    /// HW 437: Current adaptation file read status.
    pub current_adaptation_file_read_status: Integer2,

    /// HW 438: Current adaptation file write status.
    pub current_adaptation_file_write_status: Integer2,

    /// HW 439: Censor zone file read status.
    pub censor_zone_file_read_status: Integer2,

    /// HW 440: Censor zone file write status.
    pub censor_zone_file_write_status: Integer2,

    /// HW 441: Remote VCP file read status.
    pub remote_vcp_file_read_status: Integer2,

    /// HW 442: Remote VCP file write status.
    pub remote_vcp_file_write_status: Integer2,

    /// HW 443: Baseline adaptation file read status.
    pub baseline_adaptation_file_read_status: Integer2,

    /// HW 444: Read status of PRF sets (bitfield).
    pub read_status_of_prf_sets: Code2,

    /// HW 445: Clutter filter map file read status.
    pub clutter_filter_map_file_read_status: Integer2,

    /// HW 446: Clutter filter map file write status.
    pub clutter_filter_map_file_write_status: Integer2,

    /// HW 447: General disk I/O error.
    pub general_disk_io_error: Integer2,

    // ==================== RSP/CPU Status (HW 448-460) ====================
    /// HW 448: RSP status (byte 0 is Code1 bitfield, byte 1 is spare).
    pub rsp_status: Integer2,

    /// HW 449: CPU temperatures (byte 0 is CPU2 temperature, byte 1 is CPU1 temperature).
    pub cpu_temperatures: Integer2,

    /// HW 450: RSP motherboard power (Watts).
    pub rsp_motherboard_power: Integer2,

    /// HW 451-460: Spare.
    pub rsp_spare_451_460: [Integer2; 10],

    // ==================== Device Status (HW 461-480) ====================
    /// HW 461: SPIP communication status.
    pub spip_comm_status: Integer2,

    /// HW 462: HCI communication status.
    pub hci_comm_status: Integer2,

    /// HW 463: Spare.
    pub device_spare_463: Integer2,

    /// HW 464: Signal processor command status.
    pub signal_processor_command_status: Integer2,

    /// HW 465: AME communication status.
    pub ame_communication_status: Integer2,

    /// HW 466: RMS link status.
    pub rms_link_status: Integer2,

    /// HW 467: RPG link status.
    pub rpg_link_status: Integer2,

    /// HW 468: Interpanel link status.
    pub interpanel_link_status: Integer2,

    /// HW 469-470: Performance check time (Unix epoch time).
    pub performance_check_time: Integer4,

    /// HW 471-479: Spare.
    pub device_spare_471_479: [Integer2; 9],

    /// HW 480: Version.
    pub version: Integer2,
}

use crate::messages::performance_maintenance_data::raw;
use crate::result::Result;
use crate::segmented_slice_reader::SegmentedSliceReader;
use std::borrow::Cow;
use std::fmt::Debug;

/// The Performance/Maintenance Data message (Message Type 3) contains detailed performance and
/// maintenance information about the RDA system, organized into sections covering communications,
/// AME, power, transmitter, tower/utilities, antenna/pedestal, RF generator/receiver, calibration,
/// file status, RSP/CPU status, and device status.
#[derive(Clone, PartialEq, Debug)]
pub struct Message<'a> {
    inner: Cow<'a, raw::Message>,
}

impl<'a> Message<'a> {
    pub(crate) fn parse(reader: &mut SegmentedSliceReader<'a, '_>) -> Result<Self> {
        let inner = reader.take_ref::<raw::Message>()?;
        Ok(Self {
            inner: Cow::Borrowed(inner),
        })
    }

    // ==================== Communications ====================

    /// Loop back test status.
    /// Values: 0=Pass, 1=Fail, 2=Timeout, 3=Not Tested
    pub fn loop_back_test_status(&self) -> u16 {
        self.inner.loop_back_test_status.get()
    }

    /// T1 output frames.
    pub fn t1_output_frames(&self) -> u32 {
        self.inner.t1_output_frames.get()
    }

    /// T1 input frames.
    pub fn t1_input_frames(&self) -> u32 {
        self.inner.t1_input_frames.get()
    }

    /// Router memory used (bytes).
    pub fn router_memory_used(&self) -> u32 {
        self.inner.router_memory_used.get()
    }

    /// Router memory free (bytes).
    pub fn router_memory_free(&self) -> u32 {
        self.inner.router_memory_free.get()
    }

    /// Router memory utilization (%).
    pub fn router_memory_utilization(&self) -> u16 {
        self.inner.router_memory_utilization.get()
    }

    /// Route to RPG.
    /// Values: 0=Normal, 1=Backup in Use, 2=Down Failure, 3=Backup Commanded Down,
    /// 4=Not Installed
    pub fn route_to_rpg(&self) -> u16 {
        self.inner.route_to_rpg.get()
    }

    /// T1 port status.
    /// Values: 1=Up, 2=Down, 3=Test
    pub fn t1_port_status(&self) -> u16 {
        self.inner.t1_port_status.get()
    }

    /// Router dedicated Ethernet port status.
    /// Values: 1=Up, 2=Down, 3=Test
    pub fn router_dedicated_ethernet_port_status(&self) -> u16 {
        self.inner.router_dedicated_ethernet_port_status.get()
    }

    /// Router commercial Ethernet port status.
    /// Values: 1=Up, 2=Down, 3=Test
    pub fn router_commercial_ethernet_port_status(&self) -> u16 {
        self.inner.router_commercial_ethernet_port_status.get()
    }

    /// CSU 24-hour errored seconds.
    pub fn csu_24hr_errored_seconds(&self) -> u32 {
        self.inner.csu_24hr_errored_seconds.get()
    }

    /// CSU 24-hour severely errored seconds.
    pub fn csu_24hr_severely_errored_seconds(&self) -> u32 {
        self.inner.csu_24hr_severely_errored_seconds.get()
    }

    /// CSU 24-hour severely errored framing seconds.
    pub fn csu_24hr_severely_errored_framing_seconds(&self) -> u32 {
        self.inner.csu_24hr_severely_errored_framing_seconds.get()
    }

    /// CSU 24-hour unavailable seconds.
    pub fn csu_24hr_unavailable_seconds(&self) -> u32 {
        self.inner.csu_24hr_unavailable_seconds.get()
    }

    /// CSU 24-hour controlled slip seconds.
    pub fn csu_24hr_controlled_slip_seconds(&self) -> u32 {
        self.inner.csu_24hr_controlled_slip_seconds.get()
    }

    /// CSU 24-hour path coding violations.
    pub fn csu_24hr_path_coding_violations(&self) -> u32 {
        self.inner.csu_24hr_path_coding_violations.get()
    }

    /// CSU 24-hour line errored seconds.
    pub fn csu_24hr_line_errored_seconds(&self) -> u32 {
        self.inner.csu_24hr_line_errored_seconds.get()
    }

    /// CSU 24-hour bursty errored seconds.
    pub fn csu_24hr_bursty_errored_seconds(&self) -> u32 {
        self.inner.csu_24hr_bursty_errored_seconds.get()
    }

    /// CSU 24-hour degraded minutes.
    pub fn csu_24hr_degraded_minutes(&self) -> u32 {
        self.inner.csu_24hr_degraded_minutes.get()
    }

    /// LAN switch CPU utilization (%).
    pub fn lan_switch_cpu_utilization(&self) -> u32 {
        self.inner.lan_switch_cpu_utilization.get()
    }

    /// LAN switch memory utilization (%).
    pub fn lan_switch_memory_utilization(&self) -> u16 {
        self.inner.lan_switch_memory_utilization.get()
    }

    /// IFDR chassis temperature (deg C).
    pub fn ifdr_chassis_temperature(&self) -> i16 {
        self.inner.ifdr_chassis_temperature.get()
    }

    /// IFDR FPGA temperature (deg C).
    pub fn ifdr_fpga_temperature(&self) -> i16 {
        self.inner.ifdr_fpga_temperature.get()
    }

    /// NTP status.
    /// Values: 0=OK, 1=Fail
    pub fn ntp_status(&self) -> u16 {
        self.inner.ntp_status.get()
    }

    /// IPC status.
    /// Values: 0=OK, 1=Fail, 2=N/A
    pub fn ipc_status(&self) -> u16 {
        self.inner.ipc_status.get()
    }

    /// Commanded channel control.
    /// Values: 0=N/A, 1=Channel 1, 2=Channel 2
    pub fn commanded_channel_control(&self) -> u16 {
        self.inner.commanded_channel_control.get()
    }

    // ==================== AME ====================

    /// Polarization.
    /// Values: 0=H Only, 1=H+V, 2=V Only
    pub fn polarization(&self) -> u16 {
        self.inner.polarization.get()
    }

    /// AME internal temperature (deg C).
    pub fn ame_internal_temperature(&self) -> f32 {
        self.inner.ame_internal_temperature.get()
    }

    /// AME receiver module temperature (deg C).
    pub fn ame_receiver_module_temperature(&self) -> f32 {
        self.inner.ame_receiver_module_temperature.get()
    }

    /// AME BITE/CAL module temperature (deg C).
    pub fn ame_bite_cal_module_temperature(&self) -> f32 {
        self.inner.ame_bite_cal_module_temperature.get()
    }

    /// AME Peltier pulse width modulation (%).
    pub fn ame_peltier_pulse_width_modulation(&self) -> u16 {
        self.inner.ame_peltier_pulse_width_modulation.get()
    }

    /// AME Peltier status.
    /// Values: 0=OFF, 1=ON
    pub fn ame_peltier_status(&self) -> u16 {
        self.inner.ame_peltier_status.get()
    }

    /// AME A/D converter status.
    /// Values: 0=OK, 1=FAIL
    pub fn ame_ad_converter_status(&self) -> u16 {
        self.inner.ame_ad_converter_status.get()
    }

    /// AME state.
    /// Values: 0=START, 1=RUNNING, 2=FLASH, 3=ERROR
    pub fn ame_state(&self) -> u16 {
        self.inner.ame_state.get()
    }

    /// AME 3.3V PS voltage (V).
    pub fn ame_3_3v_ps_voltage(&self) -> f32 {
        self.inner.ame_3_3v_ps_voltage.get()
    }

    /// AME 5V PS voltage (V).
    pub fn ame_5v_ps_voltage(&self) -> f32 {
        self.inner.ame_5v_ps_voltage.get()
    }

    /// AME 6.5V PS voltage (V).
    pub fn ame_6_5v_ps_voltage(&self) -> f32 {
        self.inner.ame_6_5v_ps_voltage.get()
    }

    /// AME 15V PS voltage (V).
    pub fn ame_15v_ps_voltage(&self) -> f32 {
        self.inner.ame_15v_ps_voltage.get()
    }

    /// AME 48V PS voltage (V).
    pub fn ame_48v_ps_voltage(&self) -> f32 {
        self.inner.ame_48v_ps_voltage.get()
    }

    /// AME STALO power (V).
    pub fn ame_stalo_power(&self) -> f32 {
        self.inner.ame_stalo_power.get()
    }

    /// Peltier current (A).
    pub fn peltier_current(&self) -> f32 {
        self.inner.peltier_current.get()
    }

    /// ADC calibration reference voltage (V).
    pub fn adc_calibration_reference_voltage(&self) -> f32 {
        self.inner.adc_calibration_reference_voltage.get()
    }

    /// AME mode.
    /// Values: 0=READY, 1=MAINTENANCE
    pub fn ame_mode(&self) -> u16 {
        self.inner.ame_mode.get()
    }

    /// AME Peltier mode.
    /// Values: 0=COOL, 1=HEAT
    pub fn ame_peltier_mode(&self) -> u16 {
        self.inner.ame_peltier_mode.get()
    }

    /// AME Peltier inside fan current (A).
    pub fn ame_peltier_inside_fan_current(&self) -> f32 {
        self.inner.ame_peltier_inside_fan_current.get()
    }

    /// AME Peltier outside fan current (A).
    pub fn ame_peltier_outside_fan_current(&self) -> f32 {
        self.inner.ame_peltier_outside_fan_current.get()
    }

    /// Horizontal TR limiter voltage (V).
    pub fn horizontal_tr_limiter_voltage(&self) -> f32 {
        self.inner.horizontal_tr_limiter_voltage.get()
    }

    /// Vertical TR limiter voltage (V).
    pub fn vertical_tr_limiter_voltage(&self) -> f32 {
        self.inner.vertical_tr_limiter_voltage.get()
    }

    /// ADC calibration offset voltage (mV).
    pub fn adc_calibration_offset_voltage(&self) -> f32 {
        self.inner.adc_calibration_offset_voltage.get()
    }

    /// ADC calibration gain correction.
    pub fn adc_calibration_gain_correction(&self) -> f32 {
        self.inner.adc_calibration_gain_correction.get()
    }

    /// RCP status.
    /// Values: 0=RCP OK, 1=NOT OK
    pub fn rcp_status(&self) -> u16 {
        self.inner.rcp_status.get()
    }

    /// RCP string (16 bytes, ASCII).
    pub fn rcp_string(&self) -> &[u8; 16] {
        &self.inner.rcp_string
    }

    /// SPIP power buttons (bitfield).
    pub fn spip_power_buttons(&self) -> u16 {
        self.inner.spip_power_buttons.get()
    }

    // ==================== Power ====================

    /// Master power administrator load (A).
    pub fn master_power_administrator_load(&self) -> f32 {
        self.inner.master_power_administrator_load.get()
    }

    /// Expansion power administrator load (A).
    pub fn expansion_power_administrator_load(&self) -> f32 {
        self.inner.expansion_power_administrator_load.get()
    }

    // ==================== Transmitter ====================

    /// +5 VDC PS.
    pub fn plus_5_vdc_ps(&self) -> u16 {
        self.inner.plus_5_vdc_ps.get()
    }

    /// +15 VDC PS.
    pub fn plus_15_vdc_ps(&self) -> u16 {
        self.inner.plus_15_vdc_ps.get()
    }

    /// +28 VDC PS.
    pub fn plus_28_vdc_ps(&self) -> u16 {
        self.inner.plus_28_vdc_ps.get()
    }

    /// -15 VDC PS.
    pub fn minus_15_vdc_ps(&self) -> u16 {
        self.inner.minus_15_vdc_ps.get()
    }

    /// +45 VDC PS.
    pub fn plus_45_vdc_ps(&self) -> u16 {
        self.inner.plus_45_vdc_ps.get()
    }

    /// Filament PS voltage.
    pub fn filament_ps_voltage(&self) -> u16 {
        self.inner.filament_ps_voltage.get()
    }

    /// Vacuum pump PS voltage.
    pub fn vacuum_pump_ps_voltage(&self) -> u16 {
        self.inner.vacuum_pump_ps_voltage.get()
    }

    /// Focus coil PS voltage.
    pub fn focus_coil_ps_voltage(&self) -> u16 {
        self.inner.focus_coil_ps_voltage.get()
    }

    /// Filament PS.
    /// Values: 0=On, 1=Off
    pub fn filament_ps(&self) -> u16 {
        self.inner.filament_ps.get()
    }

    /// Klystron warmup.
    /// Values: 0=Normal, 1=Preheat
    pub fn klystron_warmup(&self) -> u16 {
        self.inner.klystron_warmup.get()
    }

    /// Transmitter available.
    /// Values: 0=Yes, 1=No
    pub fn transmitter_available(&self) -> u16 {
        self.inner.transmitter_available.get()
    }

    /// WG switch position.
    /// Values: 0=Antenna, 1=Dummy Load
    pub fn wg_switch_position(&self) -> u16 {
        self.inner.wg_switch_position.get()
    }

    /// WG/PFN transfer interlock.
    /// Values: 0=OK, 1=Open
    pub fn wg_pfn_transfer_interlock(&self) -> u16 {
        self.inner.wg_pfn_transfer_interlock.get()
    }

    /// Maintenance mode.
    /// Values: 0=No, 1=Yes
    pub fn maintenance_mode(&self) -> u16 {
        self.inner.maintenance_mode.get()
    }

    /// Maintenance required.
    /// Values: 0=No, 1=Required
    pub fn maintenance_required(&self) -> u16 {
        self.inner.maintenance_required.get()
    }

    /// PFN switch position.
    /// Values: 0=Short Pulse, 1=Long Pulse
    pub fn pfn_switch_position(&self) -> u16 {
        self.inner.pfn_switch_position.get()
    }

    /// Modulator overload.
    pub fn modulator_overload(&self) -> u16 {
        self.inner.modulator_overload.get()
    }

    /// Modulator inverter current.
    pub fn modulator_inv_current(&self) -> u16 {
        self.inner.modulator_inv_current.get()
    }

    /// Modulator switch fail.
    pub fn modulator_switch_fail(&self) -> u16 {
        self.inner.modulator_switch_fail.get()
    }

    /// Main power voltage.
    pub fn main_power_voltage(&self) -> u16 {
        self.inner.main_power_voltage.get()
    }

    /// Charging system fail.
    pub fn charging_system_fail(&self) -> u16 {
        self.inner.charging_system_fail.get()
    }

    /// Inverse diode current.
    pub fn inverse_diode_current(&self) -> u16 {
        self.inner.inverse_diode_current.get()
    }

    /// Trigger amplifier.
    pub fn trigger_amplifier(&self) -> u16 {
        self.inner.trigger_amplifier.get()
    }

    /// Circulator temperature.
    pub fn circulator_temperature(&self) -> u16 {
        self.inner.circulator_temperature.get()
    }

    /// Spectrum filter pressure.
    pub fn spectrum_filter_pressure(&self) -> u16 {
        self.inner.spectrum_filter_pressure.get()
    }

    /// WG arc/VSWR.
    pub fn wg_arc_vswr(&self) -> u16 {
        self.inner.wg_arc_vswr.get()
    }

    /// Cabinet interlock.
    pub fn cabinet_interlock(&self) -> u16 {
        self.inner.cabinet_interlock.get()
    }

    /// Cabinet air temperature.
    pub fn cabinet_air_temperature(&self) -> u16 {
        self.inner.cabinet_air_temperature.get()
    }

    /// Cabinet airflow.
    pub fn cabinet_airflow(&self) -> u16 {
        self.inner.cabinet_airflow.get()
    }

    /// Klystron current.
    pub fn klystron_current(&self) -> u16 {
        self.inner.klystron_current.get()
    }

    /// Klystron filament current.
    pub fn klystron_filament_current(&self) -> u16 {
        self.inner.klystron_filament_current.get()
    }

    /// Klystron vacion current.
    pub fn klystron_vacion_current(&self) -> u16 {
        self.inner.klystron_vacion_current.get()
    }

    /// Klystron air temperature.
    pub fn klystron_air_temperature(&self) -> u16 {
        self.inner.klystron_air_temperature.get()
    }

    /// Klystron airflow.
    pub fn klystron_airflow(&self) -> u16 {
        self.inner.klystron_airflow.get()
    }

    /// Modulator switch maintenance.
    pub fn modulator_switch_maintenance(&self) -> u16 {
        self.inner.modulator_switch_maintenance.get()
    }

    /// Post charge regulator maintenance.
    pub fn post_charge_regulator_maintenance(&self) -> u16 {
        self.inner.post_charge_regulator_maintenance.get()
    }

    /// WG pressure/humidity.
    pub fn wg_pressure_humidity(&self) -> u16 {
        self.inner.wg_pressure_humidity.get()
    }

    /// Transmitter overvoltage.
    pub fn transmitter_overvoltage(&self) -> u16 {
        self.inner.transmitter_overvoltage.get()
    }

    /// Transmitter overcurrent.
    pub fn transmitter_overcurrent(&self) -> u16 {
        self.inner.transmitter_overcurrent.get()
    }

    /// Focus coil current.
    pub fn focus_coil_current(&self) -> u16 {
        self.inner.focus_coil_current.get()
    }

    /// Focus coil airflow.
    pub fn focus_coil_airflow(&self) -> u16 {
        self.inner.focus_coil_airflow.get()
    }

    /// Oil temperature.
    pub fn oil_temperature(&self) -> u16 {
        self.inner.oil_temperature.get()
    }

    /// PRF limit.
    pub fn prf_limit(&self) -> u16 {
        self.inner.prf_limit.get()
    }

    /// Transmitter oil level.
    pub fn transmitter_oil_level(&self) -> u16 {
        self.inner.transmitter_oil_level.get()
    }

    /// Transmitter battery charging.
    /// Values: 0=Yes, 1=No
    pub fn transmitter_battery_charging(&self) -> u16 {
        self.inner.transmitter_battery_charging.get()
    }

    /// High voltage status.
    /// Values: 0=On, 1=Off
    pub fn high_voltage_status(&self) -> u16 {
        self.inner.high_voltage_status.get()
    }

    /// Transmitter recycling summary.
    /// Values: 0=Normal, 1=Recycling
    pub fn transmitter_recycling_summary(&self) -> u16 {
        self.inner.transmitter_recycling_summary.get()
    }

    /// Transmitter inoperable.
    pub fn transmitter_inoperable(&self) -> u16 {
        self.inner.transmitter_inoperable.get()
    }

    /// Transmitter air filter.
    /// Values: 0=Dirty, 1=OK
    pub fn transmitter_air_filter(&self) -> u16 {
        self.inner.transmitter_air_filter.get()
    }

    /// Zero test bits 0-7.
    pub fn zero_test_bits(&self) -> [u16; 8] {
        self.inner.zero_test_bits.map(|v| v.get())
    }

    /// One test bits 0-7.
    pub fn one_test_bits(&self) -> [u16; 8] {
        self.inner.one_test_bits.map(|v| v.get())
    }

    /// Transmitter SPIP interface.
    pub fn xmtr_spip_interface(&self) -> u16 {
        self.inner.xmtr_spip_interface.get()
    }

    /// Transmitter summary status.
    /// Values: 0=Ready, 1=Alarm, 2=Maintenance, 3=Recycle, 4=Preheat
    pub fn transmitter_summary_status(&self) -> u16 {
        self.inner.transmitter_summary_status.get()
    }

    /// Transmitter RF power sensor (mW).
    pub fn transmitter_rf_power_sensor(&self) -> f32 {
        self.inner.transmitter_rf_power_sensor.get()
    }

    /// Horizontal transmitter peak power (kW).
    pub fn horizontal_xmtr_peak_power(&self) -> f32 {
        self.inner.horizontal_xmtr_peak_power.get()
    }

    /// Transmitter peak power (kW).
    pub fn xmtr_peak_power(&self) -> f32 {
        self.inner.xmtr_peak_power.get()
    }

    /// Vertical transmitter peak power (kW).
    pub fn vertical_xmtr_peak_power(&self) -> f32 {
        self.inner.vertical_xmtr_peak_power.get()
    }

    /// Transmitter RF average power (W).
    pub fn xmtr_rf_avg_power(&self) -> f32 {
        self.inner.xmtr_rf_avg_power.get()
    }

    /// Transmitter recycle count.
    pub fn xmtr_recycle_count(&self) -> u32 {
        self.inner.xmtr_recycle_count.get()
    }

    /// Receiver bias measurement (dB).
    pub fn receiver_bias_measurement(&self) -> f32 {
        self.inner.receiver_bias_measurement.get()
    }

    /// Transmit imbalance (dB).
    pub fn transmit_imbalance(&self) -> f32 {
        self.inner.transmit_imbalance.get()
    }

    /// Transmitter power meter zero (V).
    pub fn xmtr_power_meter_zero(&self) -> f32 {
        self.inner.xmtr_power_meter_zero.get()
    }

    // ==================== Tower/Utilities ====================

    /// AC unit 1 compressor shut off.
    pub fn ac_unit_1_compressor_shut_off(&self) -> u16 {
        self.inner.ac_unit_1_compressor_shut_off.get()
    }

    /// AC unit 2 compressor shut off.
    pub fn ac_unit_2_compressor_shut_off(&self) -> u16 {
        self.inner.ac_unit_2_compressor_shut_off.get()
    }

    /// Generator maintenance required.
    pub fn generator_maintenance_required(&self) -> u16 {
        self.inner.generator_maintenance_required.get()
    }

    /// Generator battery voltage.
    pub fn generator_battery_voltage(&self) -> u16 {
        self.inner.generator_battery_voltage.get()
    }

    /// Generator engine.
    pub fn generator_engine(&self) -> u16 {
        self.inner.generator_engine.get()
    }

    /// Generator volt/frequency.
    pub fn generator_volt_frequency(&self) -> u16 {
        self.inner.generator_volt_frequency.get()
    }

    /// Power source.
    /// Values: 0=Utility, 1=Generator
    pub fn power_source(&self) -> u16 {
        self.inner.power_source.get()
    }

    /// Transitional power source.
    /// Values: 0=OK, 1=Off
    pub fn transitional_power_source(&self) -> u16 {
        self.inner.transitional_power_source.get()
    }

    /// Generator auto run off switch.
    /// Values: 0=Manual, 1=Auto
    pub fn generator_auto_run_off_switch(&self) -> u16 {
        self.inner.generator_auto_run_off_switch.get()
    }

    /// Aircraft hazard lighting.
    pub fn aircraft_hazard_lighting(&self) -> u16 {
        self.inner.aircraft_hazard_lighting.get()
    }

    /// Equipment shelter fire detection system.
    pub fn equipment_shelter_fire_detection_system(&self) -> u16 {
        self.inner.equipment_shelter_fire_detection_system.get()
    }

    /// Equipment shelter fire/smoke.
    pub fn equipment_shelter_fire_smoke(&self) -> u16 {
        self.inner.equipment_shelter_fire_smoke.get()
    }

    /// Generator shelter fire/smoke.
    pub fn generator_shelter_fire_smoke(&self) -> u16 {
        self.inner.generator_shelter_fire_smoke.get()
    }

    /// Utility voltage/frequency.
    pub fn utility_voltage_frequency(&self) -> u16 {
        self.inner.utility_voltage_frequency.get()
    }

    /// Site security alarm.
    pub fn site_security_alarm(&self) -> u16 {
        self.inner.site_security_alarm.get()
    }

    /// Security equipment.
    pub fn security_equipment(&self) -> u16 {
        self.inner.security_equipment.get()
    }

    /// Security system.
    pub fn security_system(&self) -> u16 {
        self.inner.security_system.get()
    }

    /// Receiver connected to antenna.
    /// Values: 0=Connected, 1=Not Connected, 2=N/A
    pub fn receiver_connected_to_antenna(&self) -> u16 {
        self.inner.receiver_connected_to_antenna.get()
    }

    /// Radome hatch.
    /// Values: 0=Open, 1=Closed
    pub fn radome_hatch(&self) -> u16 {
        self.inner.radome_hatch.get()
    }

    /// AC unit 1 filter dirty.
    pub fn ac_unit_1_filter_dirty(&self) -> u16 {
        self.inner.ac_unit_1_filter_dirty.get()
    }

    /// AC unit 2 filter dirty.
    pub fn ac_unit_2_filter_dirty(&self) -> u16 {
        self.inner.ac_unit_2_filter_dirty.get()
    }

    /// Equipment shelter temperature (deg C).
    pub fn equipment_shelter_temperature(&self) -> f32 {
        self.inner.equipment_shelter_temperature.get()
    }

    /// Outside ambient temperature (deg C).
    pub fn outside_ambient_temperature(&self) -> f32 {
        self.inner.outside_ambient_temperature.get()
    }

    /// Transmitter leaving air temperature (deg C).
    pub fn transmitter_leaving_air_temp(&self) -> f32 {
        self.inner.transmitter_leaving_air_temp.get()
    }

    /// AC unit 1 discharge air temperature (deg C).
    pub fn ac_unit_1_discharge_air_temp(&self) -> f32 {
        self.inner.ac_unit_1_discharge_air_temp.get()
    }

    /// Generator shelter temperature (deg C).
    pub fn generator_shelter_temperature(&self) -> f32 {
        self.inner.generator_shelter_temperature.get()
    }

    /// Radome air temperature (deg C).
    pub fn radome_air_temperature(&self) -> f32 {
        self.inner.radome_air_temperature.get()
    }

    /// AC unit 2 discharge air temperature (deg C).
    pub fn ac_unit_2_discharge_air_temp(&self) -> f32 {
        self.inner.ac_unit_2_discharge_air_temp.get()
    }

    /// SPIP +15V PS (V).
    pub fn spip_15v_ps(&self) -> f32 {
        self.inner.spip_15v_ps.get()
    }

    /// SPIP -15V PS (V).
    pub fn spip_neg_15v_ps(&self) -> f32 {
        self.inner.spip_neg_15v_ps.get()
    }

    /// SPIP 28V PS status.
    /// Values: 0=Fail, 1=OK
    pub fn spip_28v_ps_status(&self) -> u16 {
        self.inner.spip_28v_ps_status.get()
    }

    /// SPIP 5V PS (V).
    pub fn spip_5v_ps(&self) -> f32 {
        self.inner.spip_5v_ps.get()
    }

    /// Converted generator fuel level (%).
    pub fn converted_generator_fuel_level(&self) -> u16 {
        self.inner.converted_generator_fuel_level.get()
    }

    // ==================== Antenna/Pedestal ====================

    /// Elevation +dead limit.
    pub fn elevation_plus_dead_limit(&self) -> u16 {
        self.inner.elevation_plus_dead_limit.get()
    }

    /// +150V overvoltage.
    pub fn plus_150v_overvoltage(&self) -> u16 {
        self.inner.plus_150v_overvoltage.get()
    }

    /// +150V undervoltage.
    pub fn plus_150v_undervoltage(&self) -> u16 {
        self.inner.plus_150v_undervoltage.get()
    }

    /// Elevation servo amp inhibit.
    pub fn elevation_servo_amp_inhibit(&self) -> u16 {
        self.inner.elevation_servo_amp_inhibit.get()
    }

    /// Elevation servo amp short circuit.
    pub fn elevation_servo_amp_short_circuit(&self) -> u16 {
        self.inner.elevation_servo_amp_short_circuit.get()
    }

    /// Elevation servo amp overtemp.
    pub fn elevation_servo_amp_overtemp(&self) -> u16 {
        self.inner.elevation_servo_amp_overtemp.get()
    }

    /// Elevation motor overtemp.
    pub fn elevation_motor_overtemp(&self) -> u16 {
        self.inner.elevation_motor_overtemp.get()
    }

    /// Elevation stow pin.
    pub fn elevation_stow_pin(&self) -> u16 {
        self.inner.elevation_stow_pin.get()
    }

    /// Elevation housing 5V PS.
    pub fn elevation_housing_5v_ps(&self) -> u16 {
        self.inner.elevation_housing_5v_ps.get()
    }

    /// Elevation -dead limit.
    pub fn elevation_minus_dead_limit(&self) -> u16 {
        self.inner.elevation_minus_dead_limit.get()
    }

    /// Elevation +normal limit.
    pub fn elevation_plus_normal_limit(&self) -> u16 {
        self.inner.elevation_plus_normal_limit.get()
    }

    /// Elevation -normal limit.
    pub fn elevation_minus_normal_limit(&self) -> u16 {
        self.inner.elevation_minus_normal_limit.get()
    }

    /// Elevation encoder light.
    pub fn elevation_encoder_light(&self) -> u16 {
        self.inner.elevation_encoder_light.get()
    }

    /// Elevation gearbox oil.
    pub fn elevation_gearbox_oil(&self) -> u16 {
        self.inner.elevation_gearbox_oil.get()
    }

    /// Elevation handwheel.
    pub fn elevation_handwheel(&self) -> u16 {
        self.inner.elevation_handwheel.get()
    }

    /// Elevation amp PS.
    pub fn elevation_amp_ps(&self) -> u16 {
        self.inner.elevation_amp_ps.get()
    }

    /// Azimuth servo amp inhibit.
    pub fn azimuth_servo_amp_inhibit(&self) -> u16 {
        self.inner.azimuth_servo_amp_inhibit.get()
    }

    /// Azimuth servo amp short circuit.
    pub fn azimuth_servo_amp_short_circuit(&self) -> u16 {
        self.inner.azimuth_servo_amp_short_circuit.get()
    }

    /// Azimuth servo amp overtemp.
    pub fn azimuth_servo_amp_overtemp(&self) -> u16 {
        self.inner.azimuth_servo_amp_overtemp.get()
    }

    /// Azimuth motor overtemp.
    pub fn azimuth_motor_overtemp(&self) -> u16 {
        self.inner.azimuth_motor_overtemp.get()
    }

    /// Azimuth stow pin.
    pub fn azimuth_stow_pin(&self) -> u16 {
        self.inner.azimuth_stow_pin.get()
    }

    /// Azimuth housing 5V PS.
    pub fn azimuth_housing_5v_ps(&self) -> u16 {
        self.inner.azimuth_housing_5v_ps.get()
    }

    /// Azimuth encoder light.
    pub fn azimuth_encoder_light(&self) -> u16 {
        self.inner.azimuth_encoder_light.get()
    }

    /// Azimuth gearbox oil.
    pub fn azimuth_gearbox_oil(&self) -> u16 {
        self.inner.azimuth_gearbox_oil.get()
    }

    /// Azimuth bull gear oil.
    pub fn azimuth_bull_gear_oil(&self) -> u16 {
        self.inner.azimuth_bull_gear_oil.get()
    }

    /// Azimuth handwheel.
    pub fn azimuth_handwheel(&self) -> u16 {
        self.inner.azimuth_handwheel.get()
    }

    /// Azimuth servo amp PS.
    pub fn azimuth_servo_amp_ps(&self) -> u16 {
        self.inner.azimuth_servo_amp_ps.get()
    }

    /// Servo.
    /// Values: 0=On, 1=Off
    pub fn servo(&self) -> u16 {
        self.inner.servo.get()
    }

    /// Pedestal interlock switch.
    /// Values: 0=Operational, 1=Safe
    pub fn pedestal_interlock_switch(&self) -> u16 {
        self.inner.pedestal_interlock_switch.get()
    }

    // ==================== RF Generator/Receiver ====================

    /// COHO/clock.
    pub fn coho_clock(&self) -> u16 {
        self.inner.coho_clock.get()
    }

    /// RF generator frequency select oscillator.
    pub fn rf_generator_frequency_select_oscillator(&self) -> u16 {
        self.inner.rf_generator_frequency_select_oscillator.get()
    }

    /// RF generator RF STALO.
    pub fn rf_generator_rf_stalo(&self) -> u16 {
        self.inner.rf_generator_rf_stalo.get()
    }

    /// RF generator phase-shifted COHO.
    pub fn rf_generator_phase_shifted_coho(&self) -> u16 {
        self.inner.rf_generator_phase_shifted_coho.get()
    }

    /// +9V receiver PS.
    pub fn plus_9v_receiver_ps(&self) -> u16 {
        self.inner.plus_9v_receiver_ps.get()
    }

    /// +5V receiver PS.
    pub fn plus_5v_receiver_ps(&self) -> u16 {
        self.inner.plus_5v_receiver_ps.get()
    }

    /// +/-18V receiver PS.
    pub fn plus_or_minus_18v_receiver_ps(&self) -> u16 {
        self.inner.plus_or_minus_18v_receiver_ps.get()
    }

    /// -9V receiver PS.
    pub fn minus_9v_receiver_ps(&self) -> u16 {
        self.inner.minus_9v_receiver_ps.get()
    }

    /// +5V single channel RDAIU PS.
    pub fn plus_5v_single_channel_rdaiu_ps(&self) -> u16 {
        self.inner.plus_5v_single_channel_rdaiu_ps.get()
    }

    /// Horizontal short pulse noise (dBm).
    pub fn horizontal_short_pulse_noise(&self) -> f32 {
        self.inner.horizontal_short_pulse_noise.get()
    }

    /// Horizontal long pulse noise (dBm).
    pub fn horizontal_long_pulse_noise(&self) -> f32 {
        self.inner.horizontal_long_pulse_noise.get()
    }

    /// Horizontal noise temperature (K).
    pub fn horizontal_noise_temperature(&self) -> f32 {
        self.inner.horizontal_noise_temperature.get()
    }

    /// Vertical short pulse noise (dBm).
    pub fn vertical_short_pulse_noise(&self) -> f32 {
        self.inner.vertical_short_pulse_noise.get()
    }

    /// Vertical long pulse noise (dBm).
    pub fn vertical_long_pulse_noise(&self) -> f32 {
        self.inner.vertical_long_pulse_noise.get()
    }

    /// Vertical noise temperature (K).
    pub fn vertical_noise_temperature(&self) -> f32 {
        self.inner.vertical_noise_temperature.get()
    }

    // ==================== Calibration ====================

    /// Horizontal linearity.
    pub fn horizontal_linearity(&self) -> f32 {
        self.inner.horizontal_linearity.get()
    }

    /// Horizontal dynamic range (dB).
    pub fn horizontal_dynamic_range(&self) -> f32 {
        self.inner.horizontal_dynamic_range.get()
    }

    /// Horizontal delta dBZ0 (dB).
    pub fn horizontal_delta_dbz0(&self) -> f32 {
        self.inner.horizontal_delta_dbz0.get()
    }

    /// Vertical delta dBZ0 (dB).
    pub fn vertical_delta_dbz0(&self) -> f32 {
        self.inner.vertical_delta_dbz0.get()
    }

    /// KD peak measured (dBm).
    pub fn kd_peak_measured(&self) -> f32 {
        self.inner.kd_peak_measured.get()
    }

    /// Short pulse horizontal dBZ0 (dBZ).
    pub fn short_pulse_horizontal_dbz0(&self) -> f32 {
        self.inner.short_pulse_horizontal_dbz0.get()
    }

    /// Long pulse horizontal dBZ0 (dBZ).
    pub fn long_pulse_horizontal_dbz0(&self) -> f32 {
        self.inner.long_pulse_horizontal_dbz0.get()
    }

    /// Velocity processed.
    /// Values: 0=Good, 1=Fail
    pub fn velocity_processed(&self) -> u16 {
        self.inner.velocity_processed.get()
    }

    /// Width processed.
    pub fn width_processed(&self) -> u16 {
        self.inner.width_processed.get()
    }

    /// Velocity RF gen.
    pub fn velocity_rf_gen(&self) -> u16 {
        self.inner.velocity_rf_gen.get()
    }

    /// Width RF gen.
    pub fn width_rf_gen(&self) -> u16 {
        self.inner.width_rf_gen.get()
    }

    /// Horizontal I0 (dBm).
    pub fn horizontal_i0(&self) -> f32 {
        self.inner.horizontal_i0.get()
    }

    /// Vertical I0 (dBm).
    pub fn vertical_i0(&self) -> f32 {
        self.inner.vertical_i0.get()
    }

    /// Vertical dynamic range (dB).
    pub fn vertical_dynamic_range(&self) -> f32 {
        self.inner.vertical_dynamic_range.get()
    }

    /// Short pulse vertical dBZ0 (dBZ).
    pub fn short_pulse_vertical_dbz0(&self) -> f32 {
        self.inner.short_pulse_vertical_dbz0.get()
    }

    /// Long pulse vertical dBZ0 (dBZ).
    pub fn long_pulse_vertical_dbz0(&self) -> f32 {
        self.inner.long_pulse_vertical_dbz0.get()
    }

    /// Horizontal power sense (dBm).
    pub fn horizontal_power_sense(&self) -> f32 {
        self.inner.horizontal_power_sense.get()
    }

    /// Vertical power sense (dBm).
    pub fn vertical_power_sense(&self) -> f32 {
        self.inner.vertical_power_sense.get()
    }

    /// ZDR offset (dB).
    pub fn zdr_offset(&self) -> f32 {
        self.inner.zdr_offset.get()
    }

    /// Clutter suppression delta (dB).
    pub fn clutter_suppression_delta(&self) -> f32 {
        self.inner.clutter_suppression_delta.get()
    }

    /// Clutter suppression unfiltered power (dBZ).
    pub fn clutter_suppression_unfiltered_power(&self) -> f32 {
        self.inner.clutter_suppression_unfiltered_power.get()
    }

    /// Clutter suppression filtered power (dBZ).
    pub fn clutter_suppression_filtered_power(&self) -> f32 {
        self.inner.clutter_suppression_filtered_power.get()
    }

    /// Vertical linearity.
    pub fn vertical_linearity(&self) -> f32 {
        self.inner.vertical_linearity.get()
    }

    // ==================== File Status ====================

    /// State file read status.
    pub fn state_file_read_status(&self) -> u16 {
        self.inner.state_file_read_status.get()
    }

    /// State file write status.
    pub fn state_file_write_status(&self) -> u16 {
        self.inner.state_file_write_status.get()
    }

    /// Bypass map file read status.
    pub fn bypass_map_file_read_status(&self) -> u16 {
        self.inner.bypass_map_file_read_status.get()
    }

    /// Bypass map file write status.
    pub fn bypass_map_file_write_status(&self) -> u16 {
        self.inner.bypass_map_file_write_status.get()
    }

    /// Current adaptation file read status.
    pub fn current_adaptation_file_read_status(&self) -> u16 {
        self.inner.current_adaptation_file_read_status.get()
    }

    /// Current adaptation file write status.
    pub fn current_adaptation_file_write_status(&self) -> u16 {
        self.inner.current_adaptation_file_write_status.get()
    }

    /// Censor zone file read status.
    pub fn censor_zone_file_read_status(&self) -> u16 {
        self.inner.censor_zone_file_read_status.get()
    }

    /// Censor zone file write status.
    pub fn censor_zone_file_write_status(&self) -> u16 {
        self.inner.censor_zone_file_write_status.get()
    }

    /// Remote VCP file read status.
    pub fn remote_vcp_file_read_status(&self) -> u16 {
        self.inner.remote_vcp_file_read_status.get()
    }

    /// Remote VCP file write status.
    pub fn remote_vcp_file_write_status(&self) -> u16 {
        self.inner.remote_vcp_file_write_status.get()
    }

    /// Baseline adaptation file read status.
    pub fn baseline_adaptation_file_read_status(&self) -> u16 {
        self.inner.baseline_adaptation_file_read_status.get()
    }

    /// Read status of PRF sets (bitfield).
    pub fn read_status_of_prf_sets(&self) -> u16 {
        self.inner.read_status_of_prf_sets.get()
    }

    /// Clutter filter map file read status.
    pub fn clutter_filter_map_file_read_status(&self) -> u16 {
        self.inner.clutter_filter_map_file_read_status.get()
    }

    /// Clutter filter map file write status.
    pub fn clutter_filter_map_file_write_status(&self) -> u16 {
        self.inner.clutter_filter_map_file_write_status.get()
    }

    /// General disk I/O error.
    pub fn general_disk_io_error(&self) -> u16 {
        self.inner.general_disk_io_error.get()
    }

    // ==================== RSP/CPU Status ====================

    /// RSP status (byte 0 is Code1 bitfield, byte 1 is spare).
    pub fn rsp_status(&self) -> u16 {
        self.inner.rsp_status.get()
    }

    /// CPU temperatures (byte 0 is CPU2 temperature, byte 1 is CPU1 temperature).
    pub fn cpu_temperatures(&self) -> u16 {
        self.inner.cpu_temperatures.get()
    }

    /// RSP motherboard power (Watts).
    pub fn rsp_motherboard_power(&self) -> u16 {
        self.inner.rsp_motherboard_power.get()
    }

    // ==================== Device Status ====================

    /// SPIP communication status.
    pub fn spip_comm_status(&self) -> u16 {
        self.inner.spip_comm_status.get()
    }

    /// HCI communication status.
    pub fn hci_comm_status(&self) -> u16 {
        self.inner.hci_comm_status.get()
    }

    /// Signal processor command status.
    pub fn signal_processor_command_status(&self) -> u16 {
        self.inner.signal_processor_command_status.get()
    }

    /// AME communication status.
    pub fn ame_communication_status(&self) -> u16 {
        self.inner.ame_communication_status.get()
    }

    /// RMS link status.
    pub fn rms_link_status(&self) -> u16 {
        self.inner.rms_link_status.get()
    }

    /// RPG link status.
    pub fn rpg_link_status(&self) -> u16 {
        self.inner.rpg_link_status.get()
    }

    /// Interpanel link status.
    pub fn interpanel_link_status(&self) -> u16 {
        self.inner.interpanel_link_status.get()
    }

    /// Performance check time (Unix epoch time).
    pub fn performance_check_time(&self) -> u32 {
        self.inner.performance_check_time.get()
    }

    /// Version.
    pub fn version(&self) -> u16 {
        self.inner.version.get()
    }

    /// Convert this message to an owned version with `'static` lifetime.
    pub fn into_owned(self) -> Message<'static> {
        Message {
            inner: Cow::Owned(self.inner.into_owned()),
        }
    }
}

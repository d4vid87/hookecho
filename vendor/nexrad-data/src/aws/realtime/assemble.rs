//! Assembles real-time data chunks into a complete volume scan.

use crate::aws::realtime::Chunk;
use crate::result::Error;

/// Assembles a collection of real-time data chunks into a [`Scan`](nexrad_model::data::Scan).
///
/// Takes an iterator of chunks (typically collected from a [`ChunkIterator`](super::ChunkIterator)
/// or `chunk_stream`) and combines them into a complete volume scan.
///
/// The chunks should all belong to the same volume and should include a start chunk.
/// Chunks are processed in the order provided; radials from all chunks are combined
/// and grouped into sweeps by elevation.
///
/// # Errors
///
/// Returns an error if:
/// - No start chunk is found (needed for the volume coverage pattern)
/// - Decompression or message decoding fails for any chunk
///
/// # Example
///
/// ```ignore
/// use nexrad_data::aws::realtime::{ChunkIterator, assemble_volume};
///
/// let init = ChunkIterator::start("KTLX").await?;
/// let mut chunks = vec![init.latest_chunk];
/// // ... collect more chunks ...
/// let scan = assemble_volume(chunks.into_iter().map(|dc| dc.chunk))?;
/// ```
pub fn assemble_volume<'a>(
    chunks: impl IntoIterator<Item = Chunk<'a>>,
) -> crate::result::Result<nexrad_model::data::Scan> {
    use nexrad_decode::messages::volume_coverage_pattern as vcp;
    use nexrad_decode::messages::MessageContents;
    use nexrad_model::data::{
        ChannelConfiguration, ElevationCut, PulseWidth, Radial, Scan, Sweep, VolumeCoveragePattern,
        WaveformType,
    };

    let mut all_radials: Vec<Radial> = Vec::new();
    let mut coverage_pattern_message: Option<vcp::Message<'static>> = None;
    let mut site_location: Option<(f32, f32, i16, u16)> = None;
    let mut site_identifier: Option<[u8; 4]> = None;

    for chunk in chunks {
        match chunk {
            Chunk::Start(file) => {
                // Extract site identifier from the volume header
                if site_identifier.is_none() {
                    if let Some(header) = file.header() {
                        if let Some(icao) = header.icao_of_radar() {
                            let bytes = icao.as_bytes();
                            let mut id = [0u8; 4];
                            let len = bytes.len().min(4);
                            id[..len].copy_from_slice(&bytes[..len]);
                            site_identifier = Some(id);
                        }
                    }
                }

                // Process records from the start chunk
                for record in file.records()? {
                    let record = if record.compressed() {
                        record.decompress()?
                    } else {
                        crate::volume::Record::new(record.data().to_vec())
                    };

                    for message in record.messages()? {
                        match message.into_contents() {
                            MessageContents::DigitalRadarData(m) => {
                                if site_location.is_none() {
                                    if let Some(vol_block) = m.volume_data_block() {
                                        site_location = Some((
                                            vol_block.inner().latitude_raw(),
                                            vol_block.inner().longitude_raw(),
                                            vol_block.inner().site_height_raw(),
                                            vol_block.inner().tower_height_raw(),
                                        ));
                                    }
                                }
                                all_radials.push(m.into_radial()?);
                            }
                            MessageContents::VolumeCoveragePattern(m) => {
                                if coverage_pattern_message.is_none() {
                                    coverage_pattern_message = Some(m.into_owned());
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            Chunk::IntermediateOrEnd(record) => {
                let record = if record.compressed() {
                    record.decompress()?
                } else {
                    crate::volume::Record::new(record.data().to_vec())
                };

                for message in record.messages()? {
                    match message.into_contents() {
                        MessageContents::DigitalRadarData(m) => {
                            if site_location.is_none() {
                                if let Some(vol_block) = m.volume_data_block() {
                                    site_location = Some((
                                        vol_block.inner().latitude_raw(),
                                        vol_block.inner().longitude_raw(),
                                        vol_block.inner().site_height_raw(),
                                        vol_block.inner().tower_height_raw(),
                                    ));
                                }
                            }
                            all_radials.push(m.into_radial()?);
                        }
                        MessageContents::VolumeCoveragePattern(m) => {
                            if coverage_pattern_message.is_none() {
                                coverage_pattern_message = Some(m.into_owned());
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // Build the VCP model
    let vcp_msg = coverage_pattern_message.ok_or(Error::MissingCoveragePattern)?;
    let header = vcp_msg.header();

    let pulse_width = match header.pulse_width() {
        vcp::PulseWidth::Short => PulseWidth::Short,
        vcp::PulseWidth::Long => PulseWidth::Long,
        vcp::PulseWidth::Unknown => PulseWidth::Unknown,
    };

    let elevation_cuts: Vec<ElevationCut> = vcp_msg
        .elevations()
        .iter()
        .map(|elev| {
            let channel_config = match elev.channel_configuration() {
                vcp::ChannelConfiguration::ConstantPhase => ChannelConfiguration::ConstantPhase,
                vcp::ChannelConfiguration::RandomPhase => ChannelConfiguration::RandomPhase,
                vcp::ChannelConfiguration::SZ2Phase => ChannelConfiguration::SZ2Phase,
                vcp::ChannelConfiguration::UnknownPhase => ChannelConfiguration::Unknown,
            };

            let waveform = match elev.waveform_type() {
                vcp::WaveformType::CS => WaveformType::CS,
                vcp::WaveformType::CDW => WaveformType::CDW,
                vcp::WaveformType::CDWO => WaveformType::CDWO,
                vcp::WaveformType::B => WaveformType::B,
                vcp::WaveformType::SPP => WaveformType::SPP,
                vcp::WaveformType::Unknown => WaveformType::Unknown,
            };

            ElevationCut::new(
                elev.elevation_angle(),
                channel_config,
                waveform,
                elev.azimuth_rate(),
                elev.super_resolution_half_degree_azimuth(),
                elev.super_resolution_quarter_km_reflectivity(),
                elev.super_resolution_doppler_to_300km(),
                elev.super_resolution_dual_pol_to_300km(),
                elev.surveillance_prf_number(),
                elev.surveillance_prf_pulse_count_radial(),
                elev.reflectivity_threshold(),
                elev.velocity_threshold(),
                elev.spectrum_width_threshold(),
                elev.differential_reflectivity_threshold(),
                elev.differential_phase_threshold(),
                elev.correlation_coefficient_threshold(),
                elev.is_sails_cut(),
                elev.sails_sequence_number(),
                elev.is_mrle_cut(),
                elev.mrle_sequence_number(),
                elev.is_mpda_cut(),
                elev.is_base_tilt_cut(),
            )
        })
        .collect();

    let coverage_pattern = VolumeCoveragePattern::new(
        header.pattern_number(),
        header.version(),
        header.doppler_velocity_resolution(),
        pulse_width,
        header.is_sails_vcp(),
        header.number_of_sails_cuts(),
        header.is_mrle_vcp(),
        header.number_of_mrle_cuts(),
        header.is_mpda_vcp(),
        header.is_base_tilt_vcp(),
        header.number_of_base_tilts(),
        header.vcp_sequencing_sequence_active(),
        header.vcp_sequencing_truncated(),
        elevation_cuts,
    );

    let sweeps = Sweep::from_radials(all_radials);

    // Build site metadata if available
    let site = site_location.map(|(lat, lon, height, tower)| {
        nexrad_model::meta::Site::new(site_identifier.unwrap_or([0u8; 4]), lat, lon, height, tower)
    });

    match site {
        Some(site) => Ok(Scan::with_site(site, coverage_pattern, sweeps)),
        None => Ok(Scan::new(coverage_pattern, sweeps)),
    }
}

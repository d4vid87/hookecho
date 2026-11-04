//! Small wire fixtures exercise the decoder, rather than constructing already-decoded radials.
#![cfg(not(target_arch = "wasm32"))]

use nexrad_decode::messages::{decode_messages, MessageContents};
use nexrad_model::data::{PulseWidth, Radial, Scan, Sweep, VolumeCoveragePattern};

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_be_bytes());
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
}

fn message_frame(message_type: u8, body: &[u8]) -> Vec<u8> {
    // The decoder input includes the 12-byte communications prefix, then the 16-byte
    // message header. Type 1 occupies a fixed 2432-byte record; Type 31 is variable-length.
    let mut bytes = vec![
        0;
        if message_type == 1 {
            2432
        } else {
            28 + body.len()
        }
    ];
    put_u16(&mut bytes, 12, ((16 + body.len()) / 2) as u16);
    bytes[14] = 8; // ORDA, single channel
    bytes[15] = message_type;
    put_u16(&mut bytes, 16, 1); // sequence
    put_u16(&mut bytes, 18, 20_000); // date
    put_u16(&mut bytes, 24, 1); // segment count
    put_u16(&mut bytes, 26, 1); // segment number
    bytes[28..28 + body.len()].copy_from_slice(body);
    bytes
}

fn msg31(nyquist_cm_per_s: Option<u16>, legacy_rad_block: bool) -> Vec<u8> {
    // Only the radial constants block is needed: no reflectivity or velocity gates.
    let rad_size = if legacy_rad_block { 20 } else { 28 };
    let body_len = if nyquist_cm_per_s.is_some() {
        36 + rad_size
    } else {
        32
    };
    let mut body = vec![0; body_len];
    body[..4].copy_from_slice(b"KTLX");
    put_u32(&mut body, 4, 1_000); // milliseconds after midnight
    put_u16(&mut body, 8, 20_000);
    put_u16(&mut body, 10, 1); // azimuth number
    put_u32(&mut body, 12, 90.0_f32.to_bits());
    put_u16(&mut body, 18, body_len as u16);
    body[20] = 1; // half-degree azimuth spacing
    body[21] = 3; // scan start
    body[22] = 1; // elevation number
    put_u32(&mut body, 24, 0.5_f32.to_bits());
    if let Some(nyquist) = nyquist_cm_per_s {
        put_u16(&mut body, 30, 1); // block count
        put_u32(&mut body, 32, 36); // RAD pointer, relative to the radial header
        body[36..40].copy_from_slice(b"RRAD");
        put_u16(&mut body, 40, rad_size as u16);
        put_u16(&mut body, 42, 4_600); // unambiguous range, deliberately different
        put_u16(&mut body, 52, nyquist); // RAD Nyquist velocity, in hundredths of m/s
        put_u16(&mut body, 54, 1); // flags, not Nyquist
    }
    message_frame(31, &body)
}

fn msg1(nyquist_cm_per_s: i16) -> Vec<u8> {
    let mut body = vec![0; 100];
    put_u32(&mut body, 0, 1_000);
    put_u16(&mut body, 4, 20_000);
    put_u16(&mut body, 6, 4_600); // unambiguous range
    put_u16(&mut body, 8, 16_384); // 90-degree azimuth
    put_u16(&mut body, 10, 1); // azimuth number
    put_u16(&mut body, 12, 3); // scan start
    put_u16(&mut body, 14, 91); // approximately half-degree elevation
    put_u16(&mut body, 16, 1); // elevation number
    put_u16(&mut body, 42, 2); // 0.5 m/s gate resolution is NOT Nyquist
    put_u16(&mut body, 44, 212); // VCP number is NOT Nyquist
                                 // ICD Message Type 1: Nyquist is the signed halfword at body byte 60. Surrounding
                                 // spare bytes are nonzero to catch a field offset mistake even for unknown (zero).
    body[58..64].copy_from_slice(&[0x12, 0x34, 0, 0, 0x56, 0x78]);
    body[60..62].copy_from_slice(&nyquist_cm_per_s.to_be_bytes());
    message_frame(1, &body)
}

fn decode_radial(bytes: &[u8], expected: Option<f32>) -> Radial {
    let mut messages = decode_messages(bytes).expect("decode synthetic radar message");
    assert_eq!(
        messages.len(),
        1,
        "the decoder must not silently skip the fixture"
    );
    let (borrowed, owned) = match messages.pop().unwrap().into_contents() {
        MessageContents::DigitalRadarData(message) => {
            let borrowed = message.radial().unwrap();
            (borrowed, message.into_radial().unwrap())
        }
        MessageContents::DigitalRadarDataLegacy(message) => {
            let borrowed = message.radial().unwrap();
            (borrowed, message.into_radial().unwrap())
        }
        other => panic!("unexpected message: {other:?}"),
    };
    assert_eq!(borrowed.nyquist_velocity_mps(), expected);
    assert_eq!(owned.nyquist_velocity_mps(), expected);
    assert_eq!(owned.clone().nyquist_velocity_mps(), expected);
    assert_eq!(borrowed, owned);
    owned
}

#[test]
fn msg31_modern_and_legacy_rad_constants_preserve_nyquist() {
    for legacy in [false, true] {
        decode_radial(&msg31(Some(3_300), legacy), Some(33.0));
        decode_radial(&msg31(Some(0), legacy), None);
    }
    decode_radial(&msg31(None, false), None);
}

#[test]
fn msg1_reads_signed_nyquist_at_byte_60() {
    decode_radial(&msg1(2_750), Some(27.5));
    decode_radial(&msg1(0), None);
    decode_radial(&msg1(-1), None);
}

#[test]
fn decoded_nyquist_survives_worker_scan_and_sweep_postcard() {
    let radials = vec![
        decode_radial(&msg31(Some(3_300), false), Some(33.0)),
        decode_radial(&msg1(2_750), Some(27.5)),
        decode_radial(&msg31(Some(0), true), None),
    ];
    let sweep = Sweep::new(1, radials);
    let vcp = VolumeCoveragePattern::new(
        212,
        1,
        0.5,
        PulseWidth::Short,
        false,
        0,
        false,
        0,
        false,
        false,
        0,
        false,
        false,
        vec![],
    );
    let scan = Scan::new(vcp, vec![sweep.clone()]);

    // The decode worker returns Scan bytes, and the binning worker receives a Sweep.
    let scan_wire = postcard::to_allocvec(&scan).unwrap();
    let received_scan: Scan = postcard::from_bytes(&scan_wire).unwrap();
    let sweep_wire = postcard::to_allocvec(&sweep).unwrap();
    let received_sweep: Sweep = postcard::from_bytes(&sweep_wire).unwrap();
    assert_eq!(received_scan, scan);
    assert_eq!(received_sweep, sweep);
    for radials in [
        received_scan.sweeps()[0].radials(),
        received_sweep.radials(),
    ] {
        let metadata: Vec<_> = radials.iter().map(Radial::nyquist_velocity_mps).collect();
        assert_eq!(metadata, vec![Some(33.0), Some(27.5), None]);
    }
}

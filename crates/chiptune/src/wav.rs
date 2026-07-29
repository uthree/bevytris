//! Minimal 16-bit mono WAV writer, used only by the offline renderer.

use crate::SAMPLE_RATE;

/// Encode `samples` (nominally -1..1) as a 16-bit mono RIFF/WAVE file.
pub fn encode(samples: &[f32]) -> Vec<u8> {
    let data_len = samples.len() * 2;
    let mut out = Vec::with_capacity(44 + data_len);
    let u32le = |out: &mut Vec<u8>, v: u32| out.extend_from_slice(&v.to_le_bytes());

    out.extend_from_slice(b"RIFF");
    u32le(&mut out, 36 + data_len as u32);
    out.extend_from_slice(b"WAVEfmt ");
    u32le(&mut out, 16); // PCM chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    u32le(&mut out, SAMPLE_RATE);
    u32le(&mut out, SAMPLE_RATE * 2); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits
    out.extend_from_slice(b"data");
    u32le(&mut out, data_len as u32);
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_is_a_valid_riff_wave() {
        let bytes = encode(&[0.0; 100]);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(bytes.len(), 44 + 200);
        assert_eq!(
            u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
            36 + 200
        );
        assert_eq!(
            u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
            SAMPLE_RATE
        );
    }

    #[test]
    fn samples_are_clamped_not_wrapped() {
        let bytes = encode(&[2.0, -2.0]);
        let a = i16::from_le_bytes(bytes[44..46].try_into().unwrap());
        let b = i16::from_le_bytes(bytes[46..48].try_into().unwrap());
        assert_eq!(a, 32767);
        assert_eq!(b, -32767);
    }
}

//! NAL unit reassembly: convert AirPlay-format NALUs to Annex B format.
//!
//! Type 0 (encrypted image data):
//!   Decrypted payload contains one or more NALUs, each prefixed with
//!   4 bytes big-endian length. Convert to Annex B start codes (00 00 00 01).
//!
//! Type 1 (SPS/PPS):
//!   Unencrypted payload with SPS and PPS data. Extract and output with
//!   Annex B start codes.

/// Convert a decrypted Type 0 payload to Annex B format.
///
/// Returns a Vec of bytes with each NALU prefixed by `00 00 00 01`.
pub fn nalus_to_annex_b(payload: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    let mut offset = 0usize;

    while offset + 4 <= payload.len() {
        // Read 4 bytes big-endian length
        let nalu_size = u32::from_be_bytes([
            payload[offset],
            payload[offset + 1],
            payload[offset + 2],
            payload[offset + 3],
        ]) as usize;

        offset += 4;

        // If naluSize == 1, stop (sentinel value)
        if nalu_size <= 1 {
            break;
        }

        // Check bounds
        if offset + nalu_size > payload.len() {
            break;
        }

        // Write Annex B start code
        output.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);

        // Write NALU data
        output.extend_from_slice(&payload[offset..offset + nalu_size]);

        offset += nalu_size;
    }

    output
}

/// Extract SPS and PPS from a Type 1 payload and return as Annex B.
///
/// Type 1 payload layout (big-endian):
///   Offset 0-1: reserved
///   Offset 2-3: reserved
///   Offset 4-5: reserved
///   Offset 6-7: spsLen (u16)
///   Offset 8+:  SPS data (spsLen bytes)
///              ppsCount (1 byte, skip)
///              ppsLen (u16)
///              PPS data (ppsLen bytes)
pub fn extract_sps_pps(payload: &[u8]) -> Option<Vec<u8>> {
    if payload.len() < 10 {
        return None;
    }

    let sps_len = u16::from_be_bytes([payload[6], payload[7]]) as usize;

    if 8 + sps_len > payload.len() {
        return None;
    }

    let sps_data = &payload[8..8 + sps_len];
    let rest = &payload[8 + sps_len..];

    // Skip ppsCount (1 byte)
    if rest.len() < 3 {
        // Only SPS, no PPS
        let mut output = Vec::new();
        output.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        output.extend_from_slice(sps_data);
        return Some(output);
    }

    let pps_len = u16::from_be_bytes([rest[1], rest[2]]) as usize;

    if 3 + pps_len > rest.len() {
        // Only SPS
        let mut output = Vec::new();
        output.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
        output.extend_from_slice(sps_data);
        return Some(output);
    }

    let pps_data = &rest[3..3 + pps_len];

    let mut output = Vec::new();
    output.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
    output.extend_from_slice(sps_data);
    output.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
    output.extend_from_slice(pps_data);

    Some(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nalus_to_annex_b() {
        // Two NALUs: [len=3: 0x01 0x02 0x03] [len=2: 0x04 0x05]
        let payload = [
            0x00, 0x00, 0x00, 0x03, 0x01, 0x02, 0x03,
            0x00, 0x00, 0x00, 0x02, 0x04, 0x05,
        ];
        let result = nalus_to_annex_b(&payload);
        assert_eq!(
            result,
            vec![
                0x00, 0x00, 0x00, 0x01, 0x01, 0x02, 0x03,
                0x00, 0x00, 0x00, 0x01, 0x04, 0x05,
            ]
        );
    }

    #[test]
    fn test_extract_sps_pps() {
        // Minimal SPS-only payload
        let payload = [
            0x00, 0x00, // reserved
            0x00, 0x00, // reserved
            0x00, 0x00, // reserved
            0x00, 0x03, // spsLen = 3
            0x67, 0x42, 0x00, // SPS data
            0x01, // ppsCount = 1
            0x00, 0x02, // ppsLen = 2
            0x68, 0xCE, // PPS data
        ];
        let result = extract_sps_pps(&payload).unwrap();
        let expected = vec![
            0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00,
            0x00, 0x00, 0x00, 0x01, 0x68, 0xCE,
        ];
        assert_eq!(result, expected);
    }
}

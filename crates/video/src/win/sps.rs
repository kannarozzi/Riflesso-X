//! Reads the colour description from an H.264 sequence parameter set.
//!
//! Senders state in the SPS VUI whether samples use the full (0-255) or the
//! studio (16-235) range, and which YCbCr matrix applies. Getting either wrong
//! visibly shifts contrast and colours: iOS, for example, sends full range.

/// How decoded YCbCr samples must be interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorInfo {
    pub full_range: bool,
    /// BT.601 matrix instead of BT.709.
    pub bt601: bool,
}

impl Default for ColorInfo {
    /// What AirPlay senders produce when nothing else is known.
    fn default() -> Self {
        Self { full_range: true, bt601: false }
    }
}

/// Finds an SPS among the NAL units of an Annex-B access unit and reads it.
pub fn find_h264_color(annex_b: &[u8]) -> Option<ColorInfo> {
    let mut i = 0;
    while i + 4 < annex_b.len() {
        if annex_b[i..i + 3] == [0, 0, 1] {
            let start = i + 3;
            if annex_b[start] & 0x1F == 7 {
                let end = (start..annex_b.len().saturating_sub(2))
                    .find(|&j| annex_b[j..j + 3] == [0, 0, 1] || annex_b[j..j + 3] == [0, 0, 0])
                    .unwrap_or(annex_b.len());
                return h264_sps_color(&annex_b[start + 1..end]);
            }
            i = start;
        } else {
            i += 1;
        }
    }
    None
}

/// Parses an SPS payload (after the NAL header byte) up to the VUI video
/// signal type. Returns `None` if the SPS carries no colour information.
pub fn h264_sps_color(payload: &[u8]) -> Option<ColorInfo> {
    let rbsp = remove_emulation_prevention(payload);
    let mut r = BitReader::new(&rbsp);

    let profile = r.bits(8)?;
    r.bits(16)?; // constraint flags, level
    r.ue()?; // seq_parameter_set_id
    if matches!(profile, 100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135) {
        let chroma_format = r.ue()?;
        if chroma_format == 3 {
            r.bits(1)?;
        }
        r.ue()?; // bit_depth_luma_minus8
        r.ue()?; // bit_depth_chroma_minus8
        r.bits(1)?; // qpprime_y_zero_transform_bypass_flag
        if r.bits(1)? == 1 {
            let lists = if chroma_format == 3 { 12 } else { 8 };
            for n in 0..lists {
                if r.bits(1)? == 1 {
                    skip_scaling_list(&mut r, if n < 6 { 16 } else { 64 })?;
                }
            }
        }
    }
    r.ue()?; // log2_max_frame_num_minus4
    match r.ue()? {
        0 => {
            r.ue()?;
        }
        1 => {
            r.bits(1)?;
            r.se()?;
            r.se()?;
            for _ in 0..r.ue()? {
                r.se()?;
            }
        }
        _ => {}
    }
    r.ue()?; // max_num_ref_frames
    r.bits(1)?; // gaps_in_frame_num_value_allowed_flag
    r.ue()?; // pic_width_in_mbs_minus1
    r.ue()?; // pic_height_in_map_units_minus1
    if r.bits(1)? == 0 {
        r.bits(1)?; // mb_adaptive_frame_field_flag
    }
    r.bits(1)?; // direct_8x8_inference_flag
    if r.bits(1)? == 1 {
        for _ in 0..4 {
            r.ue()?;
        }
    }
    if r.bits(1)? == 0 {
        return None; // no VUI
    }
    if r.bits(1)? == 1 && r.bits(8)? == 255 {
        r.bits(32)?; // extended sample aspect ratio
    }
    if r.bits(1)? == 1 {
        r.bits(1)?; // overscan_appropriate_flag
    }
    if r.bits(1)? == 0 {
        return None; // no video_signal_type
    }
    r.bits(3)?; // video_format
    let full_range = r.bits(1)? == 1;
    let mut bt601 = false;
    if r.bits(1)? == 1 {
        r.bits(16)?; // colour_primaries, transfer_characteristics
        bt601 = matches!(r.bits(8)?, 5 | 6);
    }
    Some(ColorInfo { full_range, bt601 })
}

fn skip_scaling_list(r: &mut BitReader, size: usize) -> Option<()> {
    let (mut last, mut next) = (8i64, 8i64);
    for _ in 0..size {
        if next != 0 {
            next = (last + r.se()? + 256) % 256;
        }
        if next != 0 {
            last = next;
        }
    }
    Some(())
}

fn remove_emulation_prevention(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut zeros = 0;
    for &b in data {
        if zeros >= 2 && b == 3 {
            zeros = 0;
            continue;
        }
        zeros = if b == 0 { zeros + 1 } else { 0 };
        out.push(b);
    }
    out
}

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn bits(&mut self, n: u32) -> Option<u64> {
        let mut v = 0u64;
        for _ in 0..n {
            let byte = *self.data.get(self.pos / 8)?;
            v = (v << 1) | u64::from((byte >> (7 - self.pos % 8)) & 1);
            self.pos += 1;
        }
        Some(v)
    }

    fn ue(&mut self) -> Option<u64> {
        let mut zeros = 0;
        while self.bits(1)? == 0 {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        Some((1u64 << zeros) - 1 + self.bits(zeros)?)
    }

    fn se(&mut self) -> Option<i64> {
        let k = self.ue()? as i64;
        Some(if k % 2 == 1 { (k + 1) / 2 } else { -(k / 2) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SPS sent by an iPhone (iOS 26) when mirroring in portrait.
    const IPHONE_SPS: [u8; 18] =
        [0x27, 0x64, 0x00, 0x20, 0xac, 0x13, 0x14, 0x50, 0x20, 0x02, 0x27, 0x88, 0x96, 0x6e, 0x02, 0x1a, 0x02, 0x04];

    #[test]
    fn iphone_sends_full_range_bt709() {
        assert_eq!(h264_sps_color(&IPHONE_SPS[1..]), Some(ColorInfo { full_range: true, bt601: false }));
    }

    #[test]
    fn finds_sps_in_access_unit() {
        let mut au = vec![0, 0, 0, 1];
        au.extend_from_slice(&IPHONE_SPS);
        au.extend_from_slice(&[0, 0, 0, 1, 0x28, 0xee, 0x3c, 0xb0]);
        assert_eq!(find_h264_color(&au), Some(ColorInfo { full_range: true, bt601: false }));
    }

    #[test]
    fn no_sps_no_color() {
        assert_eq!(find_h264_color(&[0, 0, 0, 1, 0x25, 0xb8, 0x00]), None);
    }
}

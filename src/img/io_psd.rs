//! Minimal PSD reader: the merged (flattened) image of RGB documents with 8
//! or 16 bits per channel, raw or PackBits compressed. This is the subset
//! stb_image supports, including its handling of missing channels and the
//! white matte Photoshop applies to the merged image of documents with
//! transparency.
//!
//! All header fields are untrusted and checked against the buffer.

use super::image::{Image, check_dimensions};
use crate::common::{Error, Result};

/// Bounds checked big endian reader over the file.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    name: &'a str,
}

impl<'a> Reader<'a> {
    fn corrupt(&self, what: &str) -> Error {
        Error::runtime(format!("'{}': corrupt PSD file ({what})", self.name))
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.data.len()).ok_or_else(|| self.corrupt("truncated"))?;
        let s = &self.data[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// Skips a section preceded by its 32-bit length.
    fn skip_section(&mut self) -> Result<()> {
        let len = self.u32()? as usize;
        self.take(len).map(|_| ())
    }
}

/// Header facts needed before decoding.
struct PsdInfo {
    width: i32,
    height: i32,
    depth: u16,
    channels: u16,
}

pub fn is_psd(bytes: &[u8]) -> bool {
    bytes.starts_with(b"8BPS")
}

fn read_header(r: &mut Reader) -> Result<PsdInfo> {
    if r.take(4)? != b"8BPS" {
        return Err(r.corrupt("bad signature"));
    }
    if r.u16()? != 1 {
        return Err(r.corrupt("unsupported version"));
    }
    r.take(6)?; // reserved
    let channels = r.u16()?;
    if channels > 16 {
        return Err(r.corrupt("too many channels"));
    }
    let height = r.u32()?;
    let width = r.u32()?;
    check_dimensions(width as i64, height as i64, r.name)?;
    let depth = r.u16()?;
    if depth != 8 && depth != 16 {
        return Err(Error::runtime(format!("'{}': unsupported PSD bit depth {depth} (8 or 16 supported)", r.name)));
    }
    if r.u16()? != 3 {
        return Err(Error::runtime(format!("'{}': unsupported PSD color mode (only RGB is supported)", r.name)));
    }
    Ok(PsdInfo { width: width as i32, height: height as i32, depth, channels })
}

/// Decodes PackBits data until `out` is full. A packet that would overrun
/// `out` is an error, so crafted counts cannot write out of bounds.
fn unpack_bits(r: &mut Reader, out: &mut [u8]) -> Result<()> {
    let mut pos = 0;
    while pos < out.len() {
        let n = r.u8()?;
        match n {
            // No-op packet.
            128 => {}
            0..=127 => {
                let len = n as usize + 1;
                if len > out.len() - pos {
                    return Err(r.corrupt("bad RLE data"));
                }
                out[pos..pos + len].copy_from_slice(r.take(len)?);
                pos += len;
            }
            _ => {
                let len = 257 - n as usize;
                if len > out.len() - pos {
                    return Err(r.corrupt("bad RLE data"));
                }
                let v = r.u8()?;
                out[pos..pos + len].fill(v);
                pos += len;
            }
        }
    }
    Ok(())
}

/// Decodes the merged image. Returns the image with values in 0..1 and
/// the bit depth (8 or 16).
pub fn decode_psd(bytes: &[u8], name: &str) -> Result<(Image, i32)> {
    let mut r = Reader { data: bytes, pos: 0, name };
    let info = read_header(&mut r)?;
    r.skip_section()?; // color mode data
    r.skip_section()?; // image resources
    r.skip_section()?; // layer and mask info
    let compression = r.u16()?;
    if compression > 1 {
        return Err(Error::runtime(format!("'{name}': unsupported PSD compression {compression}")));
    }

    let pixels = info.width as usize * info.height as usize;
    let sample = (info.depth / 8) as usize;
    let plane_len = pixels * sample;
    if compression == 1 {
        // Byte counts of every row of every channel; the PackBits stream is
        // decoded continuously per channel, so they are not needed.
        let rows = info.height as usize * info.channels as usize;
        r.take(rows * 2)?;
    }

    let mut img = Image::new(info.width, info.height);
    let max = if info.depth == 16 { 65535.0f32 } else { 255.0f32 };
    let mut plane = vec![0u8; plane_len];
    // Image data holds the channels one after another; only the first four
    // (r, g, b, alpha) are used, missing ones become 0 (color) or 1 (alpha).
    for c in 0..4usize {
        if c >= info.channels as usize {
            let v = if c == 3 { 1.0 } else { 0.0 };
            img.data.iter_mut().skip(c).step_by(4).for_each(|d| *d = v);
            continue;
        }
        if compression == 1 {
            unpack_bits(&mut r, &mut plane)?;
        } else {
            plane.copy_from_slice(r.take(plane_len)?);
        }
        for (d, s) in img.data.iter_mut().skip(c).step_by(4).zip(plane.chunks_exact(sample)) {
            let v = if sample == 2 { u16::from_be_bytes([s[0], s[1]]) as f32 } else { s[0] as f32 };
            *d = v / max;
        }
    }

    // Photoshop blends the merged image of documents with transparency
    // against white; undo that like stb_image does.
    if info.channels >= 4 {
        for p in img.data.chunks_exact_mut(4) {
            let a = p[3];
            if a != 0.0 && a != 1.0 {
                let ra = 1.0 / a;
                let inv_a = 1.0 - ra;
                for v in &mut p[..3] {
                    *v = (*v * ra + inv_a).clamp(0.0, 1.0);
                }
            }
        }
    }
    Ok((img, info.depth as i32))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn psd(channels: u16, w: u32, h: u32, depth: u16, compression: u16, data: &[u8]) -> Vec<u8> {
        let mut b = b"8BPS".to_vec();
        b.extend_from_slice(&1u16.to_be_bytes());
        b.extend_from_slice(&[0; 6]);
        b.extend_from_slice(&channels.to_be_bytes());
        b.extend_from_slice(&h.to_be_bytes());
        b.extend_from_slice(&w.to_be_bytes());
        b.extend_from_slice(&depth.to_be_bytes());
        b.extend_from_slice(&3u16.to_be_bytes());
        for _ in 0..3 {
            b.extend_from_slice(&0u32.to_be_bytes());
        }
        b.extend_from_slice(&compression.to_be_bytes());
        b.extend_from_slice(data);
        b
    }

    #[test]
    fn raw_rgb() {
        let (img, bits) = decode_psd(&psd(3, 2, 1, 8, 0, &[255, 0, 0, 255, 0, 0]), "t.psd").unwrap();
        assert_eq!(bits, 8);
        assert_eq!(img.data, vec![1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0]);
    }

    #[test]
    fn rle_rgb() {
        // 2x1, 3 channels: 2 row counts each, then a run of 2 per channel.
        let mut d = vec![0, 2, 0, 2, 0, 2];
        d.extend_from_slice(&[255, 10, 255, 20, 255, 30]);
        let (img, _) = decode_psd(&psd(3, 2, 1, 8, 1, &d), "t.psd").unwrap();
        assert_eq!(img.data[0], 10.0 / 255.0);
        assert_eq!(img.data[5], 20.0 / 255.0);
        assert_eq!(img.data[6], 30.0 / 255.0);
    }

    #[test]
    fn hostile_inputs_fail_cleanly() {
        // Truncated pixel data.
        assert!(decode_psd(&psd(3, 4, 4, 8, 0, &[0; 10]), "t.psd").is_err());
        // RLE run that overruns the plane.
        assert!(decode_psd(&psd(1, 2, 1, 8, 1, &[0, 2, 0x81, 7]), "t.psd").is_err());
        // Zero and huge dimensions.
        assert!(decode_psd(&psd(3, 0, 1, 8, 0, &[]), "t.psd").is_err());
        assert!(decode_psd(&psd(3, 70000, 1, 8, 0, &[]), "t.psd").is_err());
        assert!(decode_psd(&psd(3, 65536, 65536, 8, 0, &[]), "t.psd").is_err());
        // Bad depth and section lengths beyond the end of the file.
        assert!(decode_psd(&psd(3, 1, 1, 32, 0, &[0; 12]), "t.psd").is_err());
        let mut b = psd(3, 1, 1, 8, 0, &[0; 3]);
        b[26..30].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(decode_psd(&b, "t.psd").is_err());
        assert!(decode_psd(&b[..20], "t.psd").is_err());
    }
}

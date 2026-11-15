//! A small, dependency-free PSD reader.
//!
//! Lumen shows the flattened picture that Photoshop stores at the end of every
//! PSD file — the "composite image data" — and not the individual layers. A
//! viewer has a single canvas, and the composite is exactly what Photoshop
//! shows when every layer is visible, so this is both the correct picture and
//! by far the cheapest one to produce: the layer section is skipped entirely.
//!
//! Supported: PSD (version 1), RGB and grayscale, 8 and 16 bits per channel,
//! uncompressed and RLE-compressed data. Everything else (PSB, CMYK, Lab,
//! indexed, 32-bit float) returns a clear error instead of a broken image.

/// Largest picture that will be decoded. A PSD is allowed to be huge, but
/// allocating four bytes per pixel for a 500 megapixel file would be a
/// 2 GB allocation, so it is refused with an explanation.
const MAX_PIXELS: u64 = 120_000_000;

/// A decoded composite image, straight RGBA.
#[derive(Debug)]
pub struct PsdImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Decodes the composite image of a PSD file.
pub fn decode(bytes: &[u8]) -> Result<PsdImage, String> {
    let mut reader = Reader::new(bytes);

    if reader.take(4)? != b"8BPS" {
        return Err("psd: missing the 8BPS signature".to_owned());
    }
    let version = reader.u16()?;
    if version != 1 {
        return Err(format!(
            "psd: version {version} (PSB) is not supported, only PSD version 1"
        ));
    }
    reader.skip(6)?; // reserved

    let channels = reader.u16()? as usize;
    let height = reader.u32()?;
    let width = reader.u32()?;
    let depth = reader.u16()?;
    let color_mode = reader.u16()?;

    if width == 0 || height == 0 {
        return Err("psd: the canvas has no size".to_owned());
    }
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(format!("psd: {width}x{height} is too large to open"));
    }
    if !(1..=56).contains(&channels) {
        return Err(format!("psd: {channels} channels is not a valid count"));
    }
    let sample_bytes = match depth {
        8 => 1,
        16 => 2,
        32 => return Err("psd: 32-bit (floating point) files are not supported".to_owned()),
        other => return Err(format!("psd: {other} bits per channel is not valid")),
    };
    // 1 = grayscale, 3 = RGB. Everything else needs colour management that a
    // viewer has no business guessing at.
    let base_channels = match color_mode {
        1 => 1,
        3 => 3,
        other => {
            return Err(format!(
                "psd: colour mode {other} is not supported (only RGB and grayscale)"
            ))
        }
    };
    let has_alpha = channels > base_channels;

    // Colour mode data, image resources and layer info are not needed for the
    // composite, but they have to be stepped over to reach it.
    for _ in 0..3 {
        let length = reader.u32()? as usize;
        reader.skip(length)?;
    }

    let compression = reader.u16()?;
    let pixels = u64::from(width) * u64::from(height);
    let row_bytes = width as usize * sample_bytes;

    // One buffer per channel that ends up in the picture; extra channels (spot
    // colours, layer masks) are read and thrown away.
    let mut planes: Vec<Vec<u8>> = Vec::with_capacity(base_channels + 1);
    let wanted = base_channels + usize::from(has_alpha);

    match compression {
        0 => {
            for index in 0..channels {
                let length = pixels as usize * sample_bytes;
                let data = reader.take(length)?;
                if index < wanted {
                    planes.push(data.to_vec());
                }
            }
        }
        1 => {
            // One row byte-count per channel per row, then the packed rows.
            let mut counts = Vec::with_capacity(channels * height as usize);
            for _ in 0..channels * height as usize {
                counts.push(reader.u16()? as usize);
            }
            for index in 0..channels {
                let mut plane = vec![0u8; row_bytes * height as usize];
                for row in 0..height as usize {
                    let count = counts[index * height as usize + row];
                    let packed = reader.take(count)?;
                    let start = row * row_bytes;
                    unpack_row(packed, &mut plane[start..start + row_bytes])?;
                }
                if index < wanted {
                    planes.push(plane);
                }
            }
        }
        other => {
            return Err(format!(
                "psd: compression {other} is not supported (only raw and RLE)"
            ))
        }
    }

    // Planar -> interleaved RGBA.
    let mut out = vec![0u8; pixels as usize * 4];
    for index in 0..pixels as usize {
        let read = |plane: usize| -> u8 {
            // 16-bit samples are big endian; the high byte is the one that
            // survives the conversion to 8 bits per channel.
            planes[plane][index * sample_bytes]
        };
        let (red, green, blue) = if base_channels == 3 {
            (read(0), read(1), read(2))
        } else {
            let gray = read(0);
            (gray, gray, gray)
        };
        let alpha = if has_alpha {
            read(base_channels)
        } else {
            255
        };
        let offset = index * 4;
        out[offset] = red;
        out[offset + 1] = green;
        out[offset + 2] = blue;
        out[offset + 3] = alpha;
    }

    Ok(PsdImage {
        width,
        height,
        pixels: out,
    })
}

/// Expands one PackBits-compressed row into `out`.
///
/// The format is the one TIFF and PSD share: a length byte followed by that
/// many literal bytes when positive, a repeat count when negative, and `-128`
/// as a no-op.
fn unpack_row(packed: &[u8], out: &mut [u8]) -> Result<(), String> {
    let mut input = 0usize;
    let mut output = 0usize;

    while output < out.len() {
        let header = *packed
            .get(input)
            .ok_or_else(|| "psd: a compressed row ended early".to_owned())?;
        input += 1;

        if header < 128 {
            // Literal run of `header + 1` bytes.
            let count = header as usize + 1;
            let end = input + count;
            if end > packed.len() || output + count > out.len() {
                return Err("psd: a literal run runs past the end of the row".to_owned());
            }
            out[output..output + count].copy_from_slice(&packed[input..end]);
            input = end;
            output += count;
        } else if header != 128 {
            // Repeat run: 257 - header copies of the next byte.
            let count = 257 - header as usize;
            let value = *packed
                .get(input)
                .ok_or_else(|| "psd: a repeat run ended early".to_owned())?;
            input += 1;
            if output + count > out.len() {
                return Err("psd: a repeat run runs past the end of the row".to_owned());
            }
            out[output..output + count].fill(value);
            output += count;
        }
        // 128 is a no-op and simply skipped.
    }

    Ok(())
}

/// Big-endian cursor over the file, with bounds checks on every read.
struct Reader<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], String> {
        let end = self
            .position
            .checked_add(count)
            .ok_or_else(|| "psd: length overflow".to_owned())?;
        if end > self.data.len() {
            return Err("psd: the file ends in the middle of a section".to_owned());
        }
        let slice = &self.data[self.position..end];
        self.position = end;
        Ok(slice)
    }

    fn skip(&mut self, count: usize) -> Result<(), String> {
        self.take(count).map(|_| ())
    }

    fn u16(&mut self) -> Result<u16, String> {
        let bytes = self.take(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&mut self) -> Result<u32, String> {
        let bytes = self.take(4)?;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a PSD file the way Photoshop writes one, minus the layers.
    ///
    /// `rle` selects the RLE compression path, which is what real files use;
    /// the raw path exists mainly for very small images.
    fn build_psd(width: u32, height: u32, planes: &[Vec<u8>], rle: bool) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"8BPS");
        out.extend_from_slice(&1u16.to_be_bytes()); // version
        out.extend_from_slice(&[0; 6]); // reserved
        out.extend_from_slice(&(planes.len() as u16).to_be_bytes());
        out.extend_from_slice(&height.to_be_bytes());
        out.extend_from_slice(&width.to_be_bytes());
        out.extend_from_slice(&8u16.to_be_bytes()); // depth
        out.extend_from_slice(&3u16.to_be_bytes()); // RGB

        // Empty colour mode data, image resources and layer info.
        for _ in 0..3 {
            out.extend_from_slice(&0u32.to_be_bytes());
        }

        if !rle {
            out.extend_from_slice(&0u16.to_be_bytes());
            for plane in planes {
                out.extend_from_slice(plane);
            }
            return out;
        }

        out.extend_from_slice(&1u16.to_be_bytes());
        // Every row is stored as literal runs of at most 128 bytes, because a
        // length byte above 127 would mean "repeat" instead. Each run costs one
        // header byte on top of the data, and the row count includes those.
        let chunks_per_row = (width as usize).div_ceil(128);
        let row_length = (width as usize + chunks_per_row) as u16;
        for _ in planes {
            for _ in 0..height {
                out.extend_from_slice(&row_length.to_be_bytes());
            }
        }
        for plane in planes {
            for row in 0..height as usize {
                let start = row * width as usize;
                for chunk in plane[start..start + width as usize].chunks(128) {
                    out.push((chunk.len() - 1) as u8);
                    out.extend_from_slice(chunk);
                }
            }
        }
        out
    }

    fn planes_for(width: u32, height: u32) -> Vec<Vec<u8>> {
        let count = (width * height) as usize;
        let mut red = Vec::with_capacity(count);
        let mut green = Vec::with_capacity(count);
        let mut blue = Vec::with_capacity(count);
        let mut alpha = Vec::with_capacity(count);
        for index in 0..count {
            red.push((index % 251) as u8);
            green.push((index % 17) as u8);
            blue.push(200);
            alpha.push(if index % 2 == 0 { 255 } else { 64 });
        }
        vec![red, green, blue, alpha]
    }

    #[test]
    fn decodes_an_uncompressed_file() {
        let planes = planes_for(7, 5);
        let file = build_psd(7, 5, &planes, false);
        let image = decode(&file).expect("should decode");
        assert_eq!((image.width, image.height), (7, 5));
        assert_eq!(image.pixels.len(), 7 * 5 * 4);
        assert_eq!(&image.pixels[0..4], &[0, 0, 200, 255]);
        // Second pixel: index 1 -> red 1, green 1, blue 200, alpha 64.
        assert_eq!(&image.pixels[4..8], &[1, 1, 200, 64]);
    }

    #[test]
    fn decodes_an_rle_compressed_file() {
        let planes = planes_for(9, 6);
        let file = build_psd(9, 6, &planes, true);
        let image = decode(&file).expect("should decode");
        assert_eq!((image.width, image.height), (9, 6));
        for index in 0..(9 * 6) as usize {
            let offset = index * 4;
            assert_eq!(image.pixels[offset], (index % 251) as u8, "red at {index}");
            assert_eq!(image.pixels[offset + 2], 200, "blue at {index}");
        }
    }

    #[test]
    fn a_missing_alpha_channel_is_opaque() {
        let planes = planes_for(4, 4);
        let file = build_psd(4, 4, &planes[0..3], true);
        let image = decode(&file).expect("should decode");
        assert!(image.pixels.chunks(4).all(|pixel| pixel[3] == 255));
    }

    #[test]
    fn garbage_is_rejected_with_a_message() {
        assert!(decode(b"not a psd at all").is_err());
        let mut file = build_psd(4, 4, &planes_for(4, 4), true);
        file.truncate(file.len() / 2);
        let error = decode(&file).expect_err("truncated file must fail");
        assert!(error.contains("psd:"), "unhelpful error: {error}");
    }

    #[test]
    fn packbits_handles_repeat_runs_and_literals() {
        // 0xFE = repeat next byte 3 times, then a literal run of 2 bytes.
        let packed = [0xFE, 0x07, 0x01, 0xAA, 0xBB];
        let mut out = [0u8; 5];
        unpack_row(&packed, &mut out).expect("should unpack");
        assert_eq!(out, [7, 7, 7, 0xAA, 0xBB]);
    }
}

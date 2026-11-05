//! Identification of Gaussian-splat files by content, never by extension alone.
//!
//! Spark (the runtime renderer, see `docs/features/spark.md`) decodes `.splat`, Gaussian
//! `.ply` and `.spz`. The core only needs to know *which* of them a file is and how many
//! splats it holds, so a generic point-cloud PLY or a random binary is rejected at import
//! with a typed error instead of failing later inside the renderer.

use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use flate2::read::GzDecoder;

use crate::assets::SPLAT_STRIDE;

/// Container formats of Gaussian splats the runtime has been proven with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplatFormat {
    /// 32-byte records: position f32x3, scale f32x3, color u8x4, rotation u8x4.
    Splat,
    /// Binary little-endian PLY with the 3D Gaussian Splatting vertex properties.
    Ply,
    /// Niantic SPZ: gzip of a fixed header plus structure-of-arrays payload.
    Spz,
}

impl SplatFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Splat => "splat",
            Self::Ply => "ply",
            Self::Spz => "spz",
        }
    }

    pub fn parse(label: &str) -> Option<Self> {
        [Self::Splat, Self::Ply, Self::Spz]
            .into_iter()
            .find(|format| format.label() == label)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SplatInfo {
    pub format: SplatFormat,
    pub count: u64,
}

/// Properties a PLY must declare on its `vertex` element to be a 3D Gaussian splat.
const PLY_REQUIRED: [&str; 14] = [
    "x", "y", "z", "f_dc_0", "f_dc_1", "f_dc_2", "opacity", "scale_0", "scale_1", "scale_2",
    "rot_0", "rot_1", "rot_2", "rot_3",
];

/// Upper bound of a PLY header, so a hostile file cannot make the probe read forever.
const PLY_HEADER_LIMIT: usize = 16 * 1024;

const SPZ_MAGIC: u32 = 0x5053_474e;

/// Identifies `path`. `Err` carries a human message; callers wrap it in their typed error.
pub fn inspect(path: &Path) -> Result<SplatInfo, String> {
    let length = std::fs::metadata(path)
        .map_err(|error| format!("Cannot read `{}`: {error}", path.display()))?
        .len();
    let mut file =
        File::open(path).map_err(|error| format!("Cannot read `{}`: {error}", path.display()))?;
    let mut magic = [0u8; 4];
    let read = file
        .read(&mut magic)
        .map_err(|error| format!("Cannot read `{}`: {error}", path.display()))?;
    drop(file);
    if read >= 3 && &magic[..3] == b"ply" {
        return inspect_ply(path);
    }
    if read >= 2 && magic[..2] == [0x1f, 0x8b] {
        return inspect_spz(path);
    }
    inspect_raw(length)
}

fn inspect_raw(length: u64) -> Result<SplatInfo, String> {
    if length == 0 || !length.is_multiple_of(SPLAT_STRIDE) {
        return Err(format!(
            "Not a Gaussian splat: a .splat file is a multiple of {SPLAT_STRIDE} bytes, got {length}"
        ));
    }
    Ok(SplatInfo {
        format: SplatFormat::Splat,
        count: length / SPLAT_STRIDE,
    })
}

fn inspect_ply(path: &Path) -> Result<SplatInfo, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    let mut reader = BufReader::new(file);
    let mut header_bytes = 0usize;
    let mut line = String::new();
    let mut lines = Vec::new();
    loop {
        line.clear();
        let read = reader
            .read_line(&mut line)
            .map_err(|error| format!("Cannot read the PLY header: {error}"))?;
        if read == 0 {
            return Err("The PLY header has no `end_header`".into());
        }
        header_bytes += read;
        if header_bytes > PLY_HEADER_LIMIT {
            return Err("The PLY header is larger than 16 KiB".into());
        }
        let trimmed = line.trim_end().to_string();
        if trimmed == "end_header" {
            break;
        }
        lines.push(trimmed);
    }
    parse_ply_header(&lines)
}

/// Checks the header lines (without `end_header`). Public for the unit tests of adapters.
pub fn parse_ply_header(lines: &[String]) -> Result<SplatInfo, String> {
    if lines.first().map(String::as_str) != Some("ply") {
        return Err("Not a PLY file".into());
    }
    if !lines
        .iter()
        .any(|line| line.starts_with("format binary_little_endian"))
    {
        return Err("Only `binary_little_endian` Gaussian PLY files are supported".into());
    }
    let mut count = None;
    let mut properties = Vec::new();
    let mut in_vertex = false;
    for line in lines {
        let parts: Vec<&str> = line.split_whitespace().collect();
        match parts.as_slice() {
            ["element", name, n] => {
                in_vertex = *name == "vertex";
                if in_vertex {
                    count = Some(
                        n.parse::<u64>()
                            .map_err(|_| format!("Invalid vertex count `{n}`"))?,
                    );
                } else if *name == "chunk" {
                    return Err(
                        "Compressed (chunked) PLY is not supported; export a plain Gaussian PLY"
                            .into(),
                    );
                }
            }
            ["property", _, name] if in_vertex => properties.push(*name),
            _ => {}
        }
    }
    let count = count
        .filter(|count| *count > 0)
        .ok_or("The PLY has no `vertex` element")?;
    let missing: Vec<&str> = PLY_REQUIRED
        .into_iter()
        .filter(|name| !properties.contains(name))
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "This PLY is not a Gaussian splat: missing vertex properties {}",
            missing.join(", ")
        ));
    }
    Ok(SplatInfo {
        format: SplatFormat::Ply,
        count,
    })
}

fn inspect_spz(path: &Path) -> Result<SplatInfo, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    let mut header = [0u8; 16];
    GzDecoder::new(file)
        .read_exact(&mut header)
        .map_err(|error| format!("Not a valid SPZ (gzip) file: {error}"))?;
    parse_spz_header(&header)
}

/// Decodes the 16-byte SPZ header: magic, version, point count, SH degree, fractional bits.
pub fn parse_spz_header(header: &[u8; 16]) -> Result<SplatInfo, String> {
    let word = |at: usize| {
        u32::from_le_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]])
    };
    if word(0) != SPZ_MAGIC {
        return Err("The gzip stream is not an SPZ file (bad magic)".into());
    }
    let version = word(4);
    if !(2..=3).contains(&version) {
        return Err(format!("SPZ version {version} is not supported (2 or 3)"));
    }
    let count = u64::from(word(8));
    if count == 0 {
        return Err("The SPZ has no splats".into());
    }
    Ok(SplatInfo {
        format: SplatFormat::Spz,
        count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    #[test]
    fn gaussian_ply_headers_are_accepted_and_generic_point_clouds_are_not() {
        let gaussian = lines(
            "ply\nformat binary_little_endian 1.0\nelement vertex 12\nproperty float x\nproperty float y\nproperty float z\nproperty float f_dc_0\nproperty float f_dc_1\nproperty float f_dc_2\nproperty float opacity\nproperty float scale_0\nproperty float scale_1\nproperty float scale_2\nproperty float rot_0\nproperty float rot_1\nproperty float rot_2\nproperty float rot_3",
        );
        let info = parse_ply_header(&gaussian).unwrap();
        assert_eq!((info.format, info.count), (SplatFormat::Ply, 12));

        let points = lines(
            "ply\nformat binary_little_endian 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z",
        );
        let error = parse_ply_header(&points).unwrap_err();
        assert!(error.contains("not a Gaussian splat"), "{error}");
        assert!(error.contains("opacity"), "{error}");

        let ascii = lines("ply\nformat ascii 1.0\nelement vertex 3");
        assert!(
            parse_ply_header(&ascii)
                .unwrap_err()
                .contains("binary_little_endian")
        );
    }

    #[test]
    fn spz_header_needs_magic_version_and_points() {
        let mut header = [0u8; 16];
        header[..4].copy_from_slice(&SPZ_MAGIC.to_le_bytes());
        header[4..8].copy_from_slice(&2u32.to_le_bytes());
        header[8..12].copy_from_slice(&500u32.to_le_bytes());
        assert_eq!(parse_spz_header(&header).unwrap().count, 500);
        header[4..8].copy_from_slice(&9u32.to_le_bytes());
        assert!(parse_spz_header(&header).unwrap_err().contains("version 9"));
        assert!(parse_spz_header(&[0u8; 16]).unwrap_err().contains("magic"));
    }

    #[test]
    fn raw_splat_length_must_be_a_multiple_of_the_record() {
        assert_eq!(inspect_raw(64).unwrap().count, 2);
        assert!(inspect_raw(65).is_err());
        assert!(inspect_raw(0).is_err());
    }
}

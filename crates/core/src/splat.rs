//! Identification of Gaussian-splat files by content, never by extension alone.
//!
//! Spark (the runtime renderer, see `docs/features/spark.md`) decodes `.splat`, Gaussian
//! `.ply` and `.spz`. The core only needs to know *which* of them a file is and how many
//! splats it holds, so a generic point-cloud PLY or a random binary is rejected at import
//! with a typed error instead of failing later inside the renderer.

use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
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

/// How many splat positions [`measure`] looks at at most: an even stride over the file, so the
/// cost does not grow with a 1.2-million-splat capture and the answer is the same every time.
pub const MEASURE_SAMPLES: u64 = 20_000;
/// Fraction of the sampled positions each end of an axis may discard, so a few stray floaters
/// far from the scene do not decide where the camera looks.
const OUTLIER_FRACTION: f64 = 0.02;

/// Where a cloud sits in the coordinates of its file (the frame the renderer places the mesh
/// in): a box that holds the bulk of the splats, never the stray ones.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Extent {
    pub format: SplatFormat,
    /// Splats in the file.
    pub count: u64,
    /// Finite positions the box was computed from (an even sample of `count`).
    pub measured: u64,
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl Extent {
    pub fn center(&self) -> [f64; 3] {
        std::array::from_fn(|axis| (self.min[axis] + self.max[axis]) / 2.0)
    }

    /// Half of the box diagonal: the radius of the sphere that holds the box.
    pub fn radius(&self) -> f64 {
        let squares: f64 = (0..3)
            .map(|axis| (self.max[axis] - self.min[axis]).powi(2))
            .sum();
        squares.sqrt() / 2.0
    }

    /// A box from sampled positions: the `OUTLIER_FRACTION` tails of each axis are dropped.
    /// `None` when no position is finite.
    pub fn from_positions(
        format: SplatFormat,
        count: u64,
        positions: impl IntoIterator<Item = [f64; 3]>,
    ) -> Option<Self> {
        let mut axes: [Vec<f64>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        for position in positions {
            if position.iter().all(|value| value.is_finite()) {
                for (axis, values) in axes.iter_mut().enumerate() {
                    values.push(position[axis]);
                }
            }
        }
        let measured = axes[0].len() as u64;
        if measured == 0 {
            return None;
        }
        let mut min = [0.0; 3];
        let mut max = [0.0; 3];
        for (axis, values) in axes.iter_mut().enumerate() {
            values.sort_by(f64::total_cmp);
            let last = values.len() - 1;
            let cut = (last as f64 * OUTLIER_FRACTION).floor() as usize;
            min[axis] = values[cut];
            max[axis] = values[last - cut];
        }
        Some(Self {
            format,
            count,
            measured,
            min,
            max,
        })
    }
}

/// Measures where the cloud of `path` sits, from the file alone (no renderer). Reads an even
/// sample of the positions: `.splat` and binary Gaussian PLY exactly as stored, SPZ through its
/// 24-bit fixed-point positions. `Err` carries a human message.
pub fn measure(path: &Path) -> Result<Extent, String> {
    let info = inspect(path)?;
    let step = info.count.div_ceil(MEASURE_SAMPLES).max(1);
    let positions = match info.format {
        SplatFormat::Splat => raw_positions(path, info.count, step)?,
        SplatFormat::Ply => ply_positions(path, step)?,
        SplatFormat::Spz => spz_positions(path, step)?,
    };
    Extent::from_positions(info.format, info.count, positions)
        .ok_or_else(|| format!("`{}` has no finite splat position", path.display()))
}

fn read_error(path: &Path, error: &std::io::Error) -> String {
    format!("Cannot read `{}`: {error}", path.display())
}

fn f32_at(record: &[u8], at: usize) -> f64 {
    f64::from(f32::from_le_bytes([
        record[at],
        record[at + 1],
        record[at + 2],
        record[at + 3],
    ]))
}

fn raw_positions(path: &Path, count: u64, step: u64) -> Result<Vec<[f64; 3]>, String> {
    let mut file = File::open(path).map_err(|error| read_error(path, &error))?;
    let mut positions = Vec::new();
    let mut record = [0u8; 12];
    let mut index = 0;
    while index < count {
        file.seek(SeekFrom::Start(index * SPLAT_STRIDE))
            .and_then(|_| file.read_exact(&mut record))
            .map_err(|error| read_error(path, &error))?;
        positions.push([f32_at(&record, 0), f32_at(&record, 4), f32_at(&record, 8)]);
        index += step;
    }
    Ok(positions)
}

/// Where the `x`, `y`, `z` floats of a PLY vertex sit, and how long a vertex is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlyLayout {
    pub count: u64,
    pub stride: usize,
    pub xyz: [usize; 3],
}

fn ply_type_size(name: &str) -> Option<usize> {
    match name {
        "char" | "uchar" | "int8" | "uint8" => Some(1),
        "short" | "ushort" | "int16" | "uint16" => Some(2),
        "int" | "uint" | "float" | "int32" | "uint32" | "float32" => Some(4),
        "double" | "float64" => Some(8),
        _ => None,
    }
}

/// The vertex layout of a PLY header (lines without `end_header`). The vertex element must come
/// first and hold only fixed-size scalars, `x`, `y` and `z` being 32-bit floats.
pub fn ply_layout(lines: &[String]) -> Result<PlyLayout, String> {
    let info = parse_ply_header(lines)?;
    let mut offset = 0usize;
    let mut xyz: [Option<usize>; 3] = [None; 3];
    let mut elements = 0;
    let mut in_vertex = false;
    for line in lines {
        let parts: Vec<&str> = line.split_whitespace().collect();
        match parts.as_slice() {
            ["element", name, _] => {
                elements += 1;
                in_vertex = *name == "vertex";
                if !in_vertex && elements == 1 {
                    return Err("The PLY `vertex` element must come first to be measured".into());
                }
            }
            ["property", "list", ..] if in_vertex => {
                return Err("A PLY vertex with list properties cannot be measured".into());
            }
            ["property", kind, name] if in_vertex => {
                let size = ply_type_size(kind)
                    .ok_or_else(|| format!("Unknown PLY property type `{kind}`"))?;
                if let Some(axis) = ["x", "y", "z"].iter().position(|axis| axis == name) {
                    if size != 4 || !matches!(*kind, "float" | "float32") {
                        return Err(format!("PLY `{name}` must be a 32-bit float"));
                    }
                    xyz[axis] = Some(offset);
                }
                offset += size;
            }
            _ => {}
        }
    }
    let [Some(x), Some(y), Some(z)] = xyz else {
        return Err("The PLY vertex has no x, y, z".into());
    };
    Ok(PlyLayout {
        count: info.count,
        stride: offset,
        xyz: [x, y, z],
    })
}

fn ply_positions(path: &Path, step: u64) -> Result<Vec<[f64; 3]>, String> {
    let file = File::open(path).map_err(|error| read_error(path, &error))?;
    let mut reader = BufReader::new(file);
    let mut lines = Vec::new();
    let mut header_bytes = 0u64;
    let mut line = String::new();
    loop {
        line.clear();
        let read = reader
            .read_line(&mut line)
            .map_err(|error| read_error(path, &error))?;
        if read == 0 {
            return Err("The PLY header has no `end_header`".into());
        }
        header_bytes += read as u64;
        let trimmed = line.trim_end().to_string();
        if trimmed == "end_header" {
            break;
        }
        lines.push(trimmed);
    }
    let layout = ply_layout(&lines)?;
    let mut file = reader.into_inner();
    let mut vertex = vec![0u8; layout.stride];
    let mut positions = Vec::new();
    let mut index = 0;
    while index < layout.count {
        file.seek(SeekFrom::Start(header_bytes + index * layout.stride as u64))
            .and_then(|_| file.read_exact(&mut vertex))
            .map_err(|error| read_error(path, &error))?;
        positions.push([
            f32_at(&vertex, layout.xyz[0]),
            f32_at(&vertex, layout.xyz[1]),
            f32_at(&vertex, layout.xyz[2]),
        ]);
        index += step;
    }
    Ok(positions)
}

fn spz_positions(path: &Path, step: u64) -> Result<Vec<[f64; 3]>, String> {
    let file = File::open(path).map_err(|error| read_error(path, &error))?;
    let mut decoder = GzDecoder::new(file);
    let mut header = [0u8; 16];
    decoder
        .read_exact(&mut header)
        .map_err(|error| format!("Not a valid SPZ (gzip) file: {error}"))?;
    let info = parse_spz_header(&header)?;
    let scale = f64::from(1u32 << u32::from(header[13]).min(24));
    let mut positions = Vec::new();
    let mut record = [0u8; 9];
    let mut index = 0u64;
    while index < info.count {
        decoder
            .read_exact(&mut record)
            .map_err(|error| format!("The SPZ positions are truncated: {error}"))?;
        let fixed = |at: usize| {
            let value = i32::from(record[at])
                | (i32::from(record[at + 1]) << 8)
                | (i32::from(record[at + 2]) << 16);
            // Sign-extend the 24-bit value.
            f64::from((value << 8) >> 8) / scale
        };
        positions.push([fixed(0), fixed(3), fixed(6)]);
        index += step;
        // The next sample is `step` records on: skip the ones in between (decoded, not kept).
        let skip = (step - 1) * 9;
        if skip > 0 && index < info.count {
            std::io::copy(&mut (&mut decoder).take(skip), &mut std::io::sink())
                .map_err(|error| format!("The SPZ positions are truncated: {error}"))?;
        }
    }
    Ok(positions)
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

    #[test]
    fn the_extent_ignores_stray_floaters_and_names_nothing_for_nan() {
        // 200 splats between 10 and 12 on every axis, and two floaters a thousand units away.
        let mut points: Vec<[f64; 3]> = (0..200)
            .map(|i| {
                let t = 10.0 + 2.0 * f64::from(i) / 199.0;
                [t, t, t]
            })
            .collect();
        points.push([1000.0, -1000.0, 1000.0]);
        points.push([-1000.0, 1000.0, -1000.0]);
        points.push([f64::NAN, 0.0, 0.0]);
        let extent = Extent::from_positions(SplatFormat::Splat, 203, points).unwrap();
        assert_eq!(extent.measured, 202, "the NaN position is not measured");
        for axis in 0..3 {
            assert!(
                (10.0..=12.0).contains(&extent.min[axis])
                    && (10.0..=12.0).contains(&extent.max[axis]),
                "axis {axis}: {extent:?}"
            );
        }
        assert!(extent.radius() < 4.0, "{}", extent.radius());
        assert!(
            Extent::from_positions(SplatFormat::Splat, 1, [[f64::NAN; 3]]).is_none(),
            "nothing finite, nothing measured"
        );
    }

    #[test]
    fn a_ply_layout_finds_xyz_among_other_properties_and_refuses_what_it_cannot_read() {
        let header = |extra: &str, x: &str| {
            lines(&format!(
                "ply\nformat binary_little_endian 1.0\nelement vertex 5\n{extra}property {x} x\nproperty float y\nproperty float z\n\
                 property float f_dc_0\nproperty float f_dc_1\nproperty float f_dc_2\nproperty float opacity\n\
                 property float scale_0\nproperty float scale_1\nproperty float scale_2\n\
                 property float rot_0\nproperty float rot_1\nproperty float rot_2\nproperty float rot_3"
            ))
        };
        let plain = ply_layout(&header("", "float")).unwrap();
        assert_eq!(
            (plain.count, plain.stride, plain.xyz),
            (5, 14 * 4, [0, 4, 8])
        );
        // A leading byte pushes x, y, z by one.
        let shifted = ply_layout(&header("property uchar flag\n", "float")).unwrap();
        assert_eq!((shifted.stride, shifted.xyz), (14 * 4 + 1, [1, 5, 9]));
        assert!(
            ply_layout(&header("", "double"))
                .unwrap_err()
                .contains("32-bit float")
        );
    }
}

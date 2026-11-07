//! Reading the capture metadata of a SHARP Gaussian-splat PLY, and the camera it implies (BL143).
//!
//! Source of every rule below: Apple's `ml-sharp` (`sharp/utils/gaussians.py::save_ply`, its README
//! and `sharp/cli/predict.py`), read in place, never copied. What the file says:
//!
//! * The `vertex` element is a plain 3D Gaussian splat (`x y z f_dc_0..2 opacity scale_0..2 rot_0..3`)
//!   in the **OpenCV camera convention: x right, y down, z forward**, the scene centred around
//!   `(0, 0, +z)` and seen from the origin.
//! * Trailing one-property elements describe the source image: `intrinsic` (9 floats, a row-major
//!   `3x3` pinhole matrix `fx 0 W/2 / 0 fy H/2 / 0 0 1` in pixels of the **source image**),
//!   `image_size` (`[width, height]`, `uint`), `extrinsic` (16 floats, an identity placeholder),
//!   `frame` (`[1, particles]`), `disparity` (10 % and 90 % quantiles of `1/z`), `color_space`
//!   (`0` sRGB, `1` linearRGB) and `version` (3 bytes).
//!
//! The editor draws with Three.js: x right, **y up, the camera looks down -z**. A rotation of 180
//! degrees around X maps one to the other: `(x, y, z) -> (x, -y, -z)`. The OpenCV *up* direction is
//! `-y`; after the rotation it is `+y`, which is the fixed up of every editor camera, so no `up` is
//! stored. The camera of the source image is then the origin looking down -z with the vertical field
//! of view `2 atan(H / (2 fy))`.
//!
//! Nothing here guesses: a file that has no capture elements is a plain splat (no `Capture`), and a
//! file that has some but not a consistent set is refused with `incomplete_capture` or
//! `ambiguous_capture` and its details. The caller may import it anyway with [`Choice::Ignore`], which
//! drops the metadata and keeps the file as a plain splat (axes unspecified).

use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use serde_json::{Value, json};

use crate::content::Camera;
use crate::state::error::CommandError;

/// Orientation the editor applies to a capture's splat: degrees around X, `XYZ` order.
pub const ORIENTATION: [f64; 3] = [180.0, 0.0, 0.0];
/// The source frame's up direction (`-y`, OpenCV). [`ORIENTATION`] maps it onto the editor's `+y`.
pub const SOURCE_UP: [f64; 3] = [0.0, -1.0, 0.0];
pub const AXES: &str = "opencv";

/// Largest header the probe reads (the same bound as `splat::inspect`).
const HEADER_LIMIT: usize = 16 * 1024;
/// The principal point of the file is the image centre; a pinhole camera of the editor cannot
/// shift it, so more than this many pixels of difference is refused rather than ignored.
const PRINCIPAL_TOLERANCE_PX: f64 = 1.0;
/// `fx` and `fy` of one image: square pixels. Relative tolerance.
const ASPECT_TOLERANCE: f64 = 1e-3;
const IDENTITY_TOLERANCE: f64 = 1e-6;
/// Names of the elements the SHARP writer appends after `vertex`.
const CAPTURE_ELEMENTS: [&str; 7] = [
    "extrinsic",
    "intrinsic",
    "image_size",
    "frame",
    "disparity",
    "color_space",
    "version",
];

/// What to do with the capture metadata of a SHARP file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Choice {
    /// Use it when it is complete and consistent, refuse the file when it is not.
    #[default]
    Auto,
    /// Import the file as a plain splat, whatever its metadata says (axes unspecified).
    Ignore,
}

impl Choice {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "auto" => Some(Self::Auto),
            "ignore" => Some(Self::Ignore),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorSpace {
    Srgb,
    LinearRgb,
}

impl ColorSpace {
    pub fn label(self) -> &'static str {
        match self {
            Self::Srgb => "sRGB",
            Self::LinearRgb => "linearRGB",
        }
    }

    fn parse(label: &str) -> Option<Self> {
        match label {
            "sRGB" => Some(Self::Srgb),
            "linearRGB" => Some(Self::LinearRgb),
            _ => None,
        }
    }
}

/// The camera that took the source image of a SHARP splat, as the file declares it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Capture {
    /// Source image `[width, height]` in pixels.
    pub image: [u32; 2],
    /// Focal lengths in pixels of the source image.
    pub fx: f64,
    pub fy: f64,
    /// Principal point in pixels (the image centre for SHARP files).
    pub cx: f64,
    pub cy: f64,
    /// Vertical field of view in degrees: `2 atan(H / (2 fy))`.
    pub vfov: f64,
    pub color_space: ColorSpace,
    /// Writer version `[major, minor, patch]`; `None` when the file has no `version` element.
    pub version: Option<[u8; 3]>,
}

impl Capture {
    /// Width over height of the source image.
    pub fn aspect(&self) -> f64 {
        f64::from(self.image[0]) / f64::from(self.image[1])
    }

    /// The camera of the source image in the editor's axes (after [`ORIENTATION`]): at the origin,
    /// looking down -z, with the vertical field of view of the intrinsics. A still camera.
    pub fn camera(&self) -> Camera {
        Camera {
            position: [0.0, 0.0, 0.0],
            target: [0.0, 0.0, -1.0],
            fov: self.vfov,
            orbit: 0.0,
        }
    }

    /// Where a point of the file lands in the source image, in pixels (`x` right, `y` down), by the
    /// pinhole model of the primary source: `u = fx x / z + cx`, `v = fy y / z + cy`.
    pub fn project(&self, point: [f64; 3]) -> Option<[f64; 2]> {
        (point[2] > 0.0).then(|| {
            [
                self.fx * point[0] / point[2] + self.cx,
                self.fy * point[1] / point[2] + self.cy,
            ]
        })
    }

    /// Where the same point lands in a frame of `width x height` pixels seen through
    /// [`Self::camera`] after [`ORIENTATION`]: the Three.js perspective model with a vertical field
    /// of view (`ndc_y = y / (-z tan(fov/2))`, `ndc_x = x / (-z tan(fov/2) aspect)`).
    pub fn project_in_editor(&self, point: [f64; 3], size: [u32; 2]) -> Option<[f64; 2]> {
        let (x, y, z) = (point[0], -point[1], -point[2]);
        if z >= 0.0 {
            return None;
        }
        let tan = (self.vfov.to_radians() / 2.0).tan();
        let aspect = f64::from(size[0]) / f64::from(size[1]);
        let ndc = [x / (-z * tan * aspect), y / (-z * tan)];
        Some([
            (ndc[0] + 1.0) / 2.0 * f64::from(size[0]),
            (1.0 - ndc[1]) / 2.0 * f64::from(size[1]),
        ])
    }

    /// Things worth saying that do not stop the import.
    pub fn warnings(&self) -> Vec<(&'static str, String)> {
        let mut warnings = Vec::new();
        if self.color_space == ColorSpace::LinearRgb {
            warnings.push((
                "capture_linear_colors",
                "The file stores linearRGB colors; the editor draws them as stored (BL139)".into(),
            ));
        }
        match self.version {
            Some([1, ..]) => {}
            Some(version) => warnings.push((
                "capture_version",
                format!(
                    "The writer version is {}.{}.{}; this reading was checked against 1.x",
                    version[0], version[1], version[2]
                ),
            )),
            None => warnings.push((
                "capture_version",
                "The file has no `version` element".into(),
            )),
        }
        warnings
    }

    /// One line for the library: `SHARP 1536x1152 · FOV 49.6°`.
    pub fn label(&self) -> String {
        format!(
            "SHARP {}×{} · FOV {:.1}°",
            self.image[0], self.image[1], self.vfov
        )
    }

    pub fn to_json(&self) -> Value {
        json!({
            "source": "sharp", "axes": AXES, "up": SOURCE_UP, "orientation": ORIENTATION,
            "image": self.image, "fx": self.fx, "fy": self.fy, "cx": self.cx, "cy": self.cy,
            "vfov": self.vfov, "color_space": self.color_space.label(),
            "version": self.version,
            "camera": self.camera().to_json(),
        })
    }

    /// The part of [`Self::to_json`] that is stored in the project document.
    pub fn to_document(&self) -> Value {
        json!({
            "image": self.image, "fx": self.fx, "fy": self.fy, "cx": self.cx, "cy": self.cy,
            "color_space": self.color_space.label(), "version": self.version,
        })
    }

    /// Reads a stored capture, recomputing the derived field of view so a hand edit cannot
    /// disagree with the intrinsics.
    pub fn from_document(value: &Value) -> Result<Self, String> {
        let number = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_f64)
                .filter(|number| number.is_finite())
                .ok_or_else(|| format!("capture `{key}` is a finite number"))
        };
        let image = value
            .get("image")
            .and_then(Value::as_array)
            .filter(|items| items.len() == 2)
            .and_then(|items| {
                Some([
                    u32::try_from(items[0].as_u64()?).ok()?,
                    u32::try_from(items[1].as_u64()?).ok()?,
                ])
            })
            .ok_or("capture `image` is [width, height]")?;
        let color_space = value
            .get("color_space")
            .and_then(Value::as_str)
            .and_then(ColorSpace::parse)
            .ok_or("capture `color_space` is `sRGB` or `linearRGB`")?;
        let version = match value.get("version") {
            None | Some(Value::Null) => None,
            Some(raw) => {
                let items = raw
                    .as_array()
                    .filter(|items| items.len() == 3)
                    .and_then(|items| {
                        Some([
                            u8::try_from(items[0].as_u64()?).ok()?,
                            u8::try_from(items[1].as_u64()?).ok()?,
                            u8::try_from(items[2].as_u64()?).ok()?,
                        ])
                    })
                    .ok_or("capture `version` is [major, minor, patch]")?;
                Some(items)
            }
        };
        let capture = build(
            image,
            number("fx")?,
            number("fy")?,
            number("cx")?,
            number("cy")?,
            color_space,
            version,
        )
        .map_err(|error| error.message)?;
        Ok(capture)
    }
}

fn ambiguous(message: impl Into<String>, details: Value) -> CommandError {
    CommandError::new("ambiguous_capture", message).details(details)
}

/// Validates the intrinsics and derives the field of view.
fn build(
    image: [u32; 2],
    fx: f64,
    fy: f64,
    cx: f64,
    cy: f64,
    color_space: ColorSpace,
    version: Option<[u8; 3]>,
) -> Result<Capture, CommandError> {
    let [width, height] = image;
    if width == 0 || height == 0 || width > 65_535 || height > 65_535 {
        return Err(ambiguous(
            format!("The source image size {width}x{height} is not usable (1..65535 per side)"),
            json!({ "image_size": image }),
        ));
    }
    if !(fx.is_finite() && fy.is_finite() && fx > 0.0 && fy > 0.0) {
        return Err(ambiguous(
            format!("The focal lengths fx={fx}, fy={fy} must be finite and positive"),
            json!({ "fx": fx, "fy": fy }),
        ));
    }
    if (fx - fy).abs() > fx.max(fy) * ASPECT_TOLERANCE {
        return Err(ambiguous(
            format!(
                "fx={fx} and fy={fy} differ: non-square pixels do not map to one field of view"
            ),
            json!({ "fx": fx, "fy": fy, "tolerance": ASPECT_TOLERANCE }),
        ));
    }
    let centre = [f64::from(width) / 2.0, f64::from(height) / 2.0];
    if (cx - centre[0]).abs() > PRINCIPAL_TOLERANCE_PX
        || (cy - centre[1]).abs() > PRINCIPAL_TOLERANCE_PX
    {
        return Err(ambiguous(
            format!(
                "The principal point ({cx}, {cy}) is not the image centre ({}, {}): the editor's \
                 camera is centred",
                centre[0], centre[1]
            ),
            json!({ "principal_point": [cx, cy], "image_centre": centre,
                "tolerance_px": PRINCIPAL_TOLERANCE_PX }),
        ));
    }
    let vfov = (2.0 * (f64::from(height) / (2.0 * fy)).atan()).to_degrees();
    if !(1.0..170.0).contains(&vfov) {
        return Err(ambiguous(
            format!("The vertical field of view {vfov:.2} degrees is outside 1..170"),
            json!({ "vfov": vfov, "min": 1, "max": 170 }),
        ));
    }
    Ok(Capture {
        image,
        fx,
        fy,
        cx,
        cy,
        vfov,
        color_space,
        version,
    })
}

#[derive(Debug, Clone)]
struct Property {
    name: String,
    kind: String,
}

#[derive(Debug, Clone)]
struct Element {
    name: String,
    count: u64,
    properties: Vec<Property>,
    /// Byte offset of the element's data from the start of the file.
    offset: u64,
}

impl Element {
    fn record_size(&self) -> Result<u64, String> {
        self.properties
            .iter()
            .map(|property| {
                type_size(&property.kind).ok_or_else(|| {
                    format!(
                        "Property `{}` of element `{}` has the unsupported type `{}`",
                        property.name, self.name, property.kind
                    )
                })
            })
            .sum()
    }
}

fn type_size(kind: &str) -> Option<u64> {
    match kind {
        "char" | "uchar" | "int8" | "uint8" => Some(1),
        "short" | "ushort" | "int16" | "uint16" => Some(2),
        "int" | "uint" | "float" | "int32" | "uint32" | "float32" => Some(4),
        "double" | "float64" => Some(8),
        _ => None,
    }
}

fn decode(kind: &str, bytes: &[u8]) -> f64 {
    match kind {
        "char" | "int8" => f64::from(bytes[0] as i8),
        "uchar" | "uint8" => f64::from(bytes[0]),
        "short" | "int16" => f64::from(i16::from_le_bytes([bytes[0], bytes[1]])),
        "ushort" | "uint16" => f64::from(u16::from_le_bytes([bytes[0], bytes[1]])),
        "int" | "int32" => f64::from(i32::from_le_bytes(bytes[..4].try_into().unwrap())),
        "uint" | "uint32" => f64::from(u32::from_le_bytes(bytes[..4].try_into().unwrap())),
        "float" | "float32" => f64::from(f32::from_le_bytes(bytes[..4].try_into().unwrap())),
        _ => f64::from_le_bytes(bytes[..8].try_into().unwrap()),
    }
}

fn io_error(path: &Path, error: &std::io::Error) -> CommandError {
    CommandError::new(
        "asset_unreadable",
        format!("Cannot read `{}`: {error}", path.display()),
    )
    .target(path.to_string_lossy())
}

fn invalid(path: &Path, message: impl Into<String>) -> CommandError {
    CommandError::new("invalid_asset", message).target(path.to_string_lossy())
}

/// Parses the header into elements with their byte offsets.
fn layout(path: &Path, reader: &mut impl BufRead) -> Result<Vec<Element>, CommandError> {
    let mut consumed = 0usize;
    let mut elements: Vec<Element> = Vec::new();
    let mut line = String::new();
    let mut first = true;
    let mut binary = false;
    loop {
        line.clear();
        let read = reader
            .read_line(&mut line)
            .map_err(|error| io_error(path, &error))?;
        if read == 0 {
            return Err(invalid(path, "The PLY header has no `end_header`"));
        }
        consumed += read;
        if consumed > HEADER_LIMIT {
            return Err(invalid(path, "The PLY header is larger than 16 KiB"));
        }
        let text = line.trim_end();
        if first {
            if text != "ply" {
                return Err(invalid(path, "Not a PLY file"));
            }
            first = false;
            continue;
        }
        let parts: Vec<&str> = text.split_whitespace().collect();
        match parts.as_slice() {
            ["format", name, ..] => binary = *name == "binary_little_endian",
            ["element", name, count] => elements.push(Element {
                name: (*name).to_string(),
                count: count
                    .parse()
                    .map_err(|_| invalid(path, format!("Invalid count `{count}` of `{name}`")))?,
                properties: Vec::new(),
                offset: 0,
            }),
            ["property", "list", ..] => {
                return Err(invalid(
                    path,
                    "A PLY with list properties is not a Gaussian splat",
                ));
            }
            ["property", kind, name] => {
                let element = elements
                    .last_mut()
                    .ok_or_else(|| invalid(path, "A `property` before any `element`"))?;
                element.properties.push(Property {
                    name: (*name).to_string(),
                    kind: (*kind).to_string(),
                });
            }
            ["end_header"] => break,
            _ => {}
        }
    }
    if !binary {
        return Err(invalid(
            path,
            "Only `binary_little_endian` Gaussian PLY files are supported",
        ));
    }
    let mut offset = consumed as u64;
    for element in &mut elements {
        element.offset = offset;
        let size = element
            .record_size()
            .map_err(|message| invalid(path, message))?;
        offset += size * element.count;
    }
    Ok(elements)
}

/// Reads the capture metadata of `path` (an already identified Gaussian PLY).
///
/// * `Ok(None)`: the file has no capture element: a plain splat, axes unspecified.
/// * `Ok(Some(_))`: a complete, consistent SHARP capture.
/// * `Err(incomplete_capture | ambiguous_capture)`: capture elements without a usable meaning;
///   nothing is guessed. [`Choice::Ignore`] turns every one of them into `Ok(None)`.
pub fn read(path: &Path, choice: Choice) -> Result<Option<Capture>, CommandError> {
    if choice == Choice::Ignore {
        return Ok(None);
    }
    read_capture(path).map_err(|mut error| {
        if error.code == "ambiguous_capture" {
            error.message.push_str(
                ". Import it with `capture: ignore` to keep it as a plain splat (axes unspecified)",
            );
            error
                .target
                .get_or_insert_with(|| path.to_string_lossy().into_owned());
        }
        error
    })
}

fn read_capture(path: &Path) -> Result<Option<Capture>, CommandError> {
    let file = File::open(path).map_err(|error| io_error(path, &error))?;
    let length = file
        .metadata()
        .map_err(|error| io_error(path, &error))?
        .len();
    let mut reader = BufReader::new(file);
    let elements = layout(path, &mut reader)?;
    let present: Vec<&str> = elements
        .iter()
        .map(|element| element.name.as_str())
        .filter(|name| CAPTURE_ELEMENTS.contains(name))
        .collect();
    if present.is_empty() {
        return Ok(None);
    }
    let find = |name: &str| elements.iter().find(|element| element.name == name);
    let missing: Vec<&str> = ["intrinsic", "image_size"]
        .into_iter()
        .filter(|name| find(name).is_none())
        .collect();
    if !missing.is_empty() {
        return Err(CommandError::new(
            "incomplete_capture",
            format!(
                "The PLY has capture elements ({}) but not {}: the camera cannot be recovered. \
                 Import it with `capture: ignore` to keep it as a plain splat",
                present.join(", "),
                missing.join(", ")
            ),
        )
        .target(path.to_string_lossy())
        .details(json!({ "present": present, "missing": missing })));
    }
    let end = elements
        .last()
        .map(|element| element.offset + element.record_size().unwrap_or(0) * element.count)
        .unwrap_or(0);
    if length < end {
        return Err(invalid(
            path,
            format!("The PLY is truncated: the header promises {end} bytes, the file has {length}"),
        ));
    }
    let mut values = |name: &str,
                      count: u64,
                      allowed: &[&str]|
     -> Result<Option<Vec<f64>>, CommandError> {
        let Some(element) = find(name) else {
            return Ok(None);
        };
        let shaped = element.count == count
            && element.properties.len() == 1
            && allowed.contains(&element.properties[0].kind.as_str());
        if !shaped {
            return Err(ambiguous(
                format!(
                    "Element `{name}` has {} records of ({}); the SHARP writer emits {count} of one \
                     {} property",
                    element.count,
                    element
                        .properties
                        .iter()
                        .map(|property| format!("{} {}", property.kind, property.name))
                        .collect::<Vec<_>>()
                        .join(", "),
                    allowed.join("/")
                ),
                json!({ "element": name, "count": element.count, "expected_count": count }),
            ));
        }
        let kind = &element.properties[0].kind;
        let size = type_size(kind).unwrap_or(4) as usize;
        reader
            .seek(SeekFrom::Start(element.offset))
            .map_err(|error| io_error(path, &error))?;
        let mut bytes = vec![0u8; size * count as usize];
        reader
            .read_exact(&mut bytes)
            .map_err(|error| io_error(path, &error))?;
        Ok(Some(
            bytes
                .chunks_exact(size)
                .map(|chunk| decode(kind, chunk))
                .collect(),
        ))
    };
    let floats = ["float", "float32"];
    let intrinsic = values("intrinsic", 9, &floats)?.expect("checked above");
    let size =
        values("image_size", 2, &["uint", "uint32", "int", "int32"])?.expect("checked above");
    if let Some(extrinsic) = values("extrinsic", 16, &floats)? {
        let identity: Vec<f64> = (0..16)
            .map(|index| if index % 5 == 0 { 1.0 } else { 0.0 })
            .collect();
        if extrinsic
            .iter()
            .zip(&identity)
            .any(|(value, expected)| (value - expected).abs() > IDENTITY_TOLERANCE)
        {
            return Err(ambiguous(
                "The `extrinsic` matrix is not the identity placeholder the SHARP writer emits: \
                 whether the splat is in camera or world coordinates is not stated",
                json!({ "extrinsic": extrinsic }),
            ));
        }
    }
    let vertices = find("vertex").map_or(0, |element| element.count);
    if let Some(frame) = values("frame", 2, &["int", "int32", "uint", "uint32"])?
        && (frame[0] != 1.0 || frame[1] != vertices as f64)
    {
        return Err(ambiguous(
            format!(
                "`frame` says {} frame(s) of {} particles; one frame of {vertices} is supported",
                frame[0], frame[1]
            ),
            json!({ "frame": frame, "vertices": vertices }),
        ));
    }
    if let Some(disparity) = values("disparity", 2, &floats)?
        && !disparity.iter().all(|value| value.is_finite())
    {
        return Err(ambiguous(
            "`disparity` holds a value that is not finite",
            json!({ "disparity": disparity }),
        ));
    }
    let color_space = match values("color_space", 1, &["uchar", "uint8"])? {
        None => ColorSpace::Srgb,
        Some(code) => match code[0] as u8 {
            0 => ColorSpace::Srgb,
            1 => ColorSpace::LinearRgb,
            other => {
                return Err(ambiguous(
                    format!("`color_space` is {other}; 0 is sRGB and 1 is linearRGB"),
                    json!({ "color_space": other }),
                ));
            }
        },
    };
    let version = values("version", 3, &["uchar", "uint8"])?
        .map(|bytes| [bytes[0] as u8, bytes[1] as u8, bytes[2] as u8]);
    // The matrix is row-major `fx 0 cx / 0 fy cy / 0 0 1`: anything else is skew or a different model.
    let off_axis = [1usize, 3, 6, 7];
    if off_axis.iter().any(|index| intrinsic[*index] != 0.0) || intrinsic[8] != 1.0 {
        return Err(ambiguous(
            "The `intrinsic` matrix is not `fx 0 cx / 0 fy cy / 0 0 1`: skew is not supported",
            json!({ "intrinsic": intrinsic }),
        ));
    }
    let image = [size[0] as u32, size[1] as u32];
    if size
        .iter()
        .any(|value| *value < 0.0 || value.fract() != 0.0)
    {
        return Err(ambiguous(
            "The image size is not made of whole non-negative pixels",
            json!({ "image_size": size }),
        ));
    }
    build(
        image,
        intrinsic[0],
        intrinsic[4],
        intrinsic[2],
        intrinsic[5],
        color_space,
        version,
    )
    .map(Some)
    .map_err(|error| error.target(path.to_string_lossy()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{SharpSpec, write_sharp_ply};
    use core::prelude::v1::test;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("tessel-sharp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_sharp_file_yields_the_camera_of_its_source_image() {
        let dir = temp("camera");
        let path = dir.join("a.ply");
        write_sharp_ply(&path, &SharpSpec::default()).unwrap();
        let capture = read(&path, Choice::Auto).unwrap().unwrap();
        assert_eq!(capture.image, [160, 120]);
        // fy = 120 / (2 tan(25 deg)) puts the vertical field of view at 50 degrees.
        assert!((capture.vfov - 50.0).abs() < 1e-3, "{}", capture.vfov);
        assert_eq!(capture.color_space, ColorSpace::Srgb);
        assert_eq!(capture.version, Some([1, 5, 0]));
        let camera = capture.camera();
        assert_eq!(camera.position, [0.0, 0.0, 0.0]);
        assert_eq!(camera.target, [0.0, 0.0, -1.0]);
        assert!((camera.fov - capture.vfov).abs() < 1e-12);
        assert!(capture.warnings().is_empty());
        assert_eq!(capture.label(), "SHARP 160×120 · FOV 50.0°");
    }

    #[test]
    fn the_editor_camera_and_the_pinhole_of_the_file_agree_on_every_pixel() {
        let dir = temp("pinhole");
        let path = dir.join("a.ply");
        write_sharp_ply(&path, &SharpSpec::default()).unwrap();
        let capture = read(&path, Choice::Auto).unwrap().unwrap();
        for point in [
            [0.0, 0.0, 3.0],
            [0.5, -0.25, 2.0],
            [-1.2, 0.8, 5.5],
            [0.1, 0.1, 0.7],
            [2.0, -1.5, 9.0],
        ] {
            let source = capture.project(point).unwrap();
            let editor = capture.project_in_editor(point, capture.image).unwrap();
            assert!(
                (source[0] - editor[0]).abs() < 1e-9 && (source[1] - editor[1]).abs() < 1e-9,
                "{point:?}: file {source:?} vs editor {editor:?}"
            );
        }
        // Behind the camera is nowhere, in both models.
        assert!(capture.project([0.0, 0.0, -1.0]).is_none());
        assert!(
            capture
                .project_in_editor([0.0, 0.0, -1.0], capture.image)
                .is_none()
        );
        // Without the 180 degree turn (y kept down, z kept forward) the picture would be upside
        // down: a point below the axis in the file is below it in the image.
        let below = capture.project([0.0, 1.0, 4.0]).unwrap();
        assert!(below[1] > capture.cy);
    }

    #[test]
    fn a_plain_gaussian_ply_has_no_capture_and_ignore_drops_the_metadata() {
        let dir = temp("plain");
        let plain = dir.join("plain.ply");
        write_sharp_ply(
            &plain,
            &SharpSpec {
                metadata: false,
                ..SharpSpec::default()
            },
        )
        .unwrap();
        assert_eq!(read(&plain, Choice::Auto).unwrap(), None);
        let sharp = dir.join("sharp.ply");
        write_sharp_ply(&sharp, &SharpSpec::default()).unwrap();
        assert_eq!(read(&sharp, Choice::Ignore).unwrap(), None);
    }

    #[test]
    fn missing_or_contradictory_metadata_is_refused_with_its_reason() {
        let dir = temp("refused");
        let check = |name: &str, spec: SharpSpec, code: &str, needle: &str| {
            let path = dir.join(format!("{name}.ply"));
            write_sharp_ply(&path, &spec).unwrap();
            let error = read(&path, Choice::Auto).unwrap_err();
            assert_eq!(error.code, code, "{name}: {error:?}");
            assert!(error.message.contains(needle), "{name}: {}", error.message);
            assert!(error.details.is_some(), "{name}: details");
            // The explicit choice imports it as a plain splat.
            assert_eq!(read(&path, Choice::Ignore).unwrap(), None);
        };
        check(
            "no-intrinsic",
            SharpSpec {
                intrinsic: None,
                ..SharpSpec::default()
            },
            "incomplete_capture",
            "intrinsic",
        );
        check(
            "no-size",
            SharpSpec {
                image_size: None,
                ..SharpSpec::default()
            },
            "incomplete_capture",
            "image_size",
        );
        check(
            "skew",
            SharpSpec {
                intrinsic: Some([100.0, 0.5, 80.0, 0.0, 100.0, 60.0, 0.0, 0.0, 1.0]),
                ..SharpSpec::default()
            },
            "ambiguous_capture",
            "skew",
        );
        check(
            "rectangular-pixels",
            SharpSpec {
                intrinsic: Some([100.0, 0.0, 80.0, 0.0, 130.0, 60.0, 0.0, 0.0, 1.0]),
                ..SharpSpec::default()
            },
            "ambiguous_capture",
            "differ",
        );
        check(
            "off-centre",
            SharpSpec {
                intrinsic: Some([100.0, 0.0, 120.0, 0.0, 100.0, 60.0, 0.0, 0.0, 1.0]),
                ..SharpSpec::default()
            },
            "ambiguous_capture",
            "principal point",
        );
        check(
            "zero-focal",
            SharpSpec {
                intrinsic: Some([0.0, 0.0, 80.0, 0.0, 0.0, 60.0, 0.0, 0.0, 1.0]),
                ..SharpSpec::default()
            },
            "ambiguous_capture",
            "focal",
        );
        check(
            "empty-image",
            SharpSpec {
                image_size: Some([0, 120]),
                ..SharpSpec::default()
            },
            "ambiguous_capture",
            "0x120",
        );
        let mut moved = [0.0f32; 16];
        for index in [0, 5, 10, 15] {
            moved[index] = 1.0;
        }
        moved[3] = 2.0;
        check(
            "moved-extrinsic",
            SharpSpec {
                extrinsic: Some(moved),
                ..SharpSpec::default()
            },
            "ambiguous_capture",
            "extrinsic",
        );
        check(
            "two-frames",
            SharpSpec {
                frames: Some([2, 5]),
                ..SharpSpec::default()
            },
            "ambiguous_capture",
            "frame",
        );
        check(
            "colorspace",
            SharpSpec {
                color_space: Some(7),
                ..SharpSpec::default()
            },
            "ambiguous_capture",
            "color_space",
        );
    }

    #[test]
    fn linear_colors_and_an_unknown_writer_are_warnings_not_refusals() {
        let dir = temp("warn");
        let path = dir.join("a.ply");
        write_sharp_ply(
            &path,
            &SharpSpec {
                color_space: Some(1),
                version: Some([2, 0, 1]),
                ..SharpSpec::default()
            },
        )
        .unwrap();
        let capture = read(&path, Choice::Auto).unwrap().unwrap();
        let codes: Vec<&str> = capture.warnings().iter().map(|(code, _)| *code).collect();
        assert_eq!(codes, ["capture_linear_colors", "capture_version"]);
    }

    #[test]
    fn a_truncated_file_is_an_invalid_asset_and_the_document_form_round_trips() {
        let dir = temp("truncated");
        let path = dir.join("a.ply");
        write_sharp_ply(&path, &SharpSpec::default()).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        std::fs::write(&path, &bytes[..bytes.len() - 5]).unwrap();
        assert_eq!(read(&path, Choice::Auto).unwrap_err().code, "invalid_asset");
        std::fs::write(&path, &bytes).unwrap();
        let capture = read(&path, Choice::Auto).unwrap().unwrap();
        let stored = capture.to_document();
        assert_eq!(Capture::from_document(&stored).unwrap(), capture);
        // A hand-edited document that contradicts itself is refused, not trusted.
        let mut bad = stored.clone();
        bad["fx"] = json!(-4.0);
        assert!(Capture::from_document(&bad).is_err());
    }
}

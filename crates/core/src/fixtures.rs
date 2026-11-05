//! Small procedural fixtures for tests, examples and the viewport proof.
//!
//! Everything here is generated from arithmetic, so there is no third-party asset and no
//! licence to track. All output is deterministic: same arguments, same bytes.
//! `./scripts/fixtures <dir>` writes the standard set (see `docs/features/viewport.md`).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::audio::{Pcm, encode_wav_pcm16};

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for byte in bytes {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(&body).to_be_bytes());
}

/// Encodes an RGBA8 image as PNG using stored (uncompressed) deflate blocks. Fixtures are
/// tiny, so skipping compression keeps the encoder dependency-free and byte-stable.
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    assert_eq!(
        rgba.len(),
        (width * height * 4) as usize,
        "RGBA buffer size"
    );
    let mut raw = Vec::with_capacity(rgba.len() + height as usize);
    for row in rgba.chunks_exact((width * 4) as usize) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut zlib = vec![0x78, 0x01];
    let mut blocks = raw.chunks(65_535).peekable();
    while let Some(block) = blocks.next() {
        zlib.push(u8::from(blocks.peek().is_none()));
        zlib.extend_from_slice(&(block.len() as u16).to_le_bytes());
        zlib.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &header);
    chunk(&mut out, b"IDAT", &zlib);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Colors of [`quadrants_png`], top-left, top-right, bottom-left, bottom-right.
pub const QUADRANT_COLORS: [[u8; 3]; 4] =
    [[220, 40, 40], [40, 200, 60], [40, 80, 220], [230, 210, 40]];

/// Four flat quadrants, so a probe in the middle of each one has an exact expected color.
pub fn quadrants_png(width: u32, height: u32) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let index = usize::from(y >= height / 2) * 2 + usize::from(x >= width / 2);
            let [r, g, b] = QUADRANT_COLORS[index];
            rgba.extend_from_slice(&[r, g, b, 255]);
        }
    }
    encode_png(width, height, &rgba)
}

/// Flat color of frame `index` in [`video_frame_png`]; distinct for the first 8 frames.
pub fn video_frame_color(index: i64) -> [u8; 3] {
    let step = index.rem_euclid(8) as u8;
    [30 + step * 25, 200 - step * 20, 60 + step * 10]
}

/// One frame of the procedural frame-sequence video: flat color encoding the frame number.
pub fn video_frame_png(width: u32, height: u32, index: i64) -> Vec<u8> {
    let [r, g, b] = video_frame_color(index);
    let rgba: Vec<u8> = (0..width * height).flat_map(|_| [r, g, b, 255]).collect();
    encode_png(width, height, &rgba)
}

/// A sine tone as PCM16 WAV.
pub fn tone_wav(
    sample_rate: u32,
    seconds: f64,
    frequency: f64,
    amplitude: f32,
    channels: u16,
) -> Vec<u8> {
    let frames = (f64::from(sample_rate) * seconds).round() as usize;
    let mut samples = Vec::with_capacity(frames * usize::from(channels));
    for frame in 0..frames {
        let value = (std::f64::consts::TAU * frequency * frame as f64 / f64::from(sample_rate))
            .sin() as f32;
        for _ in 0..channels {
            samples.push(value * amplitude);
        }
    }
    encode_wav_pcm16(&Pcm {
        sample_rate,
        channels,
        samples,
    })
}

/// Deterministic xorshift, so fixtures never depend on a random source.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// A Gaussian-splat cloud in the 32-byte `.splat` layout (position f32×3, scale f32×3,
/// color u8×4, rotation u8×4 as `w,x,y,z`): a shell of `count` splats on a unit sphere
/// whose color follows the position, plus a ring, so a camera orbit visibly changes it.
pub fn splat_cloud(count: usize) -> Vec<u8> {
    let mut rng = Lcg(0x9E37_79B9_7F4A_7C15);
    let mut out = Vec::with_capacity(count * 32);
    for index in 0..count {
        let (position, scale): ([f32; 3], f32) = if index % 5 == 0 {
            let angle = rng.next() * std::f32::consts::TAU;
            (
                [
                    1.6 * angle.cos(),
                    -0.2 + (rng.next() - 0.5) * 0.08,
                    1.6 * angle.sin(),
                ],
                0.035,
            )
        } else {
            let z = rng.next() * 2.0 - 1.0;
            let angle = rng.next() * std::f32::consts::TAU;
            let radius = (1.0 - z * z).sqrt();
            ([radius * angle.cos(), z, radius * angle.sin()], 0.05)
        };
        for value in position.iter().chain(&[scale, scale, scale * 0.6]) {
            out.extend_from_slice(&value.to_le_bytes());
        }
        let channel = |value: f32| ((value * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0).round() as u8;
        out.extend_from_slice(&[
            channel(position[0]),
            channel(position[1]),
            channel(position[2]),
            235,
        ]);
        out.extend_from_slice(&[255, 128, 128, 128]);
    }
    out
}

/// One splat of [`splat_cloud`], decoded from its 32-byte record.
struct CloudPoint {
    position: [f32; 3],
    scale: [f32; 3],
    rgba: [u8; 4],
    /// Quaternion as `w, x, y, z`, unit length.
    rotation: [f32; 4],
}

fn cloud_points(count: usize) -> Vec<CloudPoint> {
    let bytes = splat_cloud(count);
    bytes
        .as_chunks::<32>()
        .0
        .iter()
        .map(|record| {
            let float = |at: usize| {
                f32::from_le_bytes([record[at], record[at + 1], record[at + 2], record[at + 3]])
            };
            let quaternion: [f32; 4] =
                std::array::from_fn(|i| (f32::from(record[28 + i]) - 128.0) / 128.0);
            let length = quaternion
                .iter()
                .map(|v| v * v)
                .sum::<f32>()
                .sqrt()
                .max(1e-6);
            CloudPoint {
                position: [float(0), float(4), float(8)],
                scale: [float(12), float(16), float(20)],
                rgba: [record[24], record[25], record[26], record[27]],
                rotation: quaternion.map(|v| v / length),
            }
        })
        .collect()
}

/// Zeroth spherical-harmonic coefficient: how 3DGS files store the base color.
const SH_C0: f32 = 0.282_094_8;

/// The same cloud as a binary little-endian Gaussian PLY (`f_dc`, logit opacity, log scale).
pub fn splat_ply(count: usize) -> Vec<u8> {
    let mut out = format!(
        "ply\nformat binary_little_endian 1.0\nelement vertex {count}\n\
         property float x\nproperty float y\nproperty float z\n\
         property float f_dc_0\nproperty float f_dc_1\nproperty float f_dc_2\nproperty float opacity\n\
         property float scale_0\nproperty float scale_1\nproperty float scale_2\n\
         property float rot_0\nproperty float rot_1\nproperty float rot_2\nproperty float rot_3\nend_header\n"
    )
    .into_bytes();
    for point in cloud_points(count) {
        let alpha = (f32::from(point.rgba[3]) / 255.0).clamp(0.001, 0.999);
        let values = [
            point.position[0],
            point.position[1],
            point.position[2],
            (f32::from(point.rgba[0]) / 255.0 - 0.5) / SH_C0,
            (f32::from(point.rgba[1]) / 255.0 - 0.5) / SH_C0,
            (f32::from(point.rgba[2]) / 255.0 - 0.5) / SH_C0,
            (alpha / (1.0 - alpha)).ln(),
            point.scale[0].ln(),
            point.scale[1].ln(),
            point.scale[2].ln(),
            point.rotation[0],
            point.rotation[1],
            point.rotation[2],
            point.rotation[3],
        ];
        for value in values {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    out
}

/// The same cloud as Niantic SPZ v2 (gzip of header + positions, alphas, colors, scales,
/// rotations), 12 fractional bits, no spherical harmonics.
pub fn splat_spz(count: usize) -> Vec<u8> {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Write;
    let points = cloud_points(count);
    let mut raw = Vec::new();
    raw.extend_from_slice(&0x5053_474eu32.to_le_bytes());
    raw.extend_from_slice(&2u32.to_le_bytes());
    raw.extend_from_slice(&(points.len() as u32).to_le_bytes());
    raw.extend_from_slice(&[0, 12, 0, 0]);
    for point in &points {
        for value in point.position {
            let fixed = (value * 4096.0).round() as i32;
            raw.extend_from_slice(&fixed.to_le_bytes()[..3]);
        }
    }
    raw.extend(points.iter().map(|point| point.rgba[3]));
    for point in &points {
        for channel in &point.rgba[..3] {
            // SPZ colors are `(sh_dc * 0.15 + 0.5) * 255`.
            let sh = (f32::from(*channel) / 255.0 - 0.5) / SH_C0;
            raw.push(((sh * 0.15 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8);
        }
    }
    for point in &points {
        for scale in point.scale {
            raw.push(((scale.ln() + 10.0) * 16.0).round().clamp(0.0, 255.0) as u8);
        }
    }
    for point in &points {
        // v2 stores x, y, z of a quaternion whose w is non-negative; w is rebuilt on load.
        let sign = if point.rotation[0] < 0.0 { -1.0 } else { 1.0 };
        for axis in &point.rotation[1..] {
            raw.push((sign * axis * 127.5 + 127.5).round().clamp(0.0, 255.0) as u8);
        }
    }
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(&raw).expect("gzip to memory cannot fail");
    encoder.finish().expect("gzip to memory cannot fail")
}

/// A Three.js scene definition (`tessel-scene` v1): a box that spins, a sphere, and the
/// given splat asset.
pub fn scene_json(splat_asset: &str) -> String {
    format!(
        r##"{{
  "format": "tessel-scene",
  "version": 1,
  "background": [0.04, 0.05, 0.1, 1.0],
  "objects": [
    {{ "type": "box", "size": [0.8, 0.8, 0.8], "position": [-2.2, 0.0, 0.0], "color": "#e8a33d", "spin": [0, 60, 0] }},
    {{ "type": "sphere", "radius": 0.5, "position": [2.2, 0.0, 0.0], "color": "#4fa3e8" }},
    {{ "type": "splat", "asset": "{splat_asset}", "position": [0.0, 0.0, 0.0], "scale": 1.0, "spin": [0, 20, 0] }}
  ]
}}
"##
    )
}

/// What [`write_standard_set`] wrote, for building commands without guessing file names.
#[derive(Debug, Clone)]
pub struct StandardSet {
    pub dir: PathBuf,
    pub image: PathBuf,
    pub frames_pattern: PathBuf,
    pub frame_count: i64,
    pub tone: PathBuf,
    pub splat: PathBuf,
    pub ply: PathBuf,
    pub spz: PathBuf,
    pub scene: PathBuf,
}

pub const STANDARD_IMAGE_SIZE: (u32, u32) = (64, 36);
pub const STANDARD_VIDEO_FRAMES: i64 = 8;

/// Writes the standard fixture set into `dir` (created if needed).
pub fn write_standard_set(dir: &Path) -> io::Result<StandardSet> {
    fs::create_dir_all(dir.join("frames"))?;
    let (width, height) = STANDARD_IMAGE_SIZE;
    let image = dir.join("quadrants.png");
    fs::write(&image, quadrants_png(width, height))?;
    for index in 0..STANDARD_VIDEO_FRAMES {
        fs::write(
            dir.join("frames").join(format!("f{index:05}.png")),
            video_frame_png(width, height, index),
        )?;
    }
    let tone = dir.join("tone-440.wav");
    fs::write(&tone, tone_wav(48_000, 2.0, 440.0, 0.5, 1))?;
    let splat = dir.join("cloud.splat");
    fs::write(&splat, splat_cloud(4000))?;
    let ply = dir.join("cloud.ply");
    fs::write(&ply, splat_ply(4000))?;
    let spz = dir.join("cloud.spz");
    fs::write(&spz, splat_spz(4000))?;
    let scene = dir.join("scene.tessel-scene.json");
    fs::write(&scene, scene_json("cloud"))?;
    Ok(StandardSet {
        dir: dir.to_path_buf(),
        image,
        frames_pattern: dir.join("frames").join("f%05d.png"),
        frame_count: STANDARD_VIDEO_FRAMES,
        tone,
        splat,
        ply,
        spz,
        scene,
    })
}

/// Canvas of the proof project.
pub const PROOF_CANVAS: (u32, u32) = (320, 180);
/// Sequence frames of the proof project (2 s at 24 fps).
pub const PROOF_LENGTH: i64 = 48;

/// The semantic requests that build the viewport proof project from [`StandardSet`] files:
/// a scene (splat + primitives, orbiting camera) at the bottom, image then frame-sequence
/// video on a media track, a title on top and a tone on an audio track.
///
/// The same list runs through the Rust API, `tessel-headless`/`tessel-render` and, line by
/// line, `./scripts/ui op` against the open window (`docs/features/viewport.md`).
pub fn proof_requests(set: &StandardSet) -> Vec<serde_json::Value> {
    use serde_json::json;
    let path = |p: &Path| p.to_string_lossy().into_owned();
    vec![
        json!({"op":"project.create","name":"Viewport proof","sequence":"main","rate":24,
            "width":PROOF_CANVAS.0,"height":PROOF_CANVAS.1,"length":PROOF_LENGTH}),
        json!({"op":"asset.add","kind":"image","id":"img","name":"Quadrants","path":path(&set.image)}),
        json!({"op":"asset.add","kind":"video","id":"vid","name":"Counter frames","pattern":path(&set.frames_pattern),
            "rate":8,"frames":set.frame_count}),
        json!({"op":"asset.add","kind":"audio","id":"tone","name":"Tone 440 Hz","path":path(&set.tone)}),
        json!({"op":"asset.add","kind":"splat","id":"cloud","name":"Splat cloud","path":path(&set.splat)}),
        json!({"op":"asset.add","kind":"scene","id":"scn","name":"Demo scene","path":path(&set.scene)}),
        json!({"op":"asset.add","kind":"text","id":"label","name":"Label",
            "text":{"content":"TESSEL","size":14,"color":"#ffffff"}}),
        json!({"op":"track.add","id":"scene-track","name":"Scene","kind":"video"}),
        json!({"op":"track.add","id":"media-track","name":"Media","kind":"video"}),
        json!({"op":"track.add","id":"title-track","name":"Titles","kind":"video"}),
        json!({"op":"track.add","id":"music","name":"Music","kind":"audio"}),
        json!({"op":"clip.add","id":"c-scene","track":"scene-track","asset":"scn","start":0,"duration":48,
            "camera":{"position":[0,1.2,5],"target":[0,0,0],"fov":50,"orbit":45}}),
        json!({"op":"clip.add","id":"c-img","track":"media-track","asset":"img","start":0,"duration":24,
            "properties":{"zoom":50}}),
        json!({"op":"clip.add","id":"c-vid","track":"media-track","asset":"vid","start":24,"duration":24,
            "properties":{"zoom":50}}),
        json!({"op":"clip.add","id":"c-title","track":"title-track","asset":"label","start":0,"duration":48,
            "properties":{"position_y":38}}),
        json!({"op":"clip.add","id":"c-tone","track":"music","asset":"tone","start":0,"duration":48}),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::{Asset, Declared, MediaKind};
    use crate::time::Rate;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("tessel-fixtures-{name}-{}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        dir
    }

    #[test]
    fn generation_is_byte_stable() {
        assert_eq!(quadrants_png(8, 4), quadrants_png(8, 4));
        assert_eq!(splat_cloud(100), splat_cloud(100));
        assert_eq!(splat_cloud(100).len(), 3200);
        assert_ne!(video_frame_png(4, 4, 0), video_frame_png(4, 4, 1));
        assert_eq!(video_frame_color(0), video_frame_color(8));
    }

    #[test]
    fn standard_set_probes_as_typed_assets() {
        let dir = temp_dir("probe");
        let set = write_standard_set(&dir).unwrap();
        let path = |p: &Path| p.to_string_lossy().into_owned();
        let image =
            Asset::probe(MediaKind::Still, &path(&set.image), &Declared::default()).unwrap();
        assert_eq!((image.meta.width, image.meta.height), (Some(64), Some(36)));
        let tone = Asset::probe(MediaKind::Audio, &path(&set.tone), &Declared::default()).unwrap();
        assert_eq!(tone.meta.sample_rate, Some(48_000));
        assert_eq!(tone.meta.duration.unwrap().to_f64(), 2.0);
        let splat =
            Asset::probe(MediaKind::Splat, &path(&set.splat), &Declared::default()).unwrap();
        assert_eq!(splat.meta.splat_count, Some(4000));
        let scene =
            Asset::probe(MediaKind::Scene, &path(&set.scene), &Declared::default()).unwrap();
        assert_eq!(scene.meta.requires.len(), 1);
        let video = Asset::probe_frame_sequence(
            &path(&set.frames_pattern),
            &Declared {
                rate: Rate::new(8, 1),
                frames: Some(set.frame_count),
                ..Declared::default()
            },
        )
        .unwrap();
        assert_eq!(video.meta.duration.unwrap().to_f64(), 1.0);
        // A PNG that is really a WAV is rejected with a typed error and no registration.
        let error =
            Asset::probe(MediaKind::Still, &path(&set.tone), &Declared::default()).unwrap_err();
        assert_eq!(error.code, "invalid_asset");
        fs::remove_dir_all(dir).ok();
    }
}

#[cfg(test)]
mod splat_format_tests {
    use super::*;
    use crate::splat::{SplatFormat, inspect};

    #[test]
    fn every_fixture_flavour_of_the_cloud_is_identified_by_content() {
        let dir = std::env::temp_dir().join(format!("tessel-splats-{}", std::process::id()));
        let set = write_standard_set(&dir).unwrap();
        for (path, format) in [
            (&set.splat, SplatFormat::Splat),
            (&set.ply, SplatFormat::Ply),
            (&set.spz, SplatFormat::Spz),
        ] {
            let info = inspect(path).unwrap();
            assert_eq!(
                (info.format, info.count),
                (format, 4000),
                "{}",
                path.display()
            );
        }
        // Content decides, not the extension.
        let disguised = dir.join("really-a-png.splat");
        std::fs::write(&disguised, quadrants_png(8, 8)).unwrap();
        assert!(inspect(&disguised).is_err());
        std::fs::remove_dir_all(dir).ok();
    }
}

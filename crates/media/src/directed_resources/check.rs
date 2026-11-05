//! Cheap format checks of a file that already matched its declared SHA-256.
//!
//! The hash proves the bytes are the ones the bundle declared; these checks catch a bundle that
//! declares the wrong thing about them (a JPEG named `.png`, a PNG of another size, a `.spz` that
//! is a PLY). They read only a header. Alpha and pixels are *not* decoded: the origin's preflight
//! does that with `sharp`, this crate has no image decoder (limit, documented in F038).

use std::fs::File;
use std::io::Read;
use std::path::Path;

use tessel_editor_core::scene_resources::{ResourceKind, TransportResource};
use tessel_editor_core::splat::{self, SplatFormat};

use super::{Failure, ProblemKind};

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// `path` is the file to read: the verified canonical source (preflight), the staged copy
/// (materialize) or the already-present destination file (reuse).
pub(super) fn content(path: &Path, resource: &TransportResource) -> Result<(), Failure> {
    match resource.kind {
        ResourceKind::Image => png(path, resource),
        ResourceKind::Splat => spz(path, resource),
        // The audio mix is a PCM file the bundle only pins by hash. A grid is never a supported
        // use yet (only preserved sections name one); its size check arrives with its first use.
        ResourceKind::Audio | ResourceKind::Grid => Ok(()),
    }
}

fn invalid(resource: &TransportResource, detail: impl std::fmt::Display) -> Failure {
    Failure::new(
        ProblemKind::InvalidContent,
        format!("{}: {detail}", resource.key.label()),
    )
}

/// Signature, then the IHDR chunk that must come first: length, `IHDR`, width, height.
fn png(path: &Path, resource: &TransportResource) -> Result<(), Failure> {
    let mut header = [0u8; 24];
    let read = File::open(path).and_then(|mut file| file.read_exact(&mut header));
    if let Err(error) = read {
        return Err(invalid(
            resource,
            format!("not a PNG: cannot read its header ({error})"),
        ));
    }
    if header[..8] != PNG_SIGNATURE || &header[12..16] != b"IHDR" {
        return Err(invalid(resource, "declared a png but the file is not one"));
    }
    let width = u32::from_be_bytes([header[16], header[17], header[18], header[19]]);
    let height = u32::from_be_bytes([header[20], header[21], header[22], header[23]]);
    let width_matches = resource
        .width_px
        .is_none_or(|expected| expected == u64::from(width));
    let height_matches = resource
        .height_px
        .is_none_or(|expected| expected == f64::from(height));
    if width_matches && height_matches {
        return Ok(());
    }
    Err(Failure::new(
        ProblemKind::DimensionMismatch,
        format!(
            "{} declares {:?}x{:?} px but the PNG is {width}x{height}",
            resource.key.label(),
            resource.width_px,
            resource.height_px
        ),
    ))
}

fn spz(path: &Path, resource: &TransportResource) -> Result<(), Failure> {
    match splat::inspect(path) {
        Ok(info) if info.format == SplatFormat::Spz => Ok(()),
        Ok(info) => Err(invalid(
            resource,
            format!("declared spz but the file is {}", info.format.label()),
        )),
        Err(message) => Err(invalid(resource, message)),
    }
}

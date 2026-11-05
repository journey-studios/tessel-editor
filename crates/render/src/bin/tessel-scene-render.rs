//! Headless render of a directed scene: real frames from the Three.js runtime, no window.
//!
//! ```text
//! tessel-scene-render --bundle bundle.json [--root DIR | --resources map.json]
//!     [--generation N] [--frame N]... [--size WxH] [--probe X,Y]... [--out DIR]
//!     [--gl swiftshader|gpu]
//! ```
//!
//! The frames are rendered in the order given (any order: a seek is not a sequence). One JSON
//! line per frame on stdout: `{"frame", "ok": true, "hash", "mean", "probes", "png", ...}` or
//! `{"frame", "ok": false, "error": {code, message, details?}}`. The exit code is 1 when a
//! frame, the bundle or a resource failed, 2 for a usage error.
//!
//! Resources: `--root` is the approved folder the bundle's `relativePath`s live under (default:
//! the bundle's own folder); `--resources` is an explicit map `{generation, resources:
//! {assetId: {revisionId, contentHash, path}}}` and wins over `--root`. The generation is the
//! host's project generation (default 1). Contract: `docs/features/scene-renderer.md`.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::{Value, json};
use tessel_editor_core::backend::{BackendError, FrameResult};
use tessel_editor_core::bridge::BridgeBackend;
use tessel_editor_core::scene_plan::ScenePlan;
use tessel_editor_render::scene::import_gated;
use tessel_editor_render::{
    FrameOptions, ProcessTransport, RuntimeConfig, SceneRenderer, SceneResourceMap,
};

const USAGE: &str = "Usage: tessel-scene-render --bundle FILE [--root DIR | --resources FILE] [--generation N] [--frame N]... [--size WxH] [--probe X,Y]... [--out DIR] [--gl swiftshader|gpu]";

struct Args {
    bundle: PathBuf,
    root: Option<PathBuf>,
    resources: Option<PathBuf>,
    generation: u64,
    frames: Vec<i64>,
    size: Option<(u32, u32)>,
    probes: Vec<(u32, u32)>,
    out: Option<PathBuf>,
    gl: Option<String>,
}

fn pair(text: &str, separator: char) -> Option<(u32, u32)> {
    let (left, right) = text.split_once(separator)?;
    Some((left.trim().parse().ok()?, right.trim().parse().ok()?))
}

fn parse_args() -> Result<Args, String> {
    let mut args = std::env::args().skip(1);
    let mut parsed = Args {
        bundle: PathBuf::new(),
        root: None,
        resources: None,
        generation: 1,
        frames: Vec::new(),
        size: None,
        probes: Vec::new(),
        out: None,
        gl: None,
    };
    let mut bundle = None;
    while let Some(flag) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match flag.as_str() {
            "--bundle" => bundle = Some(PathBuf::from(value("--bundle")?)),
            "--root" => parsed.root = Some(PathBuf::from(value("--root")?)),
            "--resources" => parsed.resources = Some(PathBuf::from(value("--resources")?)),
            "--generation" => {
                parsed.generation = value("--generation")?
                    .parse()
                    .map_err(|_| "--generation is a non-negative integer".to_string())?;
            }
            "--frame" => parsed.frames.push(
                value("--frame")?
                    .parse()
                    .map_err(|_| "--frame is an integer".to_string())?,
            ),
            "--size" => {
                parsed.size =
                    Some(pair(&value("--size")?, 'x').ok_or_else(|| "--size is WxH".to_string())?);
            }
            "--probe" => parsed
                .probes
                .push(pair(&value("--probe")?, ',').ok_or_else(|| "--probe is X,Y".to_string())?),
            "--out" => parsed.out = Some(PathBuf::from(value("--out")?)),
            "--gl" => {
                let gl = value("--gl")?;
                if gl != "swiftshader" && gl != "gpu" {
                    return Err("--gl is swiftshader or gpu".to_string());
                }
                parsed.gl = Some(gl);
            }
            other => return Err(format!("unknown argument `{other}`")),
        }
    }
    parsed.bundle = bundle.ok_or_else(|| "--bundle is required".to_string())?;
    if parsed.frames.is_empty() {
        parsed.frames.push(0);
    }
    Ok(parsed)
}

fn print(value: &Value) {
    println!("{value}");
}

fn failure(frame: Option<i64>, error: &BackendError) -> Value {
    json!({ "frame": frame, "ok": false, "error": error.to_json() })
}

fn success(frame: i64, result: &FrameResult) -> Value {
    json!({
        "frame": frame, "ok": true, "width": result.width, "height": result.height,
        "hash": result.hash, "mean": result.mean, "probes": result.probes, "png": result.png,
        "render_ms": result.render_ms, "total_ms": result.total_ms,
        "cache": result.detail.cache, "source": result.detail.source,
    })
}

fn read(path: &Path, what: &str) -> Result<String, BackendError> {
    fs::read_to_string(path).map_err(|error| {
        BackendError::new(
            "invalid_request",
            format!("Cannot read the {what} `{}`: {error}", path.display()),
        )
    })
}

fn resource_map(args: &Args, plan: &ScenePlan) -> Result<SceneResourceMap, BackendError> {
    if let Some(path) = &args.resources {
        let value: Value = serde_json::from_str(&read(path, "resource map")?).map_err(|error| {
            BackendError::new(
                "invalid_request",
                format!("The resource map is not JSON: {error}"),
            )
        })?;
        return SceneResourceMap::from_json(&value);
    }
    let root = match &args.root {
        Some(root) => root.clone(),
        None => args
            .bundle
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(".")),
    };
    SceneResourceMap::from_root(args.generation, plan, &root)
}

fn run(args: &Args) -> Result<bool, BackendError> {
    let scene = import_gated(&read(&args.bundle, "bundle")?)?;
    let config = RuntimeConfig::discover().map_err(BackendError::from)?;
    let mut config = config;
    config.gl = args.gl.clone().or(config.gl);
    if let Some(out) = &args.out {
        fs::create_dir_all(out).map_err(|error| {
            BackendError::new(
                "invalid_request",
                format!("Cannot create `{}`: {error}", out.display()),
            )
        })?;
    }
    let backend = BridgeBackend::new(
        "three-worker",
        ProcessTransport::new(config),
        tessel_editor_render::HANDSHAKE_TIMEOUT,
    );
    let mut renderer = SceneRenderer::new(backend);
    let mut all_ok = true;
    for &frame in &args.frames {
        let outcome = ScenePlan::build(&scene, frame)
            .map_err(|error| tessel_editor_render::scene::plan_error(&error))
            .and_then(|plan| {
                let resources = resource_map(args, &plan)?;
                let options = FrameOptions {
                    size: args.size,
                    png: args
                        .out
                        .as_ref()
                        .map(|out| out.join(format!("frame-{frame}.png"))),
                    probes: args.probes.clone(),
                    inspect: true,
                };
                renderer.render(&plan, &resources, &options)
            });
        match outcome {
            Ok(result) => print(&success(frame, &result)),
            Err(error) => {
                all_ok = false;
                print(&failure(Some(frame), &error));
            }
        }
    }
    renderer.shutdown();
    Ok(all_ok)
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(&args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            print(&failure(None, &error));
            ExitCode::FAILURE
        }
    }
}

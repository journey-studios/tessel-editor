//! Real directed-scene frames from the real runtime (Node worker + headless Chrome, SwiftShader),
//! no window involved. The oracle is the synthetic corpus of `scripts/directed-scene-proof`: its
//! pixel probes were computed by independent maths (`model.py`), not by this renderer.
//!
//! A machine without Chrome or the runtime bundle skips these with the reason on stderr; set
//! `TESSEL_REQUIRE_RUNTIME=1` (CI) to turn the skip into a failure. Run them under the shared
//! GPU lock (`/tmp/tessel-scene-gpu.lock`) when other agents may use Chrome.

mod scene_support;

use std::path::PathBuf;
use std::process::Command;

use scene_support::{corpus, expected, plan, resources, scene};
use serde_json::{Value, json};
use tessel_editor_core::backend::{BackendError, FrameResult};
use tessel_editor_core::scene_plan::Capability;
use tessel_editor_render::{
    FrameOptions, HANDSHAKE_TIMEOUT, ProcessTransport, RuntimeConfig, SceneRenderer,
};

/// Same bound as the corpus `tolerances.pixel`: flat blocks, so a channel can move a few units
/// with texture filtering and the sRGB round trip, never change the layer.
const PIXEL_TOLERANCE: i32 = 6;

fn skip(reason: &str) -> Option<SceneRenderer<ProcessTransport>> {
    assert!(
        std::env::var("TESSEL_REQUIRE_RUNTIME").as_deref() != Ok("1"),
        "the runtime is required but unavailable: {reason}"
    );
    eprintln!("SKIPPED (runtime unavailable): {reason}");
    None
}

fn renderer() -> Option<SceneRenderer<ProcessTransport>> {
    let config = match RuntimeConfig::discover() {
        Ok(config) => config,
        Err(setup) => return skip(&setup.message()),
    };
    let backend = tessel_editor_core::bridge::BridgeBackend::new(
        "three-worker",
        ProcessTransport::new(config),
        HANDSHAKE_TIMEOUT,
    );
    let mut renderer = SceneRenderer::new(backend);
    // The handshake starts Chrome; a missing browser shows up here as backend_unavailable.
    match renderer.ensure_ready() {
        Ok(_) => Some(renderer),
        Err(error) if error.code == "backend_unavailable" && !error.message.contains("Rebuild") => {
            skip(&error.message)
        }
        Err(error) => panic!("{error:?}"),
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("tessel-scene-frames-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn probe_points(frame: &Value) -> Vec<(u32, u32)> {
    frame["pixelProbes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|probe| {
            (
                probe["x"].as_u64().unwrap() as u32,
                probe["y"].as_u64().unwrap() as u32,
            )
        })
        .collect()
}

/// Every probe of `frame` (an entry of `expected.json`) against what the renderer read back.
/// Returns the worst channel difference it saw, for the evidence line.
fn assert_probes(result: &FrameResult, frame: &Value, what: &str) -> i32 {
    let mut worst = 0;
    let probes = frame["pixelProbes"].as_array().unwrap();
    assert_eq!(result.probes.len(), probes.len(), "{what}");
    for (read, probe) in result.probes.iter().zip(probes) {
        let color: Vec<i32> = probe["color"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c.as_i64().unwrap() as i32)
            .collect();
        for channel in 0..3 {
            worst = worst.max((i32::from(read[channel]) - color[channel]).abs());
            assert!(
                (i32::from(read[channel]) - color[channel]).abs() <= PIXEL_TOLERANCE,
                "{what}: probe ({}, {}) of `{}` read {read:?}, expected about {color:?}",
                probe["x"],
                probe["y"],
                probe["layer"]
            );
        }
        assert_eq!(read[3], 255, "{what}: the canvas is opaque");
    }
    worst
}

fn frame_of(frames: &Value, edit_frame: i64) -> &Value {
    frames
        .as_array()
        .unwrap()
        .iter()
        .find(|frame| frame["editFrame"] == edit_frame)
        .unwrap_or_else(|| panic!("no expected frame {edit_frame}"))
}

fn render(
    renderer: &mut SceneRenderer<ProcessTransport>,
    scene: &tessel_editor_core::directed_scene::DirectedScene,
    edit_frame: i64,
    generation: u64,
    probes: Vec<(u32, u32)>,
) -> Result<FrameResult, BackendError> {
    let plan = plan(scene, edit_frame);
    let map = resources(generation, &plan);
    renderer.render(
        &plan,
        &map,
        &FrameOptions {
            probes,
            ..FrameOptions::default()
        },
    )
}

fn png_size(path: &std::path::Path) -> (u32, u32) {
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "not a PNG");
    (
        u32::from_be_bytes(bytes[16..20].try_into().unwrap()),
        u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
    )
}

#[test]
fn the_runtime_advertises_every_capability_the_core_can_ask_for_and_nothing_it_cannot_parse() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let capabilities = renderer.ensure_ready().unwrap().clone();
    let block = &capabilities.raw["capabilities"]["directed_scene"];
    assert_eq!(block["layer_kind"], "directed_scene");
    assert_eq!(block["plan_versions"], json!([1]));
    let mut advertised: Vec<Capability> = block["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|name| {
            Capability::parse(name.as_str().unwrap())
                .unwrap_or_else(|| panic!("the core does not know `{name}`"))
        })
        .collect();
    advertised.sort_by_key(|c| c.as_str());
    let mut all = Capability::ALL.to_vec();
    all.sort_by_key(|c| c.as_str());
    assert_eq!(advertised, all);
    // The feature list, not `layers` (the kinds of the ordinary plan), is what gates the messages.
    assert!(capabilities.supports("directed_scene"));
    // BL102b: the runtime of this repository lists `asset.identity` and the renderer negotiated it.
    assert!(capabilities.supports("asset.identity"));
    assert!(renderer.asset_identity());
    renderer.shutdown();
}

#[test]
fn frames_match_the_independent_pixel_oracle_in_a_seek_order_that_jumps_and_repeats() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let expected = expected();
    let scene = scene("bundle.json");
    let out = scratch("oracle");
    let mut hashes: std::collections::BTreeMap<i64, String> = std::collections::BTreeMap::new();
    let mut worst = 0;
    for seek in expected["seekOrder"].as_array().unwrap() {
        let edit_frame = seek.as_i64().unwrap();
        let frame = frame_of(&expected["frames"], edit_frame);
        let plan = plan(&scene, edit_frame);
        let png = out.join(format!("frame-{edit_frame}.png"));
        let options = FrameOptions {
            probes: probe_points(frame),
            png: Some(png.clone()),
            ..FrameOptions::default()
        };
        let result = renderer
            .render(&plan, &resources(1, &plan), &options)
            .unwrap();
        worst = worst.max(assert_probes(
            &result,
            frame,
            &format!("frame {edit_frame}"),
        ));
        assert_eq!((result.width, result.height), (320, 180));
        assert_eq!(png_size(&png), (320, 180));
        // The same edit frame is the same picture however the playhead got there.
        let previous = hashes.insert(edit_frame, result.hash.clone());
        assert!(
            previous.is_none_or(|hash| hash == result.hash),
            "frame {edit_frame} changed between visits"
        );
    }
    // The corpus animates: frames that the plan says differ must not all be one picture.
    assert!(
        hashes
            .values()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            > 3,
        "{hashes:?}"
    );
    eprintln!(
        "oracle: {} frames, {} distinct pictures, worst channel difference {worst} (tolerance {PIXEL_TOLERANCE})",
        expected["seekOrder"].as_array().unwrap().len(),
        hashes.len()
    );
    // Every load was stamped and answered by its own echo: a refused answer would have timed out.
    assert!(renderer.asset_identity());
    assert_eq!(renderer.refused_asset_states(), 0);
    renderer.release_all().unwrap();
    renderer.shutdown();
    let _ = std::fs::remove_dir_all(out);
}

#[test]
fn a_cold_renderer_draws_the_same_picture_as_one_that_has_seen_many_frames() {
    let (Some(mut warm), Some(mut cold)) = (renderer(), renderer()) else {
        return;
    };
    let scene = scene("bundle.json");
    for edit_frame in [95, 0, 48, 12, 36] {
        render(&mut warm, &scene, edit_frame, 1, vec![]).unwrap();
    }
    let seen = render(&mut warm, &scene, 20, 1, vec![]).unwrap();
    let first = render(&mut cold, &scene, 20, 1, vec![]).unwrap();
    assert_eq!(seen.hash, first.hash);
    // And the retention actually retained: the warm runtime created no new texture for the
    // pictures it already held (the cache counters are the runtime's own).
    let created = seen
        .detail
        .cache
        .as_ref()
        .map(|cache| cache["texture_created"].as_u64());
    assert_eq!(created, Some(Some(0)), "{:?}", seen.detail.cache);
    warm.shutdown();
    cold.shutdown();
}

#[test]
fn swapping_the_project_resource_changes_the_pixels_and_the_old_generation_is_refused() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let expected = expected();
    let first_scene = scene("bundle.json");
    let swap_scene = scene("bundle.swap.json");

    let first = render(
        &mut renderer,
        &first_scene,
        0,
        1,
        probe_points(frame_of(&expected["frames"], 0)),
    )
    .unwrap();
    assert_probes(&first, frame_of(&expected["frames"], 0), "hero rev-1");

    // The project is relinked: new generation, hero is another revision of the same assetId.
    for edit_frame in [0, 20, 60] {
        let frame = frame_of(&expected["swapFrames"], edit_frame);
        let swapped = render(
            &mut renderer,
            &swap_scene,
            edit_frame,
            2,
            probe_points(frame),
        )
        .unwrap();
        assert_probes(&swapped, frame, &format!("hero rev-2, frame {edit_frame}"));
        if edit_frame == 0 {
            assert_ne!(
                swapped.hash, first.hash,
                "another revision must not draw the old picture"
            );
        }
    }
    // The old generation does not come back, and nothing about it reaches the runtime.
    let stale = render(&mut renderer, &first_scene, 0, 1, vec![]).unwrap_err();
    assert_eq!(stale.code, "resource_stale");
    assert_eq!(renderer.slots().generation(), 2);

    // Returning to the first project is a new generation: it draws what it drew at the start.
    let back = render(&mut renderer, &first_scene, 0, 3, vec![]).unwrap();
    assert_eq!(back.hash, first.hash);
    renderer.shutdown();
}

#[test]
fn a_burst_of_seeks_shows_the_newest_frame_and_the_older_ones_are_cancelled_or_drawn() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let scene = scene("bundle.json");
    // A single frame first, so the pictures are loaded and the burst is only frames.
    let alone = render(&mut renderer, &scene, 60, 1, vec![]).unwrap();
    let mut submitted = Vec::new();
    for edit_frame in [0, 12, 48, 95, 60] {
        let plan = plan(&scene, edit_frame);
        submitted.push(
            renderer
                .submit(&plan, &resources(1, &plan), &FrameOptions::default())
                .unwrap(),
        );
    }
    let newest = renderer.wait(submitted[4]).unwrap();
    assert_eq!(
        newest.hash, alone.hash,
        "the newest seek is drawn exactly as when it is alone"
    );
    for frame in &submitted[..4] {
        match renderer.wait(*frame) {
            Ok(_) => {}
            Err(error) => assert_eq!(error.code, "cancelled", "{error:?}"),
        }
    }
    // The runtime is not left in a bad state by the burst.
    assert_eq!(
        render(&mut renderer, &scene, 60, 1, vec![]).unwrap().hash,
        alone.hash
    );
    renderer.shutdown();
}

#[test]
fn a_partial_or_unresolved_scene_never_reaches_the_runtime_as_a_picture() {
    let Some(mut renderer) = renderer() else {
        return;
    };
    let scene = scene("bundle.json");
    let plan = plan(&scene, 0);
    let mut partial = plan.clone();
    partial
        .unsupported
        .push(tessel_editor_core::scene_plan::UnsupportedDomain {
            domain: tessel_editor_core::scene_plan::Domain::ALL[0],
            subject: None,
            reason: None,
        });
    let map = resources(1, &plan);
    let error = renderer
        .render(&partial, &map, &FrameOptions::default())
        .unwrap_err();
    assert_eq!(error.code, "unsupported");
    let error = renderer
        .render(
            &plan,
            &tessel_editor_render::SceneResourceMap::new(1),
            &FrameOptions::default(),
        )
        .unwrap_err();
    assert_eq!(error.code, "resource_unresolved");
    assert_eq!(renderer.slots().occupied(), 0);
    // The runtime is fine afterwards.
    renderer
        .render(&plan, &map, &FrameOptions::default())
        .unwrap();
    renderer.shutdown();
}

#[test]
fn the_cli_renders_frames_in_the_order_given_and_reports_failures_per_frame() {
    if RuntimeConfig::discover().is_err() {
        drop(skip("runtime unavailable"));
        return;
    }
    let out = scratch("cli");
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_tessel-scene-render"))
            .args(args)
            .output()
            .unwrap()
    };
    let bundle = corpus().join("bundle.json");
    let output = run(&[
        "--bundle",
        bundle.to_str().unwrap(),
        "--frame",
        "48",
        "--frame",
        "0",
        "--frame",
        "48",
        "--probe",
        "6,12",
        "--out",
        out.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let lines: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        lines
            .iter()
            .map(|line| line["frame"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        [48, 0, 48]
    );
    assert!(lines.iter().all(|line| line["ok"] == true), "{lines:?}");
    assert_eq!(
        lines[0]["hash"], lines[2]["hash"],
        "a repeated seek is the same picture"
    );
    assert_ne!(lines[0]["hash"], lines[1]["hash"]);
    assert_eq!(png_size(&out.join("frame-48.png")), (320, 180));
    // Probe (6, 12) is `sky` at frame 0 in the oracle.
    let frame0 = frame_of(&expected()["frames"], 0).clone();
    let sky = frame0["pixelProbes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|probe| probe["x"] == 6 && probe["y"] == 12)
        .unwrap()
        .clone();
    for channel in 0..3 {
        let read = lines[1]["probes"][0][channel].as_i64().unwrap();
        assert!(
            (read - sky["color"][channel].as_i64().unwrap()).abs() <= i64::from(PIXEL_TOLERANCE)
        );
    }

    // A frame outside the shots fails alone; the others still render and the exit code says so.
    let failing = run(&[
        "--bundle",
        bundle.to_str().unwrap(),
        "--frame",
        "0",
        "--frame",
        "100000",
        "--frame",
        "12",
    ]);
    assert_eq!(failing.status.code(), Some(1));
    let lines: Vec<Value> = String::from_utf8(failing.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        lines
            .iter()
            .map(|line| line["ok"].as_bool().unwrap())
            .collect::<Vec<_>>(),
        [true, false, true]
    );
    assert!(lines[1]["error"]["code"].is_string(), "{lines:?}");

    // A resource root without the pictures is a typed error before any frame is drawn.
    let empty = scratch("cli-empty");
    let missing = run(&[
        "--bundle",
        bundle.to_str().unwrap(),
        "--root",
        empty.to_str().unwrap(),
    ]);
    assert_eq!(missing.status.code(), Some(1));
    let line: Value = serde_json::from_str(
        String::from_utf8(missing.stdout)
            .unwrap()
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        (line["ok"].clone(), line["error"]["code"].clone()),
        (json!(false), json!("resource_unresolved"))
    );

    assert_eq!(
        run(&["--frame", "0"]).status.code(),
        Some(2),
        "a usage error is exit 2"
    );
    let _ = std::fs::remove_dir_all(out);
    let _ = std::fs::remove_dir_all(empty);
}

#[test]
fn the_cli_uses_the_current_directory_for_a_bundle_filename_without_a_parent() {
    if RuntimeConfig::discover().is_err() {
        drop(skip("runtime unavailable"));
        return;
    }
    let output = Command::new(env!("CARGO_BIN_EXE_tessel-scene-render"))
        .current_dir(corpus())
        .args(["--bundle", "bundle.json", "--frame", "0"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let result: Value =
        serde_json::from_slice(output.stdout.split(|b| *b == b'\n').next().unwrap()).unwrap();
    assert_eq!(result["ok"], true);
}

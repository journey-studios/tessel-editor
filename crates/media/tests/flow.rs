//! P12: the whole plan flow through the semantic operations only (the same JSON a CLI, the
//! desktop `ui op` channel or a library caller sends): import → edit → undo → save/reopen →
//! package → originals gone → restore elsewhere → import into another project with colliding
//! IDs → export 1080p / vertical / UHD 4K with audio.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tessel_editor_core::fixtures::{video_frame_color, write_standard_set};
use tessel_editor_core::headless;
use tessel_editor_core::plan::{PlanRequest, PlanTime, PreviewIntent};
use tessel_editor_core::{CommandError, EditorState};
use tessel_editor_media::host::{MediaHost, RendererFactory};

use common::mock::{Mock, Settings};
use common::{temp, tools};

fn host() -> MediaHost {
    let factory: Arc<RendererFactory> = Arc::new(|| Ok(Box::new(Mock::new(Settings::default()))));
    MediaHost::new().with_renderer(factory)
}

fn run(
    host: &mut MediaHost,
    state: &mut EditorState,
    request: Value,
) -> Result<Value, CommandError> {
    headless::execute_ext(state, None, Some(host), &request)
}

fn ok(host: &mut MediaHost, state: &mut EditorState, request: Value) -> Value {
    run(host, state, request.clone()).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn plan_at(state: &EditorState, frame: i64) -> Value {
    state
        .plan(&PlanRequest {
            intent: PreviewIntent::Sequence,
            at: PlanTime::Frame(frame),
        })
        .unwrap()
        .to_json()
}

fn id_of(report: &Value, file_name: &str) -> String {
    report["applied"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == file_name)
        .and_then(|item| item["id"].as_str())
        .unwrap_or_else(|| panic!("{file_name} not imported: {report}"))
        .to_string()
}

fn decode_rgb(file: &Path, index: u64, width: u32, height: u32) -> Vec<u8> {
    let filter = format!(
        "select=eq(n\\,{index}),scale=in_color_matrix=bt709:in_range=tv:out_range=pc,format=rgb24"
    );
    let output = Command::new(&tools().ffmpeg)
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(file)
        .args(["-vf", &filter, "-frames:v", "1", "-f", "rawvideo", "-"])
        .output()
        .unwrap();
    assert_eq!(output.stdout.len(), (width * height * 3) as usize);
    output.stdout
}

fn center(file: &Path, index: u64, width: u32, height: u32) -> [i32; 3] {
    let rgb = decode_rgb(file, index, width, height);
    let at = (((height / 2) * width + width / 2) * 3) as usize;
    [
        i32::from(rgb[at]),
        i32::from(rgb[at + 1]),
        i32::from(rgb[at + 2]),
    ]
}

fn assert_color(actual: [i32; 3], frame: i64) {
    let expected = video_frame_color(frame);
    assert!(
        (0..3).all(|c| (actual[c] - i32::from(expected[c])).abs() <= 4),
        "frame {frame}: {actual:?} vs {expected:?}"
    );
}

#[test]
fn import_edit_save_package_restore_merge_and_export() {
    let tools = tools();
    let dir = temp("flow");
    let originals = dir.join("originals");
    let set = write_standard_set(&originals).unwrap();
    let video = common::video_with_audio(&tools, &originals, "clip.mp4");
    let music = common::mp3(&tools, &originals, "music.mp3");
    let workspace_a = dir.join("projects/alpha");

    let mut host = host();
    let mut state = EditorState::demo();
    ok(
        &mut host,
        &mut state,
        json!({"op":"project.create","name":"Alpha","sequence":"main","rate":24,
        "width":640,"height":360,"length":96,"dir":workspace_a}),
    );
    ok(
        &mut host,
        &mut state,
        json!({"op":"project.save","path":workspace_a.join("alpha.tessel.json")}),
    );

    // 1. Import splat, scene, still, video (with audio) and audio of two formats in one batch.
    let paths: Vec<&PathBuf> = vec![
        &set.splat, &set.scene, &set.image, &video, &music, &set.tone,
    ];
    let report = ok(
        &mut host,
        &mut state,
        json!({"op":"media.import","paths":paths}),
    );
    assert_eq!(report["applied"]["imported"], 6, "{report}");
    assert_eq!(report["applied"]["failed"], 0);
    let ids = |name: &str| id_of(&report, name);
    let (scene, still, clip, mp3, tone) = (
        ids("scene.tessel-scene.json"),
        ids("quadrants.png"),
        ids("clip.mp4"),
        ids("music.mp3"),
        ids("tone-440.wav"),
    );
    assert_eq!(
        report["applied"]["decoded"].as_array().unwrap().len(),
        2,
        "video and mp3 are decoded: {report}"
    );
    let after_import = ok(&mut host, &mut state, json!({"op":"history.status"}));
    assert_eq!(
        after_import["history"]["undo_label"], "Import media",
        "the whole batch is one undo unit"
    );

    // 2. Edit: tracks, clips, text, camera.
    for request in [
        json!({"op":"track.add","id":"v1","name":"Video","kind":"video"}),
        json!({"op":"track.add","id":"v2","name":"Overlay","kind":"video"}),
        json!({"op":"track.add","id":"a1","name":"Audio","kind":"audio"}),
        json!({"op":"clip.add","id":"c-scene","track":"v1","asset":scene,"start":0,"duration":48,
            "camera":{"position":[0,1,5],"target":[0,0,0],"fov":50,"orbit":30}}),
        json!({"op":"clip.add","id":"c-still","track":"v1","asset":still,"start":48,"duration":24}),
        json!({"op":"clip.add","id":"c-video","track":"v2","asset":clip,"start":24,"duration":48}),
        json!({"op":"clip.add","id":"c-mp3","track":"a1","asset":mp3,"start":0,"duration":24}),
        json!({"op":"clip.add","id":"c-tone","track":"a1","asset":tone,"start":24,"duration":48}),
    ] {
        ok(&mut host, &mut state, request);
    }
    ok(
        &mut host,
        &mut state,
        json!({"op":"move","target":"clip:c-tone","args":{"frame":36}}),
    );
    assert_eq!(
        state
            .timeline
            .clips
            .iter()
            .find(|c| c.id == "c-tone")
            .unwrap()
            .start,
        36
    );
    ok(&mut host, &mut state, json!({"op":"history.undo"}));
    assert_eq!(
        state
            .timeline
            .clips
            .iter()
            .find(|c| c.id == "c-tone")
            .unwrap()
            .start,
        24,
        "undo restores the clip"
    );

    // 3. Save, reopen: the plan is identical.
    let document = workspace_a.join("alpha.tessel.json");
    ok(&mut host, &mut state, json!({"op":"project.save"}));
    let saved_plan = plan_at(&state, 30);
    let mut reopened = EditorState::demo();
    ok(
        &mut host,
        &mut reopened,
        json!({"op":"project.open","path":document}),
    );
    // No manual `media.prepare`: a reopened project plans, renders and exports on its own.
    assert_eq!(plan_at(&reopened, 30), saved_plan);

    // 4. Package: preflight, create, verify.
    let archive = dir.join("share/alpha.tesselpack");
    let preflight = ok(
        &mut host,
        &mut state,
        json!({"op":"project.package_preflight","path":archive,"format":"archive"}),
    );
    assert_eq!(preflight["ok"], true, "{preflight}");
    let packaged = ok(
        &mut host,
        &mut state,
        json!({"op":"project.package","path":archive,"format":"archive"}),
    );
    assert_eq!(packaged["complete"], true, "{packaged}");
    let verified = ok(
        &mut host,
        &mut state,
        json!({"op":"project.verify","path":archive}),
    );
    assert_eq!(
        (verified["files"].as_u64(), &verified["complete"]),
        (Some(7), &json!(true)),
        "{verified}"
    );

    // 5. The originals disappear; the managed copies and the package are enough.
    fs::rename(&originals, dir.join("originals-gone")).unwrap();
    let mut elsewhere = EditorState::demo();
    let restored = ok(
        &mut host,
        &mut elsewhere,
        json!({"op":"project.restore","path":archive,"folder":dir.join("restored/alpha")}),
    );
    assert_eq!(restored["opened"]["action"], "project.open", "{restored}");
    assert_eq!(
        plan_at(&elsewhere, 30)["layers"],
        saved_plan["layers"],
        "restored project plays the same"
    );
    let status = ok(&mut host, &mut elsewhere, json!({"op":"media.status"}));
    assert_eq!(status["missing"], 0, "{status}");

    // 6. Merge: import the original project's sequence into the restored copy (every ID collides).
    let merged = ok(
        &mut host,
        &mut elsewhere,
        json!({"op":"project.import","source":document,"sequences":["main"]}),
    );
    let renamed = merged["applied"]["sequences"]
        .as_array()
        .map_or(0, Vec::len)
        + merged["plan"]["sequences"].as_array().map_or(0, Vec::len);
    assert!(renamed >= 1, "{merged}");
    assert_eq!(
        elsewhere.sequences.len() + 1,
        2,
        "a second sequence now exists"
    );
    ok(&mut host, &mut elsewhere, json!({"op":"history.undo"}));
    assert_eq!(
        elsewhere.sequences.len() + 1,
        1,
        "the merge is one undo unit"
    );

    // 7. Export with audio: 1080p, vertical and UHD 4K from the restored project.
    ok(&mut host, &mut elsewhere, json!({"op":"project.save"}));
    for (preset, width, height, range) in [
        ("fullhd", 1920u32, 1080u32, json!([0, 48])),
        ("vertical", 1080, 1920, json!([0, 48])),
        ("uhd4k", 3840, 2160, json!([24, 36])),
    ] {
        let target = dir.join(format!("out/{preset}.mp4"));
        let preflight = ok(
            &mut host,
            &mut elsewhere,
            json!({"op":"export.preflight","preset":preset,"destination":target,"range":range}),
        );
        assert_eq!(preflight["ok"], true, "{preset}: {preflight}");
        let report = ok(
            &mut host,
            &mut elsewhere,
            json!({"op":"export.start","preset":preset,"destination":target,"range":range}),
        );
        assert_eq!(report["verification"]["ok"], true, "{preset}: {report}");
        assert_eq!(report["output"], json!({"width":width,"height":height}));
        assert!(report["audio"]["samples"].as_u64().unwrap() > 0);
        assert!(
            report["audio"]["rms"].as_f64().unwrap() > 0.01,
            "{preset}: the mix is audible"
        );
        let first = range[0].as_i64().unwrap();
        assert_color(center(&target, 0, width, height), first);
    }

    // 8. A background export is a job: observable, cancellable, leaves nothing behind.
    let slow: Arc<RendererFactory> = Arc::new(|| {
        Ok(Box::new(Mock::new(Settings {
            delay: Duration::from_millis(60),
            ..Settings::default()
        })))
    });
    let mut slow_host = MediaHost::new().with_renderer(slow);
    let target = dir.join("out/slow.mp4");
    let started = ok(
        &mut slow_host,
        &mut elsewhere,
        json!({"op":"export.start","preset":"fullhd","destination":target,"wait":false}),
    );
    let job = started["job"].as_str().unwrap().to_string();
    loop {
        let info = ok(
            &mut slow_host,
            &mut elsewhere,
            json!({"op":"job.get","job":job}),
        );
        if info["stage"] == "render" && info["done"].as_u64().unwrap_or(0) >= 3 {
            break;
        }
        assert_ne!(info["state"], "failed", "{info}");
        std::thread::sleep(Duration::from_millis(20));
    }
    ok(
        &mut slow_host,
        &mut elsewhere,
        json!({"op":"job.cancel","job":job}),
    );
    let info = ok(
        &mut slow_host,
        &mut elsewhere,
        json!({"op":"job.wait","job":job,"timeout_ms":30000}),
    );
    assert_eq!(info["state"], "cancelled", "{info}");
    assert!(!target.exists());
    let leftovers = fs::read_dir(dir.join("out"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with(".tessel-staging")
        })
        .count();
    assert_eq!(leftovers, 0);
}

#[test]
fn media_operations_report_typed_errors() {
    let dir = temp("flow-errors");
    let mut host = host();
    let mut state = EditorState::demo();
    let error = run(
        &mut host,
        &mut state,
        json!({"op":"media.import","paths":[dir.join("x.png")]}),
    )
    .unwrap_err();
    assert_eq!(
        error.code, "no_workspace",
        "an unsaved project has nowhere to put media"
    );
    ok(
        &mut host,
        &mut state,
        json!({"op":"project.create","name":"E","dir":dir.join("w")}),
    );
    let report = ok(
        &mut host,
        &mut state,
        json!({"op":"media.import","paths":[dir.join("missing.png")]}),
    );
    assert_eq!(report["applied"]["failed"], 1);
    assert_eq!(
        run(&mut host, &mut state, json!({"op":"job.get","job":"nope"}))
            .unwrap_err()
            .code,
        "unknown_element"
    );
    assert_eq!(
        run(&mut host, &mut state, json!({"op":"media.bogus"}))
            .unwrap_err()
            .code,
        "invalid_request"
    );
    // A host without a renderer cannot export; the one above has a (mock) renderer.
    let mut bare = MediaHost::new();
    assert_eq!(
        run(
            &mut bare,
            &mut state,
            json!({"op":"export.start","preset":"fullhd","destination":dir.join("a.mp4")})
        )
        .unwrap_err()
        .code,
        "backend_unavailable"
    );
    assert_eq!(
        run(
            &mut bare,
            &mut state,
            json!({"op":"export.preflight","preset":"nope","destination":dir.join("a.mp4")})
        )
        .unwrap_err()
        .code,
        "unknown_element"
    );
}

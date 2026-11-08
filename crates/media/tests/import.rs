//! P03/P05/P06: import, probe, dedup, relink, derived media — with real files and FFmpeg.

mod common;

use std::fs;
use std::path::PathBuf;

use serde_json::json;
use tessel_editor_core::EditorState;
use tessel_editor_core::assets::{MediaKind, Source, Storage};
use tessel_editor_core::fixtures::write_standard_set;
use tessel_editor_core::jobs::{JobKind, JobRegistry};
use tessel_editor_core::state::ProjectSettings;
use tessel_editor_media::derive::Cache;
use tessel_editor_media::fsx::hash_file;
use tessel_editor_media::import::{
    self, ImportMode, ImportRequest, commit, file_state, known_hashes, prepare, relink_candidate,
    search_folder,
};
use tessel_editor_media::probe::probe;

use common::{temp, tools};

fn project(workspace: &std::path::Path) -> EditorState {
    EditorState::empty(&ProjectSettings {
        workspace: Some(workspace.to_path_buf()),
        ..ProjectSettings::default()
    })
    .unwrap()
}

#[test]
fn probe_identifies_by_content_not_by_extension() {
    let tools = tools();
    let dir = temp("probe");
    let set = write_standard_set(&dir.join("set")).unwrap();
    let video = common::video_with_audio(&tools, &dir, "clip.mp4");

    let probed = probe(&tools, &video).unwrap();
    assert_eq!(probed.kind, MediaKind::Video);
    assert_eq!(
        (probed.meta.width, probed.meta.height),
        (Some(320), Some(180))
    );
    assert_eq!(probed.meta.rate.map(|r| (r.num(), r.den())), Some((24, 1)));
    assert!(probed.has_audio && probed.meta.sample_rate == Some(48_000));
    let seconds = probed.meta.duration.unwrap().to_f64();
    assert!((seconds - 2.0).abs() < 0.1, "duration {seconds}");

    // A PNG called .mp4, an MP4 called .png: the bytes decide.
    let liar = dir.join("liar.mp4");
    fs::copy(&set.image, &liar).unwrap();
    assert_eq!(probe(&tools, &liar).unwrap().kind, MediaKind::Still);
    let renamed = dir.join("renamed.png");
    fs::copy(&video, &renamed).unwrap();
    assert_eq!(probe(&tools, &renamed).unwrap().kind, MediaKind::Video);

    assert_eq!(
        probe(&tools, &common::jpeg(&tools, &dir, "p.jpg"))
            .unwrap()
            .meta
            .format
            .as_deref(),
        Some("jpeg")
    );
    let wav = probe(&tools, &set.tone).unwrap();
    assert_eq!(
        (wav.kind, wav.meta.format.as_deref()),
        (MediaKind::Audio, Some("wav"))
    );
    let mp3 = probe(&tools, &common::mp3(&tools, &dir, "a.mp3")).unwrap();
    assert_eq!(
        (mp3.kind, mp3.meta.format.as_deref()),
        (MediaKind::Audio, Some("mp3"))
    );
    for (path, format) in [(&set.splat, "splat"), (&set.ply, "ply"), (&set.spz, "spz")] {
        let probed = probe(&tools, path).unwrap();
        assert_eq!(
            (
                probed.kind,
                probed.meta.format.as_deref(),
                probed.meta.splat_count
            ),
            (MediaKind::Splat, Some(format), Some(4000))
        );
    }

    // Rejections are typed and name the file.
    let garbage = dir.join("garbage.bin");
    fs::write(&garbage, b"definitely not media").unwrap();
    assert_eq!(
        probe(&tools, &garbage).unwrap_err().code,
        "unsupported_format"
    );
    let empty = dir.join("empty.mp4");
    fs::write(&empty, b"").unwrap();
    assert_eq!(
        probe(&tools, &empty).unwrap_err().code,
        "unsupported_format"
    );
    let generic_ply = dir.join("points.ply");
    fs::write(&generic_ply, "ply\nformat binary_little_endian 1.0\nelement vertex 1\nproperty float x\nproperty float y\nproperty float z\nend_header\n\0\0\0\0\0\0\0\0\0\0\0\0").unwrap();
    assert!(
        probe(&tools, &generic_ply)
            .unwrap_err()
            .message
            .contains("Gaussian"),
        "a plain point cloud is not accepted as splat"
    );
    assert_eq!(
        probe(&tools, &dir.join("missing.mp4")).unwrap_err().code,
        "asset_unreadable"
    );
    fs::remove_dir_all(dir).ok();
}

/// Imports through the public two-phase API (prepare in a job, commit in one transaction).
fn import_files(
    tools: &tessel_editor_media::tools::Tools,
    state: &mut EditorState,
    workspace: &std::path::Path,
    paths: Vec<PathBuf>,
    mode: ImportMode,
) -> serde_json::Value {
    let registry = JobRegistry::new(2);
    let known = known_hashes(state);
    let request = ImportRequest {
        paths,
        mode,
        folder: None,
        capture: Default::default(),
    };
    let mut prepared = Vec::new();
    let (_, outcome) = registry.run_inline(JobKind::Import, "import", |ctx| {
        prepared = prepare(tools, workspace, &request, &known, ctx)?;
        Ok(json!(null))
    });
    outcome.unwrap();
    commit(state, &prepared, None).unwrap()
}

#[test]
fn batch_import_copies_dedups_and_reports_each_file() {
    let tools = tools();
    let dir = temp("import");
    let workspace = dir.join("ws");
    let set = write_standard_set(&dir.join("set")).unwrap();
    let video = common::video_with_audio(&tools, &dir, "clip.mp4");
    let garbage = dir.join("notes.txt");
    fs::write(&garbage, "hello").unwrap();
    let originals: Vec<(PathBuf, Vec<u8>)> = [&set.image, &set.tone, &set.splat, &video]
        .iter()
        .map(|p| ((*p).clone(), fs::read(p).unwrap()))
        .collect();

    let mut state = project(&workspace);
    let report = import_files(
        &tools,
        &mut state,
        &workspace,
        vec![
            set.image.clone(),
            video.clone(),
            set.tone.clone(),
            set.splat.clone(),
            garbage.clone(),
            set.image.clone(),
        ],
        ImportMode::Copy,
    );
    assert_eq!(report["imported"], 4);
    assert_eq!(report["failed"], 1, "one bad file does not stop the batch");
    assert_eq!(
        report["duplicates"], 1,
        "the repeated image is a duplicate inside the batch"
    );
    let items = report["items"].as_array().unwrap();
    assert_eq!(items[4]["error"]["code"], "unsupported_format");
    assert_eq!(items[5]["duplicate_of"], items[0]["id"]);
    assert_eq!(
        state
            .library
            .items
            .iter()
            .filter(|i| i.asset.is_some() && i.kind != MediaKind::Timeline)
            .count(),
        4
    );

    // Managed copies live under media/<sha256>/<name>, bit-identical, relative-ready.
    let image = state
        .library
        .item("quadrants")
        .unwrap()
        .asset
        .as_ref()
        .unwrap();
    let Source::File { path } = &image.source else {
        panic!()
    };
    let hash = image.identity.hash.clone().unwrap();
    assert_eq!(
        PathBuf::from(path),
        workspace.join("media").join(&hash).join("quadrants.png")
    );
    assert_eq!(image.identity.storage, Storage::Managed);
    assert_eq!(fs::read(path).unwrap(), fs::read(&set.image).unwrap());
    assert_eq!(
        hash_file(&PathBuf::from(path), &Default::default(), |_| {})
            .unwrap()
            .0,
        hash
    );
    assert_eq!(image.identity.origin.as_deref(), set.image.to_str());

    // The batch is ONE undo step; originals are never touched, even by undo.
    let before = state.history.to_json()["undo_depth"].as_u64().unwrap();
    assert!(before >= 1);
    state.undo().unwrap();
    assert!(state.library.item("quadrants").is_none());
    assert!(
        workspace.join("media").join(&hash).exists(),
        "undo drops references, not bytes"
    );
    for (path, bytes) in &originals {
        assert_eq!(&fs::read(path).unwrap(), bytes, "{}", path.display());
    }
    state.redo().unwrap();
    assert!(state.library.item("quadrants").is_some());

    // Importing again finds the bytes already known: no new asset, no new copy.
    let again = import_files(
        &tools,
        &mut state,
        &workspace,
        vec![set.image.clone()],
        ImportMode::Copy,
    );
    assert_eq!(
        (again["imported"].as_i64(), again["duplicates"].as_i64()),
        (Some(0), Some(1))
    );
    assert!(
        fs::read_dir(workspace.join("media")).unwrap().all(|e| !e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".tessel-staging")),
        "no staging left behind"
    );
    fs::remove_dir_all(dir).ok();
}

#[test]
fn reference_mode_leaves_bytes_in_place_and_relink_demands_the_same_content() {
    let tools = tools();
    let dir = temp("relink");
    let workspace = dir.join("ws");
    let set = write_standard_set(&dir.join("set")).unwrap();
    let mut state = project(&workspace);
    import_files(
        &tools,
        &mut state,
        &workspace,
        vec![set.tone.clone(), set.splat.clone()],
        ImportMode::Reference,
    );
    assert!(
        !workspace.join("media").exists()
            || fs::read_dir(workspace.join("media"))
                .unwrap()
                .next()
                .is_none()
    );
    let tone = state
        .library
        .item("tone-440")
        .unwrap()
        .asset
        .clone()
        .unwrap();
    assert_eq!(tone.identity.storage, Storage::Reference);
    assert_eq!(file_state(&tone), import::FileState::Ok);

    // The user moves the file: the project still opens, the asset reads as missing.
    let moved = dir.join("moved");
    fs::create_dir_all(&moved).unwrap();
    fs::rename(&set.tone, moved.join("renamed-tone.wav")).unwrap();
    fs::rename(&set.splat, moved.join("cloud.splat")).unwrap();
    let status = import::status(&state);
    assert_eq!(status["missing"], 2);

    let registry = JobRegistry::new(1);
    // Same name, different bytes: refused.
    let impostor = moved.join("impostor.wav");
    fs::write(
        &impostor,
        tessel_editor_core::fixtures::tone_wav(48_000, 2.0, 880.0, 0.5, 1),
    )
    .unwrap();
    let (_, outcome) = registry.run_inline(JobKind::Probe, "relink", |ctx| {
        relink_candidate(&tools, &tone, &impostor, false, ctx).map(|_| json!(null))
    });
    let error = outcome.unwrap_err();
    assert_eq!(error.code, "relink_mismatch");
    // Different bytes with the same streams need the explicit flag and are reported.
    let (_, outcome) = registry.run_inline(JobKind::Probe, "relink", |ctx| {
        relink_candidate(&tools, &tone, &impostor, true, ctx).map(|(_, warnings)| json!(warnings))
    });
    assert!(
        outcome.unwrap()[0]
            .as_str()
            .unwrap()
            .starts_with("content_changed")
    );

    // Searching the folder finds the originals by size + hash, whatever they are called now.
    let (_, outcome) = registry.run_inline(JobKind::Probe, "search", |ctx| {
        Ok(json!(
            search_folder(&state, &moved, ctx)?
                .into_iter()
                .map(|(id, path)| (id, path.file_name().unwrap().to_string_lossy().into_owned()))
                .collect::<Vec<_>>()
        ))
    });
    let found = outcome.unwrap();
    assert_eq!(found.as_array().unwrap().len(), 2, "{found}");
    // Apply the relink through the core command: clips and ids survive, and it is undoable.
    state
        .execute(tessel_editor_core::state::Command::AddClip {
            id: Some("c1".into()),
            name: None,
            track: "audio-1".into(),
            asset: Some("tone-440".into()),
            start: 0,
            duration: 24,
            source_start: None,
            text: None,
            camera: None,
            properties: vec![],
        })
        .ok();
    let (new_asset, warnings) = relink_candidate(
        &tools,
        &tone,
        &moved.join("renamed-tone.wav"),
        false,
        &registry_ctx(&registry),
    )
    .unwrap();
    assert!(warnings.is_empty());
    state
        .execute(tessel_editor_core::state::Command::ReplaceAsset {
            asset_id: "tone-440".into(),
            asset: Box::new(new_asset),
        })
        .unwrap();
    assert_eq!(import::status(&state)["missing"], 1);
    state.undo().unwrap();
    assert_eq!(import::status(&state)["missing"], 2);
    fs::remove_dir_all(dir).ok();
}

fn registry_ctx(registry: &JobRegistry) -> tessel_editor_core::jobs::JobContext {
    // A context outside a running job: used only to carry a token in a synchronous test.
    let mut slot = None;
    registry
        .run_inline(JobKind::Probe, "ctx", |ctx| {
            slot = Some(ctx.clone());
            Ok(json!(null))
        })
        .1
        .unwrap();
    slot.unwrap()
}

#[test]
fn derived_media_is_cached_invalidated_and_never_replaces_the_original() {
    let tools = tools();
    let dir = temp("derive");
    let workspace = dir.join("ws");
    let video = common::video_with_audio(&tools, &dir, "clip.mp4");
    let mp3 = common::mp3(&tools, &dir, "a.mp3");
    let mut state = project(&workspace);
    import_files(
        &tools,
        &mut state,
        &workspace,
        vec![video.clone(), mp3.clone()],
        ImportMode::Copy,
    );
    let video_asset = state.library.item("clip").unwrap().asset.clone().unwrap();
    let mp3_asset = state.library.item("a").unwrap().asset.clone().unwrap();
    let cache = Cache::new(&workspace);
    let registry = JobRegistry::new(2);
    let ctx = registry_ctx(&registry);

    // Thumbnails: real PNGs of the requested width.
    let thumb = cache
        .thumbnail(&tools, &video_asset, MediaKind::Video, 64, 0.5)
        .unwrap();
    let thumb_probe = probe(&tools, &thumb).unwrap();
    assert_eq!(
        (thumb_probe.kind, thumb_probe.meta.width),
        (MediaKind::Still, Some(64))
    );

    // Waveform of a known tone: constant, non-silent peaks.
    let peaks = cache.waveform(&tools, &mp3_asset, 20, &ctx).unwrap();
    assert_eq!(peaks.len(), 20);
    // FFmpeg's `sine` source has amplitude 1/8.
    assert!(
        peaks[2..18].iter().all(|p| (0.10..0.14).contains(p)),
        "{peaks:?}"
    );

    // Deterministic decode: constant-rate PNG frames, same result when rebuilt from scratch.
    let (frames, reused) = cache.decode_video(&tools, &video_asset, &ctx).unwrap();
    assert!(!reused);
    assert_eq!(frames.count, 48, "2 s at 24 fps");
    let first =
        fs::read(tessel_editor_core::assets::expand_pattern(&frames.pattern, 10).unwrap()).unwrap();
    let (again, reused) = cache.decode_video(&tools, &video_asset, &ctx).unwrap();
    assert!(reused, "an unchanged source reuses the cache");
    assert_eq!(again, frames);
    cache.clear().unwrap();
    let (rebuilt, reused) = cache.decode_video(&tools, &video_asset, &ctx).unwrap();
    assert!(!reused);
    assert_eq!(
        fs::read(tessel_editor_core::assets::expand_pattern(&rebuilt.pattern, 10).unwrap())
            .unwrap(),
        first,
        "decode is deterministic"
    );

    // A different source hash (the file changed) must not hit the old cache.
    let mut changed = video_asset.clone();
    changed.identity.hash = Some("0".repeat(64));
    let (other, reused) = cache.decode_video(&tools, &changed, &ctx).unwrap();
    assert!(!reused);
    assert_ne!(other.pattern, frames.pattern);
    // Same hash but another ffmpeg fingerprint is covered by the manifest; here: tamper it.
    let manifest = PathBuf::from(&frames.pattern)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("cache.json");
    let mut stored: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    stored["fingerprint"]["ffmpeg"] = json!("some other build");
    fs::write(&manifest, serde_json::to_vec(&stored).unwrap()).unwrap();
    assert!(
        !cache.decode_video(&tools, &video_asset, &ctx).unwrap().1,
        "tool change invalidates"
    );

    // Proxy: preview only, same frame count and duration as the original.
    let proxy = cache.proxy(&tools, &video_asset, 90, &ctx).unwrap();
    let proxied = probe(&tools, &proxy).unwrap();
    assert_eq!(proxied.meta.height, Some(90));
    assert_eq!(proxied.meta.rate, video_asset.meta.rate);
    let (a, b) = (
        proxied.meta.duration.unwrap().to_f64(),
        video_asset.meta.duration.unwrap().to_f64(),
    );
    assert!((a - b).abs() < 0.05, "proxy {a} vs original {b}");
    assert_eq!(
        tessel_editor_media::derive::for_export(&video_asset).map(PathBuf::from),
        Some(
            workspace
                .join("media")
                .join(video_asset.identity.hash.clone().unwrap())
                .join("clip.mp4")
        ),
        "export reads the original, never the proxy"
    );

    // Compressed audio gets a WAV the offline mixer reads; the original stays as is.
    let (wav, _) = cache
        .decode_audio(
            &tools,
            &mp3_asset,
            mp3_asset.meta.audio_stream(0).unwrap(),
            &ctx,
        )
        .unwrap();
    let info = tessel_editor_core::audio::wav_info(&fs::read(&wav).unwrap()).unwrap();
    assert_eq!((info.sample_rate, info.channels), (44_100, 1));
    let pruned = cache.prune(0).unwrap();
    assert!(pruned > 0 && cache.size() == 0);
    fs::remove_dir_all(dir).ok();
}

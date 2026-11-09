//! BL144: the rules of library thumbnails and the header identity, through the library and the
//! JSON executor (no window, no renderer): what is wanted, whether a result still belongs, and
//! what the header says after another project is opened.

use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};
use tessel_editor_core::fixtures::{
    SharpSpec, StandardSet, proof_requests, write_sharp_ply, write_standard_set,
};
use tessel_editor_core::splat::measure;
use tessel_editor_core::thumbnails::{Failure, Thumbnail, WIDTH, size_for};
use tessel_editor_core::{CommandError, EditorState, Id, headless};

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tessel-thumbs-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(state: &mut EditorState, request: Value) -> Result<Value, CommandError> {
    headless::execute(state, &request)
}

fn ok(state: &mut EditorState, request: Value) -> Value {
    run(state, request.clone()).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

/// The viewport proof project, with its files "hashed" the way the media import would.
fn project(set: &StandardSet) -> EditorState {
    let mut state = EditorState::demo();
    for request in proof_requests(set) {
        ok(&mut state, request);
    }
    for item in &mut state.library.items {
        if let Some(asset) = item.asset.as_mut() {
            asset.identity.hash = Some(format!("hash-of-{}", item.id));
        }
    }
    state
}

fn picture(state: &EditorState, id: &str, path: &str) -> Thumbnail {
    Thumbnail {
        path: path.to_string(),
        width: WIDTH,
        height: size_for(state.timeline.width, state.timeline.height).1,
        identity: state.thumbnail_identity(id).expect("an identity"),
    }
}

fn ids(state: &EditorState, retry: bool) -> Vec<String> {
    state
        .thumbnail_targets(retry)
        .into_iter()
        .map(|target| target.asset)
        .collect()
}

#[test]
fn only_hashed_splats_and_scenes_are_wanted() {
    let dir = temp("targets");
    let set = write_standard_set(&dir).unwrap();
    let mut state = project(&set);
    assert_eq!(ids(&state, false), ["cloud", "scn"]);
    for id in ["img", "vid", "tone", "label"] {
        assert_eq!(state.thumbnail_report(id)["state"], "none", "{id}");
    }

    // A file nobody hashed has no identity: the cache is addressed by content hash. The scene
    // is drawn with that splat, so it waits for the splat's hash too.
    state.library.items[3].asset.as_mut().unwrap().identity.hash = None;
    assert!(
        state.thumbnail_identity("cloud").is_none(),
        "the unhashed splat has no identity"
    );
    assert!(
        state.thumbnail_identity("scn").is_none(),
        "a scene waits for the hash of what it is drawn with"
    );
    assert!(ids(&state, false).is_empty());
    state.library.items[3].asset.as_mut().unwrap().identity.hash = Some("late".into());
    assert_eq!(ids(&state, false), ["cloud", "scn"]);
}

#[test]
fn the_identity_follows_the_file_and_the_picture_size_not_the_edits() {
    let dir = temp("identity");
    let set = write_standard_set(&dir).unwrap();
    let mut state = project(&set);
    let before = state.thumbnail_identity("cloud").unwrap();
    let revision = state.project.revision;

    // An edit that does not change the picture keeps the identity.
    ok(
        &mut state,
        json!({"op":"clip.property","clip":"c-img","property":"zoom","value":80}),
    );
    assert_ne!(state.project.revision, revision);
    assert_eq!(state.thumbnail_identity("cloud").unwrap(), before);

    // Another file, or another aspect ratio, is another picture.
    state.library.items[3].asset.as_mut().unwrap().identity.hash = Some("another".into());
    let relinked = state.thumbnail_identity("cloud").unwrap();
    assert_ne!(relinked, before);
    state.timeline.width = 1080;
    state.timeline.height = 1920;
    assert_ne!(state.thumbnail_identity("cloud").unwrap(), relinked);
    assert_eq!(
        state.thumbnail_targets(false)[0].height,
        size_for(1080, 1920).1
    );
}

#[test]
fn a_stored_thumbnail_is_shown_only_while_it_is_the_picture_of_the_asset() {
    let dir = temp("show");
    let set = write_standard_set(&dir).unwrap();
    let mut state = project(&set);
    let stamp = state.thumbnail_stamp();
    let revision = state.project.revision;
    let document = state.to_document(Some(&dir));

    state
        .set_thumbnail(
            &stamp,
            "cloud",
            picture(&state, "cloud", "/cache/cloud.png"),
        )
        .unwrap();
    assert_eq!(state.thumbnail("cloud").unwrap().path, "/cache/cloud.png");
    assert_eq!(state.thumbnail_report("cloud")["state"], "ready");
    assert_eq!(
        ids(&state, false),
        ["scn"],
        "a drawn splat is not asked for again"
    );

    // Session state: no edit, nothing in the document, and undo has nothing to give back.
    assert_eq!(state.project.revision, revision);
    assert_eq!(state.to_document(Some(&dir)), document);

    // The file is replaced (relink): the old picture is hidden at once and wanted again.
    state.library.items[3].asset.as_mut().unwrap().identity.hash = Some("replacement".into());
    assert!(state.thumbnail("cloud").is_none());
    assert_eq!(state.thumbnail_report("cloud")["state"], "pending");
    assert_eq!(ids(&state, false), ["cloud", "scn"]);

    // Forgetting what no longer matches keeps the book from growing.
    assert!(!state.thumbnails.is_empty());
    state.prune_thumbnails();
    assert!(state.thumbnails.is_empty());
}

#[test]
fn a_result_of_another_project_or_session_is_refused_and_changes_nothing() {
    let dir = temp("stale");
    let set = write_standard_set(&dir).unwrap();
    let mut first = project(&set);
    let stamp = first.thumbnail_stamp();
    let made = picture(&first, "cloud", "/cache/first.png");

    // A different project with the same asset IDs and hashes: same names, other project.
    let mut other = project(&set);
    let refused = other
        .set_thumbnail(&stamp, "cloud", made.clone())
        .unwrap_err();
    assert_eq!(refused.code, "stale_result", "{refused:?}");
    assert!(other.thumbnails.is_empty());
    assert!(other.thumbnail("cloud").is_none());

    // The same document opened again is a new session even with the same ID and revision.
    let document = dir.join("film.tessel.json");
    ok(&mut first, json!({"op":"project.save","path":document}));
    let mut reopened = EditorState::demo();
    ok(&mut reopened, json!({"op":"project.open","path":document}));
    assert_eq!(reopened.project.id, first.project.id);
    for item in &mut reopened.library.items {
        if let Some(asset) = item.asset.as_mut() {
            asset.identity.hash = Some(format!("hash-of-{}", item.id));
        }
    }
    let refused = reopened
        .set_thumbnail(&stamp, "cloud", made.clone())
        .unwrap_err();
    assert_eq!(refused.code, "stale_result");
    assert!(reopened.thumbnails.is_empty());
    assert_eq!(refused.target.as_deref(), Some("cloud"));

    // The project it was drawn for still takes it.
    first.set_thumbnail(&stamp, "cloud", made).unwrap();
    assert!(first.thumbnail("cloud").is_some());
}

#[test]
fn a_result_for_an_asset_that_changed_while_it_was_drawn_is_refused() {
    let dir = temp("changed");
    let set = write_standard_set(&dir).unwrap();
    let mut state = project(&set);
    let stamp = state.thumbnail_stamp();
    let made = picture(&state, "cloud", "/cache/old.png");
    state.library.items[3].asset.as_mut().unwrap().identity.hash = Some("newer".into());

    let refused = state.set_thumbnail(&stamp, "cloud", made).unwrap_err();
    assert_eq!(refused.code, "stale_result");
    assert!(state.thumbnail("cloud").is_none());
    let refused = state
        .set_thumbnail(&stamp, "ghost", picture(&state, "scn", "/cache/x.png"))
        .unwrap_err();
    assert_eq!(refused.code, "stale_result", "an asset that is gone");
}

#[test]
fn a_failure_is_remembered_for_that_picture_and_retried_on_request() {
    let dir = temp("failure");
    let set = write_standard_set(&dir).unwrap();
    let mut state = project(&set);
    let stamp = state.thumbnail_stamp();
    let identity = state.thumbnail_identity("scn").unwrap();
    state
        .set_thumbnail_failure(
            &stamp,
            "scn",
            Failure {
                identity: identity.clone(),
                code: "backend_unavailable".into(),
                message: "run ./scripts/runtime setup".into(),
            },
        )
        .unwrap();

    let report = state.thumbnail_report("scn");
    assert_eq!(report["state"], "failed");
    assert_eq!(report["code"], "backend_unavailable");
    assert_eq!(
        ids(&state, false),
        ["cloud"],
        "not asked for again every tick"
    );
    assert_eq!(ids(&state, true), ["cloud", "scn"], "unless asked to retry");

    // Another file is another picture: the old failure does not apply to it.
    state.library.items[4].asset.as_mut().unwrap().identity.hash = Some("fixed".into());
    assert_eq!(state.thumbnail_report("scn")["state"], "pending");
    assert!(ids(&state, false).contains(&"scn".to_string()));

    // A success replaces the failure.
    let stamp = state.thumbnail_stamp();
    state
        .set_thumbnail(&stamp, "scn", picture(&state, "scn", "/cache/scn.png"))
        .unwrap();
    assert_eq!(state.thumbnail_report("scn")["state"], "ready");
}

#[test]
fn the_header_follows_the_project_that_is_open() {
    let dir = temp("header");
    let mut state = EditorState::demo();
    let demo = ok(&mut state, json!({"op":"project.header"}));
    assert_eq!(demo["label"], "North expedition / Master sequence");
    assert_eq!(ok(&mut state, json!({"op":"status"}))["header"], demo);

    ok(
        &mut state,
        json!({"op":"project.create","name":"Rain film","sequence":"cut-one","rate":24,
            "width":320,"height":180,"length":48}),
    );
    let created = ok(&mut state, json!({"op":"project.header"}));
    assert_eq!(created["project"], "Rain film");
    assert_eq!(created["sequence"], "cut-one");
    assert_eq!(created["label"], "Rain film / cut-one");
    assert_eq!(created["dirty"], true);
    assert!(created["document"].is_null());
    assert_ne!(created["project_id"], demo["project_id"]);

    // Saving, then opening that document in a fresh session, names the same project.
    let document = dir.join("rain.tessel.json");
    ok(&mut state, json!({"op":"project.save","path":document}));
    let saved = ok(&mut state, json!({"op":"project.header"}));
    assert_eq!(saved["dirty"], false);
    assert_eq!(saved["document"], json!(document));
    let mut reopened = EditorState::demo();
    ok(&mut reopened, json!({"op":"project.open","path":document}));
    let opened = ok(&mut reopened, json!({"op":"project.header"}));
    assert_eq!(opened["label"], "Rain film / cut-one");
    assert_eq!(opened["project_id"], created["project_id"]);

    // Adding and activating another sequence changes the second half, not the project.
    ok(
        &mut reopened,
        json!({"op":"sequence.add","id":"cut-two","name":"Cut two","width":180,"height":320,"activate":true}),
    );
    assert_eq!(
        ok(&mut reopened, json!({"op":"project.header"}))["label"],
        "Rain film / Cut two"
    );
}

fn set_hash(state: &mut EditorState, id: &str, hash: Option<&str>) {
    let item = state
        .library
        .items
        .iter_mut()
        .find(|item| item.id == id)
        .unwrap_or_else(|| panic!("no `{id}` in the library"));
    item.asset.as_mut().unwrap().identity.hash = hash.map(str::to_string);
}

fn requires(state: &mut EditorState, id: &str, required: &[&str]) {
    let item = state
        .library
        .items
        .iter_mut()
        .find(|item| item.id == id)
        .unwrap_or_else(|| panic!("no `{id}` in the library"));
    item.asset.as_mut().unwrap().meta.requires =
        required.iter().map(|id| Id::new(id).unwrap()).collect();
}

/// A second splat, hashed, from the same file (the content is not what these tests are about).
fn add_splat(state: &mut EditorState, set: &StandardSet, id: &str, hash: &str) {
    ok(
        state,
        json!({"op":"asset.add","kind":"splat","id":id,"name":id,"path":set.splat}),
    );
    set_hash(state, id, Some(hash));
}

#[test]
fn a_scene_picture_follows_the_splats_it_is_drawn_with_not_only_its_own_file() {
    let dir = temp("deps");
    let set = write_standard_set(&dir).unwrap();
    let mut state = project(&set);
    let scene_hash = "hash-of-scn";
    let before = state.thumbnail_identity("scn").unwrap();
    assert!(before.contains(scene_hash), "{before}");
    assert!(before.contains("cloud=splat:hash-of-cloud"), "{before}");
    let stamp = state.thumbnail_stamp();
    state
        .set_thumbnail(&stamp, "scn", picture(&state, "scn", "/cache/scn.png"))
        .unwrap();
    assert_eq!(state.thumbnail_report("scn")["state"], "ready");

    // The scene file is byte for byte the same, but the splat it names was relinked: the picture
    // that was ready is gone, the scene is wanted again, the old PNG is never shown.
    set_hash(&mut state, "cloud", Some("relinked"));
    assert_eq!(
        state
            .library
            .item("scn")
            .unwrap()
            .asset
            .as_ref()
            .unwrap()
            .identity
            .hash
            .as_deref(),
        Some(scene_hash)
    );
    let relinked = state.thumbnail_identity("scn").unwrap();
    assert_ne!(relinked, before);
    assert!(state.thumbnail("scn").is_none());
    assert_eq!(state.thumbnail_report("scn")["state"], "pending");
    assert!(ids(&state, false).contains(&"scn".to_string()));
    state.prune_thumbnails();
    assert!(
        state.thumbnails.is_empty(),
        "the picture of the old splat is forgotten"
    );

    // Putting the same content back is the same picture again (deterministic, no counter).
    set_hash(&mut state, "cloud", Some("hash-of-cloud"));
    assert_eq!(state.thumbnail_identity("scn").unwrap(), before);
}

#[test]
fn a_failure_for_a_missing_splat_ends_when_the_splat_arrives() {
    let dir = temp("missing");
    let set = write_standard_set(&dir).unwrap();
    let mut state = project(&set);

    // The splat is gone (removed, or never restored): the scene has an identity that says so.
    let present = state.thumbnail_identity("scn").unwrap();
    state.library.items.retain(|item| item.id != "cloud");
    let missing = state.thumbnail_identity("scn").unwrap();
    assert!(missing.contains("cloud=missing"), "{missing}");
    assert_ne!(missing, present);
    let dependencies = state.thumbnail_dependencies("scn");
    assert_eq!(dependencies.len(), 1);
    assert!(dependencies[0].is_missing());

    let stamp = state.thumbnail_stamp();
    state
        .set_thumbnail_failure(
            &stamp,
            "scn",
            Failure {
                identity: missing.clone(),
                code: "missing_dependency".into(),
                message: "needs the splat `cloud`".into(),
            },
        )
        .unwrap();
    assert_eq!(state.thumbnail_report("scn")["state"], "failed");
    assert!(
        ids(&state, false).is_empty(),
        "remembered, not asked for every tick"
    );

    // The splat is imported (not hashed yet: no identity, so nothing is drawn with half a fact).
    ok(
        &mut state,
        json!({"op":"asset.add","kind":"splat","id":"cloud","name":"cloud","path":set.splat}),
    );
    assert!(state.thumbnail_identity("scn").is_none());
    assert_eq!(state.thumbnail_report("scn")["state"], "none");
    // ...and hashed: the scene is pending again, the old failure does not apply to it.
    set_hash(&mut state, "cloud", Some("hash-of-cloud"));
    assert_eq!(state.thumbnail_report("scn")["state"], "pending");
    assert!(state.thumbnail_failure("scn").is_none());
    assert!(ids(&state, false).contains(&"scn".to_string()));
    state.prune_thumbnails();
    assert!(state.thumbnails.is_empty(), "the failure was forgotten");
}

#[test]
fn a_dependency_added_removed_or_reached_through_another_changes_the_identity() {
    let dir = temp("graph");
    let set = write_standard_set(&dir).unwrap();
    let mut state = project(&set);
    let base = state.thumbnail_identity("scn").unwrap();

    // A second splat the scene now names: new dependency, new identity; the order of `requires`
    // does not matter; removing it gives the first identity back.
    add_splat(&mut state, &set, "dust", "hash-of-dust");
    requires(&mut state, "scn", &["cloud", "dust"]);
    let with_dust = state.thumbnail_identity("scn").unwrap();
    assert_ne!(with_dust, base);
    requires(&mut state, "scn", &["dust", "cloud"]);
    assert_eq!(state.thumbnail_identity("scn").unwrap(), with_dust);
    set_hash(&mut state, "dust", Some("another-dust"));
    assert_ne!(state.thumbnail_identity("scn").unwrap(), with_dust);
    set_hash(&mut state, "dust", Some("hash-of-dust"));
    requires(&mut state, "scn", &["cloud"]);
    assert_eq!(state.thumbnail_identity("scn").unwrap(), base);

    // Transitive: the scene names `cloud`, and `cloud` names `dust` (a file that carries its own
    // dependencies). Relinking `dust` changes what the scene is drawn from.
    requires(&mut state, "cloud", &["dust"]);
    let through = state.thumbnail_identity("scn").unwrap();
    assert_ne!(through, base);
    assert!(through.contains("dust=splat:hash-of-dust"), "{through}");
    set_hash(&mut state, "dust", Some("relinked-dust"));
    assert_ne!(state.thumbnail_identity("scn").unwrap(), through);

    // A cycle in the model does not loop and does not list the asset as its own dependency.
    requires(&mut state, "dust", &["scn"]);
    let dependencies = state.thumbnail_dependencies("scn");
    let names: Vec<_> = dependencies.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(names, ["cloud", "dust"]);
}

#[test]
fn another_project_with_the_same_scene_file_but_other_splats_is_another_picture() {
    let dir = temp("remap");
    let set = write_standard_set(&dir).unwrap();
    let first = project(&set);
    let mut second = project(&set);

    // Same scene bytes, same IDs, but the splat behind `cloud` is another file.
    set_hash(&mut second, "cloud", Some("somebody-else"));
    assert_ne!(
        first.thumbnail_identity("scn"),
        second.thumbnail_identity("scn")
    );

    // An interchange that renamed the splat (`cloud` -> `cloud-2`) and rewrote the scene's
    // `requires` with the same map names the renamed asset, with its content.
    let mut renamed = project(&set);
    renamed
        .library
        .items
        .iter_mut()
        .find(|item| item.id == "cloud")
        .unwrap()
        .id = Id::new("cloud-2").unwrap();
    requires(&mut renamed, "scn", &["cloud-2"]);
    let identity = renamed.thumbnail_identity("scn").unwrap();
    assert!(
        identity.contains("cloud-2=splat:hash-of-cloud"),
        "{identity}"
    );
    assert_ne!(Some(identity), first.thumbnail_identity("scn"));

    // A scene whose `requires` still names `cloud` after the rename lacks it: said, not guessed.
    requires(&mut renamed, "scn", &["cloud"]);
    assert!(
        renamed
            .thumbnail_identity("scn")
            .unwrap()
            .contains("cloud=missing")
    );
}

#[test]
fn the_job_draws_from_a_small_private_project_not_from_a_copy_of_the_open_one() {
    let dir = temp("snapshot");
    let set = write_standard_set(&dir).unwrap();
    let mut state = project(&set);
    for zoom in 20..70 {
        ok(
            &mut state,
            json!({"op":"clip.property","clip":"c-img","property":"zoom","value":zoom}),
        );
    }
    let revision = state.project.revision;
    let targets = state.thumbnail_targets(false);
    let snapshot = state.thumbnail_snapshot(&targets).unwrap();

    let held: Vec<_> = snapshot
        .library
        .items
        .iter()
        .map(|item| item.id.as_str())
        .collect();
    assert_eq!(held, ["cloud", "scn"], "only what the targets need");
    assert!(snapshot.timeline.clips.is_empty() && snapshot.timeline.tracks.is_empty());
    assert_eq!(snapshot.project.id, state.project.id);
    assert_eq!(
        (snapshot.timeline.width, snapshot.timeline.height),
        (state.timeline.width, state.timeline.height)
    );
    for target in &targets {
        assert_eq!(
            snapshot.thumbnail_identity(&target.asset).as_deref(),
            Some(target.identity.as_str()),
            "the snapshot gives every target the identity it was asked with"
        );
    }
    assert_eq!(
        state.project.revision, revision,
        "the open project is untouched"
    );

    // A target that is only a splat does not drag the scene in.
    let only_splat: Vec<_> = targets.into_iter().filter(|t| t.asset == "cloud").collect();
    let small = state.thumbnail_snapshot(&only_splat).unwrap();
    assert_eq!(small.library.items.len(), 1);
}

#[test]
fn the_stand_in_id_is_short_stable_and_never_an_existing_asset() {
    let dir = temp("standin");
    let set = write_standard_set(&dir).unwrap();
    let mut state = project(&set);

    let long_a = format!("{}a", "s".repeat(63));
    let long_b = format!("{}b", "s".repeat(63));
    assert_eq!(long_a.len(), 64);
    let (a, b) = (
        state.thumbnail_stand_in_id(&long_a),
        state.thumbnail_stand_in_id(&long_b),
    );
    assert!(a.len() <= 64 && b.len() <= 64, "{a} / {b}");
    assert_ne!(a, b, "ids that differ only at the tail do not collide");
    assert_eq!(a, state.thumbnail_stand_in_id(&long_a), "deterministic");
    assert!(Id::new(&a).is_ok(), "{a} is a valid asset ID");
    assert_eq!(
        state.thumbnail_stand_in_id("cloud").len(),
        "thumb-".len() + 16
    );

    // An asset that already has that very ID: the stand-in takes the next free one.
    let taken = state.thumbnail_stand_in_id("cloud");
    add_splat(&mut state, &set, &taken, "hash-of-taken");
    let next = state.thumbnail_stand_in_id("cloud");
    assert_ne!(next, taken);
    assert_eq!(next, format!("{taken}-2"));
    assert!(Id::new(&next).is_ok());
}

#[test]
fn a_splat_is_measured_the_same_from_splat_ply_and_spz() {
    let dir = temp("measure");
    let set = write_standard_set(&dir).unwrap();
    let raw = measure(&set.splat).unwrap();
    let ply = measure(&set.ply).unwrap();
    let spz = measure(&set.spz).unwrap();
    assert_eq!((raw.count, ply.count, spz.count), (4000, 4000, 4000));
    for other in [&ply, &spz] {
        for axis in 0..3 {
            assert!(
                (raw.min[axis] - other.min[axis]).abs() < 1.5e-3,
                "{raw:?} / {other:?}"
            );
            assert!(
                (raw.max[axis] - other.max[axis]).abs() < 1.5e-3,
                "{raw:?} / {other:?}"
            );
        }
    }
    // The cloud is a unit-ish shell plus a ring of radius 1.6: centred, a few units across.
    let center = raw.center();
    assert!(center.iter().all(|c| c.abs() < 0.2), "{center:?}");
    assert!((1.5..3.5).contains(&raw.radius()), "{}", raw.radius());

    // Not a splat, not measurable: the error names the file.
    let junk = dir.join("junk.splat");
    fs::write(&junk, [1u8, 2, 3]).unwrap();
    assert!(measure(&junk).unwrap_err().contains("Not a Gaussian splat"));
}

fn capture_of<'a>(
    state: &'a mut EditorState,
    id: &str,
) -> &'a mut tessel_editor_core::sharp::Capture {
    state
        .library
        .items
        .iter_mut()
        .find(|item| item.id == id)
        .and_then(|item| item.asset.as_mut())
        .and_then(|asset| asset.meta.capture.as_mut())
        .unwrap_or_else(|| panic!("`{id}` has no capture"))
}

/// BL144.2: the capture of a SHARP splat is part of what its picture is drawn from.
#[test]
fn a_sharp_capture_is_part_of_the_picture_identity_and_a_change_invalidates_the_cache() {
    let dir = temp("capture");
    let set = write_standard_set(&dir).unwrap();
    let mut state = project(&set);
    let plain = state.thumbnail_identity("cloud").unwrap();
    assert!(
        plain.ends_with("|framing=fit:default"),
        "a splat with no capture is framed as before: {plain}"
    );

    // The same SHARP file read with its capture and as a plain splat.
    let file = dir.join("shot.ply");
    write_sharp_ply(&file, &SharpSpec::default()).unwrap();
    for (id, capture) in [("shot", "auto"), ("flat", "ignore")] {
        ok(
            &mut state,
            json!({"op":"asset.add","kind":"splat","id":id,"name":id,"path":file,"capture":capture}),
        );
        set_hash(&mut state, id, Some("same-bytes"));
    }
    let seen = state.thumbnail_identity("shot").unwrap();
    let ignored = state.thumbnail_identity("flat").unwrap();
    assert!(
        seen.contains("|framing=capture:cv1:160x120:"),
        "the identity names the capture: {seen}"
    );
    assert!(ignored.ends_with("|framing=fit:default"), "{ignored}");
    assert_ne!(seen, ignored, "same bytes, another picture");

    // A thumbnail made for that capture is shown while the capture stands.
    let stamp = state.thumbnail_stamp();
    let made = picture(&state, "shot", "/tmp/shot.png");
    state.set_thumbnail(&stamp, "shot", made.clone()).unwrap();
    assert_eq!(state.thumbnail("shot"), Some(&made));
    assert!(!ids(&state, false).contains(&"shot".to_string()));

    // Another capture of the same file (a replaced or corrected one): the old picture is not
    // shown, the asset is wanted again, a late result for the old capture is refused.
    let before = state.thumbnail_identity("shot").unwrap();
    capture_of(&mut state, "shot").fx += 1.0;
    let after = state.thumbnail_identity("shot").unwrap();
    assert_ne!(after, before);
    assert!(state.thumbnail("shot").is_none());
    assert_eq!(state.thumbnail_report("shot")["state"], "pending");
    assert!(ids(&state, false).contains(&"shot".to_string()));
    let stale = state
        .set_thumbnail(&stamp, "shot", made.clone())
        .unwrap_err();
    assert_eq!(stale.code, "stale_result");
    for change in [
        |capture: &mut tessel_editor_core::sharp::Capture| capture.fy += 1.0,
        |capture: &mut tessel_editor_core::sharp::Capture| capture.cx += 1.0,
        |capture: &mut tessel_editor_core::sharp::Capture| capture.cy += 1.0,
        |capture: &mut tessel_editor_core::sharp::Capture| capture.image = [320, 240],
    ] {
        let was = state.thumbnail_identity("shot").unwrap();
        change(capture_of(&mut state, "shot"));
        assert_ne!(state.thumbnail_identity("shot").unwrap(), was);
    }

    // Losing the capture entirely falls back to the common framing.
    state
        .library
        .items
        .iter_mut()
        .find(|item| item.id == "shot")
        .and_then(|item| item.asset.as_mut())
        .unwrap()
        .meta
        .capture = None;
    assert_eq!(state.thumbnail_identity("shot").unwrap(), ignored);
    state.prune_thumbnails();
    assert!(state.thumbnail("shot").is_none());
}

#[test]
fn the_job_snapshot_keeps_the_capture_so_the_adapter_sees_what_the_identity_names() {
    let dir = temp("capture-snapshot");
    let set = write_standard_set(&dir).unwrap();
    let mut state = project(&set);
    let file = dir.join("shot.ply");
    write_sharp_ply(&file, &SharpSpec::default()).unwrap();
    ok(
        &mut state,
        json!({"op":"asset.add","kind":"splat","id":"shot","name":"shot","path":file}),
    );
    set_hash(&mut state, "shot", Some("sharp-bytes"));
    let targets: Vec<_> = state
        .thumbnail_targets(false)
        .into_iter()
        .filter(|target| target.asset == "shot")
        .collect();
    assert_eq!(targets.len(), 1);
    let snapshot = state.thumbnail_snapshot(&targets).unwrap();
    assert_eq!(
        snapshot.thumbnail_identity("shot").as_deref(),
        Some(targets[0].identity.as_str())
    );
    assert_eq!(
        *capture_of(&mut state, "shot"),
        *snapshot
            .library
            .item("shot")
            .and_then(|item| item.asset.as_ref())
            .and_then(|asset| asset.meta.capture.as_ref())
            .expect("the snapshot carries the capture")
    );
}

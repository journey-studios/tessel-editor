//! BL144: the window header names the project that is open, and a library tile shows the picture
//! drawn for the asset it holds now. The header is the shell's (`EditorShell`), so these tests
//! mount the whole window; the pixels of a drawn thumbnail are proven by the real runtime in
//! `crates/render/tests/thumbnail_frames.rs` and by the native screenshot of
//! `scripts/header-thumbnails-proof.py`.
use std::fs;

use serde_json::{Value, json};
use tessel_editor_core::CommandError;
use tessel_editor_core::fixtures::{encode_png, write_standard_set};
use tessel_editor_core::thumbnails::{Thumbnail, WIDTH, size_for};

use super::input_tests::fixture;
use super::*;
use crate::header;
use crate::shell::EditorShell;
use crate::workstation::OpReply;
use core::prelude::v1::test;

fn settle(cx: &mut VisualTestContext) {
    for _ in 0..4 {
        cx.run_until_parked();
        cx.update(|window, cx| window.simulate_next_frame(cx));
    }
    cx.run_until_parked();
}

fn mount(cx: &mut TestAppContext) -> (Entity<EditorShell>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        install_perf_meter(cx);
        crate::theme::apply(cx, true);
    });
    let (shell, cx) = cx.add_window_view(EditorShell::new);
    cx.run_until_parked();
    (shell, cx)
}

fn op(
    view: &Entity<Workstation>,
    cx: &mut VisualTestContext,
    request: Value,
) -> Result<Value, CommandError> {
    let reply = view.update(cx, |view, cx| view.run_op(&request, cx));
    cx.run_until_parked();
    match reply? {
        OpReply::Done(value) => Ok(value),
        OpReply::Pending(_) => panic!("{request} should not wait for a frame"),
    }
}

fn title_width(cx: &mut VisualTestContext) -> f32 {
    settle(cx);
    f32::from(
        cx.debug_bounds("header-title")
            .expect("the header paints the project title")
            .size
            .width,
    )
}

/// The label the automation tree declares for the title node, and the project the status names.
fn declared(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> (String, String) {
    let label = view.read_with(cx, |view, _| view.state.header().label());
    let node = header::node("node:13", &label).expect("the header declares its title node");
    let status = op(view, cx, json!({"op":"status"})).unwrap();
    (
        node["label"].as_str().unwrap().to_string(),
        status["header"]["label"].as_str().unwrap().to_string(),
    )
}

#[gpui_kit::test]
fn the_header_names_the_open_project_and_follows_the_next_one(cx: &mut TestAppContext) {
    let (shell, cx) = mount(cx);
    let view = shell.read_with(cx, |shell, _| shell.workstation.clone());

    // The bootstrap project, as the header used to hard-code it.
    let (node, status) = declared(&view, cx);
    assert_eq!(node, "North expedition / Master sequence");
    assert_eq!(status, node);
    let demo = title_width(cx);

    // Another project replaces it: the same header, now naming that one.
    op(
        &view,
        cx,
        json!({"op":"project.create","name":"Rain","sequence":"cut","width":160,"height":90,"length":48}),
    )
    .unwrap();
    let (node, status) = declared(&view, cx);
    assert_eq!(
        (node.as_str(), status.as_str()),
        ("Rain / cut", "Rain / cut")
    );
    let rain = title_width(cx);
    assert!(
        rain < demo,
        "a shorter title paints narrower: {rain} vs {demo}"
    );

    // The painted text follows the name, not just the declaration: a longer one is wider.
    let directory = std::env::temp_dir().join(format!("tessel-identity-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let document = directory.join("rain.tessel.json");
    op(&view, cx, json!({"op":"project.save","path":document})).unwrap();
    op(
        &view,
        cx,
        json!({"op":"project.create","name":"Winter in the northern highlands","sequence":"main",
            "width":160,"height":90,"length":48}),
    )
    .unwrap();
    let (node, _) = declared(&view, cx);
    assert_eq!(node, "Winter in the northern highlands / main");
    let winter = title_width(cx);
    assert!(winter > rain, "{winter} vs {rain}");

    // Opening the saved document again names the saved project, as the CLI does.
    op(&view, cx, json!({"op":"project.open","path":document})).unwrap();
    let (node, status) = declared(&view, cx);
    assert_eq!(
        (node.as_str(), status.as_str()),
        ("Rain / cut", "Rain / cut")
    );
    assert_eq!(
        title_width(cx),
        rain,
        "the painted title is the one of the opened project"
    );
}

/// A splat the library holds, "hashed" like an imported one, in a project with no workspace (so
/// the window's own job never starts a renderer in a test).
fn project_with_a_splat(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> String {
    let dir = std::env::temp_dir().join(format!("tessel-identity-tile-{}", std::process::id()));
    let set = write_standard_set(&dir).unwrap();
    op(
        view,
        cx,
        json!({"op":"project.create","name":"Tiles","width":160,"height":90,"length":48}),
    )
    .unwrap();
    op(
        view,
        cx,
        json!({"op":"asset.add","kind":"splat","id":"cloud","name":"Cloud","path":set.splat}),
    )
    .unwrap();
    view.update(cx, |view, _| {
        view.state.library.items[0]
            .asset
            .as_mut()
            .unwrap()
            .identity
            .hash = Some("a".repeat(64));
    });
    dir.to_string_lossy().into_owned()
}

fn draw(view: &Entity<Workstation>, cx: &mut VisualTestContext, dir: &str) {
    let png = std::path::Path::new(dir).join("cloud-thumb.png");
    let (_, height) = size_for(160, 90);
    fs::write(
        &png,
        encode_png(WIDTH, height, &vec![200; (WIDTH * height * 4) as usize]),
    )
    .unwrap();
    view.update(cx, |view, cx| {
        let stamp = view.state.thumbnail_stamp();
        let identity = view.state.thumbnail_identity("cloud").unwrap();
        view.state
            .set_thumbnail(
                &stamp,
                "cloud",
                Thumbnail {
                    path: png.to_string_lossy().into_owned(),
                    width: WIDTH,
                    height,
                    identity,
                },
            )
            .unwrap();
        // What the window's tick does after it applies a finished job.
        cx.notify();
    });
}

fn thumbnail_state(view: &Entity<Workstation>, cx: &mut VisualTestContext) -> Value {
    view.read_with(cx, |view, cx| view.tree_node("media:cloud", cx))
        .expect("the splat is in the tree")["state"]["thumbnail"]["state"]
        .clone()
}

#[gpui_kit::test]
fn a_library_tile_shows_the_drawn_picture_only_while_it_is_the_picture_of_its_asset(
    cx: &mut TestAppContext,
) {
    let (view, cx) = fixture(cx);
    let dir = project_with_a_splat(&view, cx);

    // Nothing drawn yet: the tile is the icon, and the tree says the picture is pending.
    settle(cx);
    assert!(cx.debug_bounds("media-thumb-cloud").is_none());
    assert_eq!(thumbnail_state(&view, cx), "pending");

    draw(&view, cx, &dir);
    settle(cx);
    assert!(
        cx.debug_bounds("media-thumb-cloud").is_some(),
        "the tile paints the drawn picture"
    );
    assert_eq!(thumbnail_state(&view, cx), "ready");

    // The file is replaced (a relink): the old picture is not the asset's any more.
    view.update(cx, |view, cx| {
        view.state.library.items[0]
            .asset
            .as_mut()
            .unwrap()
            .identity
            .hash = Some("b".repeat(64));
        cx.notify();
    });
    settle(cx);
    assert!(
        cx.debug_bounds("media-thumb-cloud").is_none(),
        "the previous file's picture is gone from the tile"
    );
    assert_eq!(thumbnail_state(&view, cx), "pending");
}

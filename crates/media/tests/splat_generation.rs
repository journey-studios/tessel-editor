//! Exercise the real JSONL application boundary with a deterministic SHARP substitute.
#![cfg(unix)]

mod common;

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};

use serde_json::{Value, json};
use tessel_editor_core::fixtures::{SharpSpec, encode_png, write_sharp_ply};

#[test]
fn image_generation_registers_the_capture_and_placement_as_one_edit() {
    let dir = common::temp("splat-generation");
    let workspace = dir.join("project");
    let document = workspace.join("scene.tessel.json");
    let image = dir.join("torch scene.png");
    let bad_image = dir.join("broken.png");
    let png = encode_png(2, 2, &[255; 16]);
    fs::write(&image, &png).unwrap();
    fs::write(&bad_image, &png).unwrap();
    let ply = dir.join("fixture.ply");
    write_sharp_ply(&ply, &SharpSpec::default()).unwrap();

    let sharp = dir.join("sharp");
    fs::write(
        &sharp,
        r#"#!/bin/sh
set -eu
test "$1" = predict
test "$2" = -i
test "$4" = -o
test "$6" = -c
test "$8" = --device
case "$3" in *broken.png) echo 'inference failed' >&2; exit 1;; esac
name=${3##*/}
cp "$SHARP_FIXTURE" "$5/${name%.*}.ply"
"#,
    )
    .unwrap();
    fs::set_permissions(&sharp, fs::Permissions::from_mode(0o700)).unwrap();
    let mut cli = Command::new(env!("CARGO_BIN_EXE_tessel-media"))
        .arg("--empty")
        .env("TESSEL_SHARP", sharp)
        .env("TESSEL_SHARP_CHECKPOINT", &ply)
        .env("SHARP_FIXTURE", &ply)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();

    let requests = [
        json!({"op":"project.create","dir":workspace,"length":60}),
        json!({"op":"track.add","id":"video","name":"Video","kind":"video"}),
        json!({"op":"media.splat.generate","path":bad_image}),
        json!({"op":"media.splat.generate","path":image,"place":{"track":"missing","start":0,"duration":24}}),
        json!({"op":"media.splat.generate","path":image,"place":{"id":"shot","track":"video","start":0,"duration":24}}),
        json!({"op":"media.splat.generate","path":image}),
        json!({"op":"plan","frame":0}),
        json!({"op":"project.save","path":document}),
        json!({"op":"project.open","path":document}),
        json!({"op":"plan","frame":0}),
    ];
    let mut stdin = cli.stdin.take().unwrap();
    let mut stdout = BufReader::new(cli.stdout.take().unwrap());
    let mut request = |message: Value| -> Value {
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
        let mut response = String::new();
        stdout.read_line(&mut response).unwrap();
        serde_json::from_str(&response).unwrap()
    };
    let replies: Vec<_> = requests.into_iter().map(&mut request).collect();

    // Both sequences have a "video" track. A job taken after the switch must not write to it.
    let job = request(
        json!({"op":"media.splat.generate","path":image,"wait":false,
        "place":{"id":"late-shot","track":"video","start":24,"duration":24}}),
    );
    assert_eq!(job["ok"], true, "{job}");
    let job = job["result"]["job"].as_str().unwrap();
    for message in [
        json!({"op":"sequence.add","id":"other","name":"Other"}),
        json!({"op":"sequence.activate","sequence":"other"}),
        json!({"op":"track.add","id":"video","name":"Video","kind":"video"}),
        json!({"op":"job.wait","job":job}),
    ] {
        let reply = request(message);
        assert_eq!(reply["ok"], true, "{reply}");
    }
    let before = request(json!({"op":"history.status"}));
    let commit = request(json!({"op":"job.commit","job":job}));
    assert_eq!(commit["error"]["code"], "stale_result", "{commit}");
    assert_eq!(request(json!({"op":"history.status"})), before);
    let plan = request(json!({"op":"plan","frame":24}));
    assert_eq!(plan["result"]["layers"], json!([]), "{plan}");
    drop(stdin);
    assert_eq!(cli.wait().unwrap().code(), Some(1), "Intentional refusals");
    assert_eq!(replies.len(), 10, "{replies:?}");
    assert_eq!(replies[2]["error"]["code"], "sharp_failed");
    assert_eq!(replies[3]["ok"], false);
    assert_eq!(replies[4]["result"]["applied"]["asset"], "torch-scene");
    assert_eq!(replies[4]["result"]["applied"]["duplicate"], false);
    assert_eq!(replies[5]["result"]["applied"]["asset"], "torch-scene");
    assert_eq!(replies[5]["result"]["applied"]["duplicate"], true);
    assert_eq!(replies[5]["result"]["applied"]["changed"], false);
    assert_eq!(
        replies[6]["result"]["layers"][0]["orientation"],
        json!([180.0, 0.0, 0.0])
    );
    assert_eq!(
        replies[6]["result"]["layers"],
        replies[9]["result"]["layers"]
    );
    assert!(!fs::read_dir(&workspace).unwrap().flatten().any(|entry| {
        entry
            .file_name()
            .to_string_lossy()
            .starts_with(".tessel-staging-sharp")
    }));
}

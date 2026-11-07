use std::io::Write;
use std::process::{Command as Process, Stdio};

use serde_json::{Value, json};
use tessel_editor_core::{Command, EditorState, Raw, headless};

#[test]
fn json_and_library_flows_have_the_same_state_and_reject_without_mutation() {
    // Two sessions of the same project: the same identity (the ID every new project gets is
    // random, so a second `default()` would be another project), so the whole summary is compared.
    let mut direct = EditorState::default();
    let mut json_session = direct.clone();
    assert_eq!(direct.summary(), json_session.summary());
    direct
        .execute(Command::SetPlayhead {
            frame: Raw::Number(120.0),
        })
        .unwrap();
    headless::execute(
        &mut json_session,
        &json!({
            "op":"set", "target":"timeline.playhead", "field":"frame", "value":120
        }),
    )
    .unwrap();
    assert_eq!(direct.summary(), json_session.summary());
    let before = json_session.summary();
    let error = headless::execute(
        &mut json_session,
        &json!({
            "op":"move", "target":"clip:first-light", "args":{"frame":-1000}
        }),
    )
    .unwrap_err();
    assert_eq!(error.code, "out_of_bounds");
    assert_eq!(before, json_session.summary());
    assert_eq!(
        headless::execute(
            &mut json_session,
            &json!({
                "op":"invoke", "target":"toolbar.theme", "action":"toggle"
            })
        )
        .unwrap_err()
        .code,
        "desktop_required"
    );
    // F049: the performance meter is window presentation; a session has nothing to measure.
    for request in [
        json!({"op":"invoke", "target":"toolbar.perf_meter", "action":"toggle"}),
        json!({"op":"set", "target":"toolbar.perf_meter", "field":"visible", "value":true}),
    ] {
        assert_eq!(
            headless::execute(&mut json_session, &request)
                .unwrap_err()
                .code,
            "desktop_required"
        );
    }
    assert_eq!(before, json_session.summary());
}

#[test]
fn standalone_cli_keeps_session_state_and_reports_errors_without_a_desktop() {
    let mut child = Process::new(env!("CARGO_BIN_EXE_tessel-headless"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for request in [
        json!({"op":"set","target":"timeline.playhead","field":"frame","value":120}),
        json!({"op":"status"}),
        json!({"op":"move","target":"clip:first-light","args":{"frame":-1000}}),
        json!({"op":"status"}),
    ] {
        writeln!(stdin, "{request}").unwrap();
    }
    writeln!(stdin, "invalid json").unwrap();
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    let responses: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(responses.len(), 5);
    assert_eq!(responses[1]["result"]["playhead"]["frame"], 120);
    assert_eq!(responses[1]["result"], responses[3]["result"]);
    assert_eq!(responses[2]["error"]["code"], "out_of_bounds");
    assert_eq!(responses[4]["error"]["code"], "invalid_json");
}

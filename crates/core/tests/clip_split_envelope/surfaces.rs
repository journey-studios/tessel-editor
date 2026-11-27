use super::*;

#[test]
fn the_control_offers_a_cut_inside_a_ramp_and_nothing_is_refused_for_fades() {
    use tessel_editor_core::state::split::SplitAvailability;
    let mut state = visual_project((24, 1));
    set(&mut state, "t", "fade_in", json!(12));
    set(&mut state, "t", "fade_out", json!(24));
    run(
        &mut state,
        json!({"op":"invoke","target":"clip:t","action":"select"}),
    );
    for frame in [7, 11, 17, 40, 47, 65] {
        run(
            &mut state,
            json!({"op":"set","target":"timeline.playhead","field":"frame","value":frame}),
        );
        assert!(
            matches!(state.split_availability(), SplitAvailability::Ready { .. }),
            "frame {frame}"
        );
    }
    // The node of the window uses the same rule: the invoke cuts where the playhead is.
    let before = state.timeline.clips.len();
    run(
        &mut state,
        json!({"op":"invoke","target":"timeline.split","action":"invoke"}),
    );
    assert_eq!(state.timeline.clips.len(), before + 1);
}

#[test]
fn a_refused_cut_still_changes_nothing_and_a_cut_on_an_edge_is_still_an_edge() {
    let mut state = halves();
    let untouched = fingerprint(&state);
    for (frame, code) in [
        (10, "invalid_duration"),
        (6, "out_of_bounds"),
        (66, "invalid_duration"),
        (100, "out_of_bounds"),
    ] {
        let error = fail(
            &mut state,
            json!({"op":"clip.split","clip":"t-split-1","frame":frame}),
        );
        assert_eq!(error.code, code, "frame {frame}");
        assert_eq!(fingerprint(&state), untouched, "frame {frame}");
    }
    let error = fail(
        &mut state,
        json!({"op":"clip.split","clip":"t-split-1","frame":30,"new_id":"t"}),
    );
    assert_eq!(error.code, "duplicate_id");
    assert_eq!(fingerprint(&state), untouched);
}

#[test]
fn the_headless_process_cuts_inside_the_ramps_and_keeps_picture_and_sound() {
    let dir = shared_dir().join("process");
    fs::create_dir_all(&dir).unwrap();
    let set = write_standard_set(&dir).unwrap();
    let (before_wav, after_wav) = (dir.join("before.wav"), dir.join("after.wav"));
    let mut lines: Vec<Value> = proof_requests(&set);
    let prop = |clip: &str, property: &str, value: i64| json!({"op":"clip.property","clip":clip,"property":property,"value":value});
    lines.extend([
        prop("c-title", "fade_in", 12),
        prop("c-title", "fade_out", 24),
        prop("c-tone", "fade_in", 12),
        prop("c-tone", "fade_out", 24),
        json!({"op":"plan","frame":8}),
        json!({"op":"plan","frame":30}),
        json!({"op":"plan","frame":44}),
        json!({"op":"render.audio","from":0,"frames":48,"out":before_wav,
            "samples":[[0,0],[7000,1],[40000,0],[60001,1],[90000,0]]}),
        json!({"op":"clip.split","clip":"c-title","frame":5}),
        json!({"op":"clip.split","clip":"c-title-split-1","frame":30}),
        json!({"op":"clip.split","clip":"c-tone","frame":5}),
        json!({"op":"clip.split","clip":"c-tone-split-1","frame":30}),
        json!({"op":"plan","frame":8}),
        json!({"op":"plan","frame":30}),
        json!({"op":"plan","frame":44}),
        json!({"op":"render.audio","from":0,"frames":48,"out":after_wav,
            "samples":[[0,0],[7000,1],[40000,0],[60001,1],[90000,0]]}),
        json!({"op":"history.undo"}),
        json!({"op":"plan","frame":30}),
    ]);
    let mut child = Process::new(env!("CARGO_BIN_EXE_tessel-headless"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for line in &lines {
        writeln!(stdin, "{line}").unwrap();
    }
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "no cut inside a fade fails the session"
    );
    let replies: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(replies.len(), lines.len(), "every line is answered");
    assert!(
        replies.iter().all(|reply| reply["ok"] == true),
        "{replies:#?}"
    );
    let n = replies.len();
    // Layers of the same frame before and after: same opacity and geometry (the title).
    let layer = |reply: &Value| {
        let mut layers = reply["result"]["layers"].clone();
        for layer in layers.as_array_mut().unwrap() {
            let map = layer.as_object_mut().unwrap();
            for key in ["id", "clip"] {
                map.remove(key);
            }
        }
        layers
    };
    for (before, after) in [(n - 14, n - 6), (n - 13, n - 5), (n - 12, n - 4)] {
        assert_eq!(
            layer(&replies[before]),
            layer(&replies[after]),
            "plan {before}/{after}"
        );
    }
    // The mix: samples reported by the process and the WAV it wrote are the same.
    let (mixed_before, mixed_after) = (&replies[n - 11]["result"], &replies[n - 3]["result"]);
    assert_eq!(mixed_before["samples"], mixed_after["samples"]);
    assert_eq!(mixed_before["peak"], mixed_after["peak"]);
    assert_eq!(mixed_before["rms"], mixed_after["rms"]);
    assert!(mixed_after["peak"].as_f64().unwrap() > 0.01);
    let wav = |path: &PathBuf| decode_wav(&fs::read(path).unwrap()).unwrap();
    assert_eq!(wav(&before_wav).samples, wav(&after_wav).samples);
    let windows = &replies[n - 10]["result"]["fade_window"];
    assert!(
        windows["left"].is_object() || windows["right"].is_object(),
        "the cut reports the windows it wrote: {windows}"
    );
}

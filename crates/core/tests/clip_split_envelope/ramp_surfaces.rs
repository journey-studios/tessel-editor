//! BL081.2.2 through the headless process: the reported cut and edit, both halves, one session.
use super::ramp_support::*;
use super::*;

#[test]
fn the_headless_process_edits_a_fade_of_a_half_and_leaves_the_other_ramp_alone() {
    let dir = shared_dir().join("process-ramps");
    fs::create_dir_all(&dir).unwrap();
    let wav = |name: &str| dir.join(format!("{name}.wav"));
    let source = ramp(48_000, true);
    let render = |name: &str| {
        json!({"op":"render.audio","from":0,"frames":LENGTH,"out":wav(name),
            "samples":[[2000 * 41, 0], [2000 * 60, 1]]})
    };
    let prop = |clip: &str, property: &str, value: i64| json!({"op":"clip.property","clip":clip,"property":property,"value":value});
    let lines: Vec<Value> = vec![
        json!({"op":"project.create","name":"P","sequence":"main","rate":rate_json((24, 1)),
            "width":64,"height":36,"length":LENGTH}),
        json!({"op":"asset.add","kind":"audio","id":"ramp","name":"Ramp","path":source}),
        json!({"op":"track.add","id":"a1","name":"a1","kind":"audio"}),
        json!({"op":"clip.add","id":"main","track":"a1","asset":"ramp","start":3,"duration":60,
            "source_start":{"num":1,"den":4},"properties":{"pan":-35}}),
        prop("main", "fade_out", 30),
        render("whole"),
        json!({"op":"clip.split","clip":"main","frame":40}),
        render("cut"),
        prop("main-split-1", "fade_in", 5),
        render("edited"),
        json!({"op":"history.undo"}),
        render("undone"),
    ];
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
    assert_eq!(output.status.code(), Some(0));
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

    let read = |name: &str| decode_wav(&fs::read(wav(name)).unwrap()).unwrap();
    let (whole, cut, edited, undone) = (read("whole"), read("cut"), read("edited"), read("undone"));
    assert!(whole.peak() > 0.01);
    same_bits(&whole, &cut, "the cut keeps the sound");
    same_bits(&cut, &undone, "undo of the edit is the cut");
    let end = at((24, 1), LENGTH);
    assert_same(&whole, &edited, 0, 40 * 2000, "left half");
    assert_same(
        &whole,
        &edited,
        45 * 2000,
        end,
        "the fade-out the edit did not touch",
    );
    assert!(
        differs(&whole, &edited, 40 * 2000, 45 * 2000),
        "the new fade-in is heard"
    );
    // The plan the process reports says it too: the fade-out of the right half is the whole's.
    let segment_of = |reply: &Value, clip: &str| {
        reply["result"]["plan"]["segments"]
            .as_array()
            .unwrap()
            .iter()
            .find(|segment| segment["clip"] == clip)
            .cloned()
            .unwrap_or_else(|| panic!("{clip} in {reply}"))
    };
    let whole_plan = segment_of(&replies[5], "main");
    let edited_plan = segment_of(&replies[9], "main-split-1");
    assert_eq!(edited_plan["fade_out"], whole_plan["fade_out"]);
    assert_eq!(edited_plan["fade_in"], 5 * 2000);
    let anchor = |segment: &Value| {
        segment["output_start"].as_u64().unwrap() - segment["clip_offset"].as_u64().unwrap()
            + segment["clip_frames"].as_u64().unwrap()
    };
    assert_eq!(
        anchor(&edited_plan),
        anchor(&whole_plan),
        "the fade-out ends where it did"
    );
    // The samples the process reports are those of the WAV it wrote (PCM16 quantization).
    let reported = replies[9]["result"]["samples"][0]["value"]
        .as_f64()
        .unwrap() as f32;
    assert!((reported - edited.sample(2000 * 41, 0)).abs() < 1e-3);
}

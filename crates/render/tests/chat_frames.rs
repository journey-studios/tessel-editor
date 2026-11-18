//! F070: the native chat (campaign, question that types in, answer by line, source with a mark),
//! drawn by the real runtime (Node worker + headless Chrome). The sequence question → answer →
//! source highlight is proven with pixels on the monitor frame (`render.frame`, PNG) and on the
//! exported H.264 video, with stable line breaks while it types, a retimed sequence after an
//! edit, byte-exact undo and a reopened project that draws the same pixels. Skipped without the
//! runtime unless `TESSEL_REQUIRE_RUNTIME=1`; FFmpeg reads the pixels back
//! (`docs/media-setup.md`). The texts are the preset's fictitious ones: the repo is public.
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use serde_json::{Value, json};
use tessel_editor_core::session::Renderer;
use tessel_editor_core::{EditorState, headless};
use tessel_editor_media::host::{MediaHost, RendererFactory};
use tessel_editor_media::tools::Tools;
use tessel_editor_render::{RuntimeConfig, session};

const WIDTH: u32 = 640;
const HEIGHT: u32 = 360;
const RATE: i64 = 24;
/// Where the preset's texts sit on a 640x360 canvas: `(x0, y0, x1, y1)`, end exclusive.
const QUESTION: Region = (80, 116, 560, 146);
const ANSWER: Region = (80, 166, 560, 216);
const SOURCE: Region = (80, 236, 560, 257);
/// The mark under the source: 34% x 0.7% of the canvas, centred at y 22.5% below the middle.
const MARK: Region = (230, 259, 410, 264);
/// The mark's colour (`#ffb020`) as read back from the picture.
const MARK_COLOUR: [u8; 3] = [255, 176, 32];

type Region = (u32, u32, u32, u32);

struct Rig {
    state: EditorState,
    session: Box<dyn Renderer>,
    tools: Tools,
    dir: PathBuf,
}

fn rig(tag: &str) -> Option<Rig> {
    let config = match RuntimeConfig::discover() {
        Ok(config) => config,
        Err(setup) => {
            assert!(
                std::env::var("TESSEL_REQUIRE_RUNTIME").as_deref() != Ok("1"),
                "the runtime is required but unavailable: {}",
                setup.message()
            );
            eprintln!("SKIPPED (runtime unavailable): {}", setup.message());
            return None;
        }
    };
    let tools = Tools::discover().expect("FFmpeg is required for the chat tests");
    let dir = std::env::temp_dir().join(format!("tessel-chat-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut rig = Rig {
        state: EditorState::demo(),
        session: Box::new(session(&config)),
        tools,
        dir,
    };
    rig.ok(
        json!({"op":"project.create","name":"Chat","sequence":"main","rate":RATE,
        "width":WIDTH,"height":HEIGHT,"length":240}),
    );
    rig.ok(json!({"op":"invoke","target":"timeline.add_chat","action":"invoke"}));
    Some(rig)
}

impl Rig {
    fn ok(&mut self, request: Value) -> Value {
        headless::execute_with(&mut self.state, Some(self.session.as_mut()), &request)
            .unwrap_or_else(|error| panic!("{request}: {error:?}"))
    }

    /// The monitor frame as RGB, read back from the PNG `render.frame` wrote.
    fn frame(&mut self, name: &str, frame: i64) -> Vec<u8> {
        let png = self.dir.join(format!("{name}.png"));
        self.ok(json!({"op":"render.frame","frame":frame,"out":png}));
        decode(&self.tools, &png, &["-frames:v", "1"])
    }

    /// `(start_ms, end_ms)` of every child of the chat, on the clip's clock.
    fn timings(&self, child: &str) -> (i64, i64) {
        let instance = self.state.timeline.clips[0].group.clone().unwrap();
        let def = self.state.groups.def(instance.def.as_str()).unwrap();
        let at = def.children.iter().position(|c| c.id == child).unwrap();
        let timing = self
            .state
            .groups
            .timings(&instance, (WIDTH, HEIGHT))
            .unwrap()[at];
        (timing.start_ms, timing.end_ms)
    }
}

/// The first frame whose clock is `extra` frames past `ms`.
fn frame_after(ms: i64, extra: i64) -> i64 {
    (ms * RATE + 999) / 1000 + extra
}

fn decode(tools: &Tools, file: &Path, extra: &[&str]) -> Vec<u8> {
    let output = Command::new(&tools.ffmpeg)
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(file)
        .args(extra)
        .args(["-pix_fmt", "rgb24", "-f", "rawvideo", "-"])
        .output()
        .unwrap();
    assert_eq!(
        output.stdout.len(),
        (WIDTH * HEIGHT * 3) as usize,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn at(rgb: &[u8], x: u32, y: u32) -> [u8; 3] {
    let offset = ((y * WIDTH + x) * 3) as usize;
    [rgb[offset], rgb[offset + 1], rgb[offset + 2]]
}

/// Pixels of `region` for which `test` holds.
fn count(rgb: &[u8], region: Region, test: impl Fn([u8; 3]) -> bool) -> usize {
    let (x0, y0, x1, y1) = region;
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .filter(|&(x, y)| test(at(rgb, x, y)))
        .count()
}

/// The light text of the question and the answer, on the dark card.
fn light(pixel: [u8; 3]) -> bool {
    pixel.iter().all(|channel| *channel > 150)
}

/// The yellow of the source, in its own rows (the mark is below them).
fn yellow(pixel: [u8; 3]) -> bool {
    pixel[0] > 150 && pixel[1] > 120 && pixel[2] < 130
}

fn near(tolerance: u8) -> impl Fn([u8; 3]) -> bool {
    move |pixel| {
        pixel
            .iter()
            .zip(MARK_COLOUR)
            .all(|(a, w)| a.abs_diff(w) <= tolerance)
    }
}

/// Every pixel of `region` that is lit in `before` is still lit in `after`: nothing that was
/// already drawn moved or went out while more text appeared.
fn keeps_what_was_drawn(before: &[u8], after: &[u8], region: Region) {
    let (x0, y0, x1, y1) = region;
    for y in y0..y1 {
        for x in x0..x1 {
            if light(at(before, x, y)) {
                assert!(
                    at(after, x, y).iter().all(|channel| *channel > 100),
                    "the pixel ({x}, {y}) went out when the text grew"
                );
            }
        }
    }
}

#[test]
fn the_chat_types_the_question_shows_the_answer_by_line_and_marks_the_source() {
    let Some(mut rig) = rig("sequence") else {
        return;
    };
    let (question, answer, source, mark) = (
        rig.timings("question"),
        rig.timings("answer"),
        rig.timings("source"),
        rig.timings("source-mark"),
    );

    // Frame 3 (125 ms): the card is fading in; nothing of the conversation is drawn yet.
    let waiting = rig.frame("waiting", 3);
    assert_eq!(
        count(&waiting, QUESTION, light),
        0,
        "the question waits for its turn"
    );
    assert_eq!(count(&waiting, ANSWER, light), 0);
    assert_eq!(count(&waiting, MARK, near(40)), 0);

    // The question types in: each frame later has more ink, and the answer still waits.
    let early = rig.frame("typing-1", frame_after(question.0, 3));
    let middle = rig.frame("typing-2", frame_after(question.0, 10));
    let late = rig.frame("typing-3", frame_after(question.0, 20));
    let ink = [early, middle, late]
        .iter()
        .map(|frame| count(frame, QUESTION, light))
        .collect::<Vec<_>>();
    assert!(ink[0] > 0 && ink[0] < ink[1] && ink[1] < ink[2], "{ink:?}");
    let typing = rig.frame("typing-answer-waits", frame_after(question.0, 20));
    assert_eq!(
        count(&typing, ANSWER, light),
        0,
        "the answer waits for the question to end"
    );

    // Letters already typed never move: a later frame keeps every lit pixel of an earlier one.
    let typed_whole = rig.frame("question-done", frame_after(question.1, 2));
    keeps_what_was_drawn(&typing, &typed_whole, QUESTION);
    assert!(count(&typed_whole, QUESTION, light) >= ink[2]);
    assert_eq!(
        count(&typed_whole, ANSWER, light),
        0,
        "still inside the 300 ms pause"
    );

    // The answer arrives line by line: the first line, then both, nothing moving.
    let first_line = rig.frame("answer-1", frame_after(answer.0, 1));
    let both_lines = rig.frame("answer-2", frame_after(answer.1, 1));
    let one = count(&first_line, ANSWER, light);
    let two = count(&both_lines, ANSWER, light);
    assert!(one > 0, "the first line shows");
    assert!(
        two > one + one / 3,
        "the second line adds ink: {one} then {two}"
    );
    keeps_what_was_drawn(&first_line, &both_lines, ANSWER);
    assert_eq!(
        count(&first_line, SOURCE, yellow),
        0,
        "the source waits for the answer"
    );

    // The source types in, and only after it the mark appears under it.
    let source_typing = rig.frame("source-typing", frame_after(source.0, 4));
    assert!(
        count(&source_typing, SOURCE, yellow) > 0,
        "the source is typing"
    );
    assert_eq!(
        count(&source_typing, MARK, near(40)),
        0,
        "no mark while the source types"
    );
    let source_done = rig.frame("source-done", frame_after(source.1, 0));
    assert!(count(&source_done, SOURCE, yellow) > count(&source_typing, SOURCE, yellow));
    let marked = rig.frame("marked", frame_after(mark.1, 2));
    let mark_pixels = count(&marked, MARK, near(12));
    assert!(
        mark_pixels > 150,
        "the mark highlights the source: {mark_pixels} pixels"
    );

    // The finished card is the same picture from there on, and the question never changed.
    let final_frame = rig.frame("final", frame_after(mark.1, 30));
    assert_eq!(
        final_frame, marked,
        "after the last step the picture is still"
    );
    assert_eq!(
        count(&final_frame, QUESTION, light),
        count(&typed_whole, QUESTION, light)
    );
}

#[test]
fn editing_a_text_retimes_the_sequence_and_undo_restores_the_pixels_byte_for_byte() {
    let Some(mut rig) = rig("edit") else {
        return;
    };
    let answer = rig.timings("answer");
    let frame = frame_after(answer.0, 2);
    let before = rig.frame("before", frame);
    assert!(count(&before, ANSWER, light) > 0, "the answer is showing");

    // A longer question takes longer to type: at the same frame the answer has not started.
    let clip = rig.state.timeline.clips[0].id.to_string();
    rig.ok(
        json!({"op":"clip.group_text","clip":clip,"child":"question",
        "text":"Quem guarda a chave da torre, e onde ela fica escondida?"}),
    );
    let retimed = rig.frame("retimed", frame);
    assert_eq!(
        count(&retimed, ANSWER, light),
        0,
        "the answer waits for the longer question"
    );
    assert!(
        count(&retimed, QUESTION, light) > 0,
        "the longer question is still typing"
    );
    assert!(rig.timings("answer").0 > answer.0);
    rig.ok(json!({"op":"history.undo"}));
    assert_eq!(
        rig.frame("undone", frame),
        before,
        "undo restores the picture"
    );

    // A step edited through the contract changes how fast the question types.
    let slow_start = frame_after(rig.timings("question").0, 6);
    let normal = rig.frame("normal", slow_start);
    rig.ok(
        json!({"op":"set","target":"group_child:chat-1/question","field":"step_ms","value":400}),
    );
    let slow = rig.frame("slow", slow_start);
    assert!(
        count(&slow, QUESTION, light) < count(&normal, QUESTION, light),
        "a slower step has typed less"
    );
    rig.ok(json!({"op":"history.undo"}));
    assert_eq!(rig.frame("normal-again", slow_start), normal);

    // Reveal by character instead of by line on the answer: another picture at the first line.
    rig.ok(
        json!({"op":"set","target":"group_child:chat-1/answer","field":"reveal","value":"type"}),
    );
    let typed_answer = rig.frame("typed-answer", frame_after(answer.0, 2));
    assert!(
        count(&typed_answer, ANSWER, light) < count(&before, ANSWER, light),
        "a character is less ink than a whole line"
    );
    rig.ok(json!({"op":"history.undo"}));
    assert_eq!(rig.frame("by-line-again", frame_after(answer.0, 2)), before);
}

#[test]
fn a_reopened_project_draws_the_same_pixels() {
    let Some(mut rig) = rig("reopen") else {
        return;
    };
    let clip = rig.state.timeline.clips[0].id.to_string();
    rig.ok(json!({"op":"clip.group_text","clip":clip,"child":"answer",
        "text":"A resposta cabe em outra linha quando o texto fica mais longo do que o cartão."}));
    let frames = [
        frame_after(rig.timings("question").0, 8),
        frame_after(rig.timings("answer").0, 3),
        frame_after(rig.timings("source").0, 5),
        frame_after(rig.timings("source-mark").1, 4),
    ];
    let before: Vec<Vec<u8>> = frames
        .iter()
        .enumerate()
        .map(|(index, frame)| rig.frame(&format!("a{index}"), *frame))
        .collect();
    let path = rig.dir.join("project/chat.tessel.json");
    rig.ok(json!({"op":"project.save","path":path}));
    let document = rig.state.to_document(None);
    rig.state = EditorState::from_document(&document, None).unwrap();
    for (index, frame) in frames.iter().enumerate() {
        assert_eq!(
            rig.frame(&format!("b{index}"), *frame),
            before[index],
            "frame {frame}"
        );
    }
}

#[test]
fn the_exported_video_carries_the_typing_the_lines_and_the_mark() {
    let Some(mut rig) = rig("export") else {
        return;
    };
    let (question, answer, source, mark) = (
        rig.timings("question"),
        rig.timings("answer"),
        rig.timings("source"),
        rig.timings("source-mark"),
    );
    rig.ok(json!({"op":"project.save","path":rig.dir.join("project/chat.tessel.json")}));

    let config = RuntimeConfig::discover().unwrap();
    let factory: Arc<RendererFactory> = Arc::new(move || Ok(Box::new(session(&config))));
    let mut host = MediaHost::new().with_renderer(factory);
    let target = rig.dir.join("out/chat.mp4");
    let last = frame_after(mark.1, 6);
    let report = headless::execute_ext(
        &mut rig.state,
        None,
        Some(&mut host),
        &json!({"op":"export.start","preset":{"id":"p","name":"P","width":WIDTH,"height":HEIGHT},
            "destination":target,"range":[0, last + 1]}),
    )
    .unwrap_or_else(|error| panic!("export: {error:?}"));
    assert_eq!(report["verification"]["ok"], true, "{report}");

    let frame = |n: i64| {
        decode(
            &rig.tools,
            &target,
            &[
                "-vf",
                &format!("select=eq(n\\,{n}),scale=in_range=tv:out_range=pc"),
                "-frames:v",
                "1",
            ],
        )
    };
    // H.264 smooths edges and shifts a solid colour by a few levels: counts, not exact pixels.
    let waiting = frame(3);
    assert!(count(&waiting, QUESTION, light) < 4, "no question yet");
    let typing = [
        frame(frame_after(question.0, 3)),
        frame(frame_after(question.0, 10)),
        frame(frame_after(question.0, 20)),
    ];
    let ink: Vec<usize> = typing.iter().map(|f| count(f, QUESTION, light)).collect();
    assert!(ink[0] < ink[1] && ink[1] < ink[2] && ink[2] > 40, "{ink:?}");
    assert!(count(&typing[2], ANSWER, light) < 4, "the answer waits");

    let first_line = frame(frame_after(answer.0, 1));
    let both_lines = frame(frame_after(answer.1, 1));
    let (one, two) = (
        count(&first_line, ANSWER, light),
        count(&both_lines, ANSWER, light),
    );
    assert!(
        one > 40 && two > one + one / 3,
        "{one} then {two} pixels of answer"
    );

    let source_typing = frame(frame_after(source.0, 4));
    assert!(
        count(&source_typing, SOURCE, yellow) > 10,
        "the source types in"
    );
    assert!(count(&source_typing, MARK, near(40)) < 4, "no mark yet");
    let marked = frame(last);
    let mark_pixels = count(&marked, MARK, near(40));
    assert!(
        mark_pixels > 120,
        "the mark is in the video: {mark_pixels} pixels"
    );
}

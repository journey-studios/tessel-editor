//! F071: the native player (waveform, progress bar, timer) and the caption track, drawn by the real
//! runtime (Node worker + headless Chrome) from a synthetic voice: four seconds of 440 Hz, a
//! quiet two (amplitude 0.1) and a loud two (0.8). The same project is proven with pixels on the
//! monitor frame (`render.frame`, PNG) and on the exported H.264/AAC video: bars played and idle,
//! the progress fill, the timer, captions on and off, the burn option and its `.srt`, the sound
//! of the file, seek order, an edit with a byte-exact undo and a reopened project. Skipped
//! without the runtime unless `TESSEL_REQUIRE_RUNTIME=1`; FFmpeg reads the pixels back
//! (`docs/media-setup.md`). Everything here is fictitious, synthetic data: the repo is public.
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use serde_json::{Value, json};
use tessel_editor_core::audio::{Pcm, decode_wav, encode_wav_pcm16};
use tessel_editor_core::session::Renderer;
use tessel_editor_core::{EditorState, headless, persist};
use tessel_editor_media::host::{MediaHost, RendererFactory};
use tessel_editor_media::tools::Tools;
use tessel_editor_render::{RuntimeConfig, session};

const WIDTH: u32 = 640;
const HEIGHT: u32 = 360;
const RATE: i64 = 30;
/// Frames of the sequence (5 s); the voice lasts 4 s of them.
const LENGTH: i64 = 150;
const VOICE_FRAMES: i64 = 120;

type Region = (u32, u32, u32, u32);

/// The player on a 640x360 canvas: 60% x 14% wave, a 1.2% bar and the timer, stacked and
/// centred. `(x0, y0, x1, y1)`, end exclusive.
const WAVE: Region = (128, 140, 512, 190);
const BAR_ROW: u32 = 198;
const BAR: Region = (128, 197, 512, 201);
const TIMER: Region = (128, 206, 300, 221);
/// The captions: 38% of the height below the middle.
const CAPTION: Region = (60, 290, 580, 345);

const BACKGROUND: [u8; 3] = [48, 64, 80];
const WHITE: [u8; 3] = [255, 255, 255];
const IDLE: [u8; 3] = [93, 107, 130];
const FILL: [u8; 3] = [74, 144, 226];

struct Rig {
    state: EditorState,
    session: Box<dyn Renderer>,
    tools: Tools,
    dir: PathBuf,
}

/// Four seconds, mono, 48 kHz: the first two at amplitude 0.1 and the last two at 0.8.
fn write_voice(dir: &Path) -> PathBuf {
    let rate = 48_000usize;
    let samples = (0..rate * 4)
        .map(|at| {
            let level = if at / rate < 2 { 0.1 } else { 0.8 };
            level * (at as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin()
        })
        .collect();
    let path = dir.join("voice.wav");
    std::fs::write(
        &path,
        encode_wav_pcm16(&Pcm {
            sample_rate: rate as u32,
            channels: 1,
            samples,
        }),
    )
    .unwrap();
    path
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
    let tools = Tools::discover().expect("FFmpeg is required for the player tests");
    let dir = std::env::temp_dir().join(format!("tessel-player-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let voice = write_voice(&dir);
    let mut rig = Rig {
        state: EditorState::demo(),
        session: Box::new(session(&config)),
        tools,
        dir,
    };
    for request in [
        json!({"op":"project.create","name":"Player","sequence":"main","rate":RATE,
            "width":WIDTH,"height":HEIGHT,"length":LENGTH}),
        json!({"op":"asset.add","kind":"audio","id":"voice","name":"Voice","path":voice}),
        json!({"op":"track.add","id":"sound","name":"Sound","kind":"audio"}),
        json!({"op":"clip.add","id":"voice","track":"sound","asset":"voice","start":0,
            "duration":VOICE_FRAMES}),
        // A flat grey-blue backdrop so the caption box (black at 70%) is visible.
        json!({"op":"track.add","id":"backdrop","name":"Backdrop","kind":"video"}),
        json!({"op":"clip.add","id":"bg","track":"backdrop","start":0,"duration":LENGTH,
            "shape":{"kind":"rect","width":100,"height":100,"fill":"#304050"}}),
        json!({"op":"track.add","id":"top","name":"Top","kind":"video"}),
        json!({"op":"clip.add","id":"pl","track":"top","start":0,"duration":LENGTH,
            "player":{"audio":"voice"}}),
        json!({"op":"track.add","id":"words","name":"Words","kind":"video"}),
        json!({"op":"clip.add","id":"subs","track":"words","start":0,"duration":LENGTH,
            "subtitles":{"cues":[
                {"start_ms":300,"end_ms":1500,"text":"Primeira legenda"},
                {"start_ms":2500,"end_ms":3800,"text":"Segunda legenda mais longa"}]}}),
    ] {
        rig.ok(request);
    }
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

fn near(target: [u8; 3], tolerance: u8) -> impl Fn([u8; 3]) -> bool {
    move |pixel| {
        pixel
            .iter()
            .zip(target)
            .all(|(a, b)| a.abs_diff(b) <= tolerance)
    }
}

/// Pixels of `region` for which `test` holds.
fn count(rgb: &[u8], region: Region, test: impl Fn([u8; 3]) -> bool) -> usize {
    let (x0, y0, x1, y1) = region;
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .filter(|&(x, y)| test(at(rgb, x, y)))
        .count()
}

/// The columns of `region` in which `test` holds at row `y`.
fn columns(rgb: &[u8], (x0, _, x1, _): Region, y: u32, test: impl Fn([u8; 3]) -> bool) -> Vec<u32> {
    (x0..x1).filter(|&x| test(at(rgb, x, y))).collect()
}

/// The tallest run of rows, in any one column of `region`, whose pixels are not the backdrop.
fn tallest_bar(rgb: &[u8], region: Region) -> u32 {
    let (x0, y0, x1, y1) = region;
    (x0..x1)
        .map(|x| {
            (y0..y1)
                .filter(|&y| !near(BACKGROUND, 25)(at(rgb, x, y)))
                .count() as u32
        })
        .max()
        .unwrap_or(0)
}

/// Of the first and the last column of the progress fill at the bar row, how many were filled.
fn filled_width(rgb: &[u8]) -> usize {
    columns(rgb, BAR, BAR_ROW, near(FILL, 30)).len()
}

/// The horizontal extent of the near-white pixels of the captions: `(first, last)` column.
fn caption_extent(rgb: &[u8]) -> Option<(u32, u32)> {
    let (x0, y0, x1, y1) = CAPTION;
    let lit: Vec<u32> = (x0..x1)
        .filter(|&x| (y0..y1).any(|y| near(WHITE, 40)(at(rgb, x, y))))
        .collect();
    Some((*lit.first()?, *lit.last()?))
}

fn caption_ink(rgb: &[u8]) -> usize {
    count(rgb, CAPTION, near(WHITE, 40))
}

/// The dark box behind the text: much darker than the backdrop.
fn box_pixels(rgb: &[u8]) -> usize {
    count(rgb, CAPTION, |pixel| {
        pixel.iter().all(|channel| *channel < 30)
    })
}

#[test]
fn the_player_draws_played_and_idle_bars_the_fill_and_the_timer_on_the_monitor_frame() {
    let Some(mut rig) = rig("monitor") else {
        return;
    };

    // Frame 0: nothing played. The track is there, the fill is not; every bar is idle.
    let start = rig.frame("start", 0);
    assert_eq!(
        at(&start, 5, 5),
        BACKGROUND,
        "the backdrop is under the player"
    );
    assert_eq!(filled_width(&start), 0);
    assert!(
        columns(&start, BAR, BAR_ROW, near([42, 51, 68], 8)).len() > 380,
        "the track"
    );
    assert_eq!(
        count(&start, WAVE, near(WHITE, 20)),
        0,
        "no bar is played yet"
    );
    assert!(count(&start, WAVE, near(IDLE, 12)) > 200, "the idle bars");
    let start_timer = count(&start, TIMER, near(WHITE, 60));
    assert!(start_timer > 20, "the timer is written: {start_timer}");

    // Frame 60 is 2 s of the 4 s voice: half of the 384 px track is filled, the first half of
    // the bars are played and the quiet bars are much lower than the loud ones.
    let half = rig.frame("half", 60);
    let filled = filled_width(&half);
    assert!(
        filled.abs_diff(192) <= 4,
        "half of the track is filled: {filled}"
    );
    let (left, right) = ((128, 140, 320, 190), (320, 140, 512, 190));
    assert!(
        count(&half, left, near(WHITE, 20)) > 20,
        "played bars are white"
    );
    assert_eq!(count(&half, right, near(WHITE, 20)), 0, "the others wait");
    // A white bar's antialiased edge over the backdrop passes through the idle colour, so a few
    // pixels of it remain on the played side; the unplayed side is made of it.
    assert!(
        count(&half, left, near(IDLE, 12)) * 4 < count(&half, right, near(IDLE, 12)),
        "no played bar is idle"
    );
    let (quiet, loud) = (tallest_bar(&half, left), tallest_bar(&half, right));
    assert!(
        quiet <= 12,
        "the quiet half of the voice is low: {quiet} px"
    );
    assert!(loud >= 40, "the loud half of the voice is tall: {loud} px");
    assert_ne!(
        (count(&half, TIMER, near(WHITE, 60)), at(&half, 140, 212)),
        (start_timer, at(&start, 140, 212)),
        "the timer changed with the clock"
    );
    let timer_changed =
        (TIMER.1..TIMER.3).any(|y| (TIMER.0..TIMER.2).any(|x| at(&half, x, y) != at(&start, x, y)));
    assert!(timer_changed, "0:00 and 0:02 are different pictures");

    // Past the voice the player is held at the end, and the picture stops changing.
    let end = rig.frame("end", VOICE_FRAMES - 1);
    // One frame before the end of the voice: 119 of 120 frames, so the fill stops a hair short
    // of the track (progress is time-based and the last frame is not the end instant).
    assert!(
        filled_width(&end) >= 372,
        "the track is almost full: {}",
        filled_width(&end)
    );
    let held = rig.frame("held", LENGTH - 1);
    let later = rig.frame("later", VOICE_FRAMES + 10);
    assert_eq!(held, later, "after the audio nothing moves");
    assert!(filled_width(&held) >= 380);
}

#[test]
fn captions_appear_on_their_cue_centred_on_a_box_and_leave_between_cues() {
    let Some(mut rig) = rig("captions") else {
        return;
    };
    // 30 fps: cue 1 is frames 9..45, cue 2 is frames 75..114.
    let before = rig.frame("before", 5);
    assert_eq!(caption_ink(&before), 0, "no caption before the first cue");
    assert_eq!(box_pixels(&before), 0);

    let first = rig.frame("first", 20);
    let ink = caption_ink(&first);
    assert!(ink > 100, "the first caption is written: {ink}");
    assert!(
        box_pixels(&first) > 1500,
        "on its box: {}",
        box_pixels(&first)
    );
    let (from, to) = caption_extent(&first).expect("text pixels");
    assert!(
        ((from + to) / 2).abs_diff(320) <= 6,
        "a caption is centred on the canvas: {from}..{to}"
    );

    assert_eq!(
        caption_ink(&rig.frame("gap", 45)),
        0,
        "the cue ends at 1.5 s"
    );
    assert_eq!(
        caption_ink(&rig.frame("gap-2", 60)),
        0,
        "nothing between cues"
    );

    let second = rig.frame("second", 80);
    assert!(caption_ink(&second) > ink, "the second caption is longer");
    assert_ne!(first, second);
    let (from_2, to_2) = caption_extent(&second).expect("text pixels");
    assert!(to_2 - from_2 > to - from, "a longer text is wider");
    assert!(
        ((from_2 + to_2) / 2).abs_diff(320) <= 6,
        "and still centred"
    );
    assert_eq!(
        caption_ink(&rig.frame("after", 115)),
        0,
        "the last cue ended"
    );
}

#[test]
fn a_seek_in_any_order_draws_the_same_picture() {
    let Some(mut rig) = rig("seek") else {
        return;
    };
    let frames = [20, 60, 80, 100, 130];
    let forward: Vec<Vec<u8>> = frames
        .iter()
        .map(|frame| rig.frame(&format!("fwd{frame}"), *frame))
        .collect();
    // Backwards, then a jump: the player and the captions read only the frame they draw.
    for (index, frame) in frames.iter().enumerate().rev() {
        assert_eq!(
            rig.frame(&format!("back{frame}"), *frame),
            forward[index],
            "frame {frame}"
        );
    }
    // A freshly opened session draws a frame it never saw coming exactly the same.
    let path = rig.dir.join("project/player.tessel.json");
    rig.ok(json!({"op":"project.save","path":path}));
    rig.state = persist::load(&path).unwrap();
    rig.session.reset_project();
    assert_eq!(
        rig.frame("fresh", 80),
        forward[2],
        "a reopened project, a cold seek"
    );
    assert_eq!(rig.frame("fresh-0", 60), forward[1]);
}

#[test]
fn an_edit_changes_the_picture_undo_restores_it_byte_for_byte_and_reopening_keeps_it() {
    let Some(mut rig) = rig("edit") else {
        return;
    };
    let frames = [0, 20, 60, 80];
    let before: Vec<Vec<u8>> = frames
        .iter()
        .map(|frame| rig.frame(&format!("a{frame}"), *frame))
        .collect();

    rig.ok(json!({"op":"set","target":"clip_player:pl","field":"fill_color","value":"#e24a4a"}));
    rig.ok(json!({"op":"set","target":"clip_subtitles:subs","field":"color","value":"#ffd24a"}));
    let edited = rig.frame("edited", 60);
    assert_eq!(
        count(&edited, BAR, near(FILL, 30)),
        0,
        "the old fill colour is gone"
    );
    assert!(
        count(&edited, BAR, near([226, 74, 74], 30)) > 400,
        "the new fill colour"
    );
    let caption = rig.frame("edited-caption", 20);
    assert_eq!(caption_ink(&caption), 0, "white text is no more");
    assert!(
        count(&caption, CAPTION, near([255, 210, 74], 40)) > 100,
        "the new text colour"
    );

    // Two undos, two entries; the pictures are the first ones exactly, and two redos bring the
    // edited ones back exactly.
    rig.ok(json!({"op":"history.undo"}));
    rig.ok(json!({"op":"history.undo"}));
    for (index, frame) in frames.iter().enumerate() {
        assert_eq!(
            rig.frame(&format!("b{frame}"), *frame),
            before[index],
            "frame {frame} after undo"
        );
    }
    rig.ok(json!({"op":"history.redo"}));
    rig.ok(json!({"op":"history.redo"}));
    assert_eq!(rig.frame("redone", 60), edited);
    assert_eq!(rig.frame("redone-caption", 20), caption);

    // Both edits are on disk and back after a reopen (a new session: no history, same pixels).
    let path = rig.dir.join("project/player.tessel.json");
    rig.ok(json!({"op":"project.save","path":path}));
    rig.state = persist::load(&path).unwrap();
    rig.session.reset_project();
    assert_eq!(rig.frame("reopened", 60), edited);
    assert_eq!(rig.frame("reopened-caption", 20), caption);
}

/// Reads output frame `n` of `file` back through an explicit BT.709 limited-range conversion.
fn video_frame(tools: &Tools, file: &Path, n: i64) -> Vec<u8> {
    decode(
        tools,
        file,
        &[
            "-vf",
            &format!("select=eq(n\\,{n}),scale=in_color_matrix=bt709:in_range=tv:out_range=pc"),
            "-frames:v",
            "1",
        ],
    )
}

fn export(rig: &mut Rig, name: &str) -> (Value, PathBuf) {
    let config = RuntimeConfig::discover().unwrap();
    let factory: Arc<RendererFactory> = Arc::new(move || Ok(Box::new(session(&config))));
    let mut host = MediaHost::new().with_renderer(factory);
    let target = rig.dir.join(format!("out/{name}.mp4"));
    let report = headless::execute_ext(
        &mut rig.state,
        None,
        Some(&mut host),
        &json!({"op":"export.start","preset":{"id":"p","name":"P","width":WIDTH,"height":HEIGHT},
            "destination":target}),
    )
    .unwrap_or_else(|error| panic!("export: {error:?}"));
    assert_eq!(report["verification"]["ok"], true, "{report}");
    (report, target)
}

fn rms(pcm: &Pcm, seconds: (f64, f64)) -> f64 {
    let rate = f64::from(pcm.sample_rate);
    let (from, to) = ((seconds.0 * rate) as usize, (seconds.1 * rate) as usize);
    let sum: f64 = (from..to)
        .map(|at| f64::from(pcm.sample(at, 0)).powi(2))
        .sum();
    (sum / (to - from) as f64).sqrt()
}

#[test]
fn the_exported_video_carries_the_player_the_burned_captions_and_the_voice() {
    let Some(mut rig) = rig("export") else {
        return;
    };
    rig.ok(json!({"op":"project.save","path":rig.dir.join("project/player.tessel.json")}));
    let (report, target) = export(&mut rig, "burned");
    assert_eq!(report["frames"], LENGTH);
    assert!(
        report["captions"].as_array().unwrap().is_empty(),
        "burned captions write no file"
    );
    assert!(!target.with_extension("srt").exists());

    // The video, frame by frame, is the monitor's picture (H.264 moves a colour a few levels, so
    // these are counts and positions, not bytes).
    let start = video_frame(&rig.tools, &target, 0);
    assert_eq!(filled_width(&start), 0);
    assert!(
        count(&start, WAVE, near(IDLE, 25)) > 150,
        "the idle bars are in the video"
    );
    let half = video_frame(&rig.tools, &target, 60);
    let filled = filled_width(&half);
    assert!(
        filled.abs_diff(192) <= 6,
        "half of the track is filled: {filled}"
    );
    assert!(count(&half, (128, 140, 320, 190), near(WHITE, 40)) > 20);
    assert_eq!(count(&half, (320, 140, 512, 190), near(WHITE, 40)), 0);
    let (quiet, loud) = (
        tallest_bar(&half, (128, 140, 320, 190)),
        tallest_bar(&half, (320, 140, 512, 190)),
    );
    assert!(
        quiet <= 14 && loud >= 38,
        "the waveform: {quiet} px then {loud} px"
    );
    let end = video_frame(&rig.tools, &target, LENGTH - 1);
    assert!(
        filled_width(&end) >= 378,
        "held at the end: {}",
        filled_width(&end)
    );

    let first = video_frame(&rig.tools, &target, 20);
    assert!(
        caption_ink(&first) > 60,
        "the caption is burned into the video"
    );
    let (from, to) = caption_extent(&first).unwrap();
    assert!(
        ((from + to) / 2).abs_diff(320) <= 8,
        "centred: {from}..{to}"
    );
    assert_eq!(caption_ink(&video_frame(&rig.tools, &target, 60)), 0);
    assert!(caption_ink(&video_frame(&rig.tools, &target, 80)) > 60);

    // The sound of the file is the real voice: the loud half is much louder than the quiet half.
    let wav = rig.dir.join("decoded.wav");
    let status = Command::new(&rig.tools.ffmpeg)
        .args(["-v", "error", "-nostdin", "-y", "-i"])
        .arg(&target)
        .args(["-vn", "-c:a", "pcm_s16le"])
        .arg(&wav)
        .status()
        .unwrap();
    assert!(status.success());
    let pcm = decode_wav(&std::fs::read(&wav).unwrap()).unwrap();
    let (quiet_rms, loud_rms) = (rms(&pcm, (0.3, 1.7)), rms(&pcm, (2.3, 3.7)));
    assert!(quiet_rms > 0.03, "the quiet half is audible: {quiet_rms}");
    assert!(loud_rms > quiet_rms * 5.0, "{quiet_rms} then {loud_rms}");
}

#[test]
fn captions_set_not_to_burn_leave_the_picture_and_come_out_as_an_srt() {
    let Some(mut rig) = rig("sidecar") else {
        return;
    };
    rig.ok(json!({"op":"project.save","path":rig.dir.join("project/player.tessel.json")}));
    rig.ok(json!({"op":"set","target":"clip_subtitles:subs","field":"burn","value":"false"}));
    // The monitor still shows them: the option is about the export only.
    assert!(caption_ink(&rig.frame("monitor", 20)) > 100);

    let (report, target) = export(&mut rig, "clean");
    for frame in [20, 80] {
        let picture = video_frame(&rig.tools, &target, frame);
        assert_eq!(
            caption_ink(&picture),
            0,
            "no caption is burned at frame {frame}"
        );
        assert_eq!(box_pixels(&picture), 0, "and no box");
    }
    // The player is untouched by the option.
    assert!(filled_width(&video_frame(&rig.tools, &target, 60)).abs_diff(192) <= 6);

    let captions = report["captions"].as_array().unwrap();
    assert_eq!(captions.len(), 1, "{report}");
    assert_eq!(captions[0]["cues"], 2);
    let srt = std::fs::read_to_string(target.with_extension("srt")).unwrap();
    assert_eq!(
        srt,
        "1\n00:00:00,300 --> 00:00:01,500\nPrimeira legenda\n\n\
         2\n00:00:02,500 --> 00:00:03,800\nSegunda legenda mais longa\n"
    );
    assert_eq!(
        report["captions"][0]["path"].as_str().map(PathBuf::from),
        Some(target.with_extension("srt"))
    );

    // The project is exactly what it was: the export drew from a snapshot.
    assert!(!rig.state.subtitles_of("subs").unwrap().1.burn);
    assert_eq!(rig.state.subtitles_of("subs").unwrap().1.cues.len(), 2);
}

use super::*;

// ---- negative controls and the legacy model ----------------------------------------------------------

#[test]
fn the_oracle_rejects_non_finite_pcm_in_either_channel() {
    let rate = (24, 1);
    let main = Spec::audio(rate, 48_000, true, 0, 1, Frac::int(0));
    let mut state = audio_project(rate, 48_000, true, &["a1"]);
    add_audio(&mut state, "main", "a1", &main, None);
    let pcm = check(&state, 0, 1, &[("main", main)], "finite control");
    for channel in [0, 1] {
        for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut invalid_pcm = pcm.clone();
            invalid_pcm.samples[channel] = invalid;
            assert_eq!(worst_error(&invalid_pcm, &[&main], 0, 1), f64::INFINITY);
        }
    }
}

#[test]
fn a_plan_that_loses_the_phase_fails_the_oracle() {
    let rate = (30_000, 1001);
    let main = Spec::audio(rate, 44_100, false, 3, 24, Frac::new(1, 4));
    let mut state = audio_project(rate, 44_100, false, &["a1"]);
    add_audio(&mut state, "main", "a1", &main, Some(-100));
    split(&mut state, "main", 14);
    let halves = [
        ("main", main.piece(3, 14)),
        ("main-split-1", main.piece(14, 27)),
    ];
    check(&state, 0, LENGTH, &halves, "control");
    let all: Vec<&Spec> = halves.iter().map(|(_, spec)| spec).collect();
    for variant in ["floor", "round"] {
        let mut plan = state.plan_audio(0, LENGTH).unwrap();
        for segment in &mut plan.segments {
            if variant == "round" && segment.source_phase.num() * 2 >= segment.source_phase.den() {
                segment.source_start += 1;
            }
            segment.source_phase = Ratio::ZERO;
        }
        let pcm = OfflineMixer::default().mix(&plan).unwrap();
        let worst = worst_error(&pcm, &all, 0, LENGTH);
        assert!(
            worst > 1e-6,
            "{variant}: a lost phase must be seen, got {worst}"
        );
    }
}

/// The mixer before BL081.5, as a function of the oracle's own numbers: the segment starts on the
/// source sample nearest to the frame boundary's time and then advances by a float step.
fn legacy_left(spec: &Spec, from: i64, frames: i64) -> Vec<f32> {
    let step = spec.sr as f64 / 48_000.0 * (spec.speed as f64 / 100.0);
    let first = spec.boundary(spec.start.max(from));
    let last = spec.boundary(spec.end().min(from + frames));
    // Source time of the segment's first FRAME (not output sample), rounded to a whole sample.
    let advance = Frac::new(
        i128::from(spec.start.max(from) - spec.start) * spec.rate.1,
        spec.rate.0,
    )
    .mul(Frac::new(spec.speed, 100));
    let exact = spec
        .ss
        .add(advance)
        .sub(spec.offset)
        .mul(Frac::int(spec.sr));
    let start = (exact.n * 2 + exact.d) / (exact.d * 2);
    let value = |k: i128| (k as f32) * (SCALE as f32);
    (0..(last - first))
        .map(|index| {
            let position = start as f64 + index as f64 * step;
            let base = position.floor() as i128;
            let fraction = (position - position.floor()) as f32;
            let (a, b) = (value(base), value(base + 1));
            a + (b - a) * fraction
        })
        .collect()
}
fn left_of(pcm: &Pcm) -> Vec<f32> {
    (0..pcm.frames())
        .map(|frame| pcm.sample(frame, 0))
        .collect()
}

#[test]
fn integer_grids_keep_the_legacy_pcm_bit_for_bit() {
    // Whole source samples per frame and exact float steps: the old rounding lost nothing.
    for rate in [(24, 1), (25, 1), (30, 1), (60, 1)] {
        let main = Spec::audio(rate, 48_000, false, 3, 24, Frac::new(1, 4));
        let mut state = audio_project(rate, 48_000, false, &["a1"]);
        add_audio(&mut state, "main", "a1", &main, Some(-100));
        let new = mix(&state);
        let legacy = legacy_left(&main, 0, LENGTH);
        let new_left = left_of(&new);
        let first = main.boundary(3) as usize;
        assert!(
            legacy
                .iter()
                .enumerate()
                .all(|(i, value)| new_left[first + i].to_bits() == value.to_bits()),
            "{rate:?}: the PCM of an integer grid moved"
        );
        assert!(new_left[..first].iter().all(|sample| *sample == 0.0));
    }
    // A video with sound at speeds whose float steps are exact (0.5, 1.5, 2).
    for speed in [50, 150, 200] {
        let rate = (24, 1);
        let mut main = Spec::audio(rate, 48_000, false, 6, 24, Frac::new(1, 4));
        (main.speed, main.audio_clip) = (speed, false);
        let mut state = video_project(rate, &[(48_000, false, Ratio::ZERO)]);
        add_video(&mut state, "main", &main, 0, -100);
        let (new, legacy) = (left_of(&mix(&state)), legacy_left(&main, 0, LENGTH));
        let first = main.boundary(6) as usize;
        assert!(
            legacy
                .iter()
                .enumerate()
                .all(|(i, value)| new[first + i].to_bits() == value.to_bits()),
            "speed {speed}: the PCM of an integer grid moved"
        );
    }
}

#[test]
fn the_old_halves_disagreed_on_a_fractional_grid_and_the_new_ones_do_not() {
    // The reported case: 30000/1001, audio moved to frame 1 and cut at frame 2.
    let rate = (30_000, 1001);
    let main = Spec::audio(rate, 48_000, false, 1, 47, Frac::int(0));
    let (left, right) = (main.piece(1, 2), main.piece(2, 48));
    let mut state = audio_project(rate, 48_000, false, &["a1"]);
    add_audio(&mut state, "main", "a1", &main, Some(-100));
    let whole = mix(&state);
    split(&mut state, "main", 2);
    let halves = mix(&state);

    // Before: each half restarted its phase on a rounded source sample.
    let old_whole = legacy_left(&main, 0, LENGTH);
    let mut old_halves = legacy_left(&left, 0, LENGTH);
    let tail = legacy_left(&right, 0, LENGTH);
    let junction = (main.boundary(2) - main.boundary(1)) as usize;
    old_halves.truncate(junction);
    old_halves.extend(tail);
    let old_split: f32 = old_whole
        .iter()
        .zip(&old_halves)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f32::max);
    // The new model: no difference at all, and the oracle agrees with both.
    let new_split = whole
        .samples
        .iter()
        .zip(&halves.samples)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f32::max);
    let first = main.boundary(1) as usize;
    let old_vs_oracle = {
        let mut worst = 0.0f64;
        for (i, value) in old_whole.iter().enumerate() {
            let n = first as i128 + i as i128;
            worst = worst.max((f64::from(*value) - main.position(n).f64() * SCALE).abs());
        }
        worst
    };
    eprintln!(
        "BL081.5 before/after (30000/1001, 48 kHz ramp, clip at 1 cut at 2): \
         old halves vs old whole = {old_split:e}, old whole vs oracle = {old_vs_oracle:e}, \
         new halves vs new whole = {new_split:e}"
    );
    assert!(
        old_split > 1e-6,
        "the legacy model must reproduce the reported shift"
    );
    assert!(
        old_vs_oracle > 1e-6,
        "the legacy model was off the true position"
    );
    assert_eq!(new_split, 0.0);
    check(
        &state,
        0,
        LENGTH,
        &[("main", left), ("main-split-1", right)],
        "new halves",
    );
}

// ---- the same cut through the windowless process ------------------------------------------------------

#[test]
fn the_headless_process_cuts_the_case_that_was_refused_and_the_wav_is_identical() {
    use std::io::Write;
    let dir = shared_dir().join("process");
    fs::create_dir_all(&dir).unwrap();
    let (whole_wav, cut_wav) = (dir.join("whole.wav"), dir.join("cut.wav"));
    let lines = [
        json!({"op":"project.create","name":"P","sequence":"main","rate":rate_json((30_000, 1001)),
            "width":64,"height":36,"length":LENGTH}),
        json!({"op":"asset.add","kind":"audio","id":"ramp","name":"Ramp","path":ramp(44_100, false)}),
        json!({"op":"track.add","id":"a1","name":"A1","kind":"audio"}),
        json!({"op":"clip.add","id":"main","track":"a1","asset":"ramp","start":1,"duration":47,
            "properties":{"pan":-100}}),
        json!({"op":"render.audio","out":whole_wav}),
        json!({"op":"clip.split","clip":"main","frame":2}),
        json!({"op":"render.audio","out":cut_wav}),
        json!({"op":"history.undo"}),
        json!({"op":"clip.split","clip":"main","frame":5,"new_id":"tail"}),
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
    assert_eq!(output.status.code(), Some(0), "every line succeeds");
    let replies: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(replies.len(), lines.len());
    assert!(
        replies.iter().all(|reply| reply["ok"] == true),
        "{replies:?}"
    );
    assert_eq!(replies[5]["result"]["action"], "split");
    assert_eq!(
        fs::read(&whole_wav).unwrap(),
        fs::read(&cut_wav).unwrap(),
        "the WAV rendered after the cut is the one rendered before it"
    );
    assert_eq!(replies[6]["result"]["segments"], 2);
}

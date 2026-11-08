//! Rules of the rolling history, with a fake clock: every call states the time it happens at.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;

use super::*;

fn at(milliseconds: u64) -> Duration {
    Duration::from_millis(milliseconds)
}

fn started(milliseconds: u64) -> Recorder {
    let mut recorder = Recorder::default();
    assert!(recorder.start(at(milliseconds), false));
    recorder
}

fn playing(milliseconds: u64) -> Recorder {
    let mut recorder = Recorder::default();
    assert!(recorder.start(at(milliseconds), true));
    recorder
}

fn image(request: u64, render_ms: f64) -> Image {
    Image {
        request: Some(request),
        frame: None,
        render_ms: Some(render_ms),
        texture_bytes: None,
    }
}

fn agg(snapshot: &Snapshot, metric: Metric, bucket: usize) -> Agg {
    snapshot.buckets[bucket][metric.index()]
}

fn last(_: &Snapshot) -> usize {
    BUCKETS - 1
}

fn scratch(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("tessel-perf-history-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

// ------------------------------------------------------------------ window and buckets

#[test]
fn the_window_is_sixty_seconds_of_two_hundred_and_forty_buckets_of_250_ms() {
    assert_eq!(WINDOW, Duration::from_secs(60));
    assert_eq!(BUCKET, Duration::from_millis(250));
    assert_eq!(BUCKETS, 240);
    assert_eq!(WINDOW.as_millis() / BUCKET.as_millis(), BUCKETS as u128);
    let recorder = started(0);
    let snapshot = recorder.snapshot(at(120_000)).unwrap();
    assert_eq!(snapshot.buckets.len(), 240);
    assert_eq!(
        snapshot.window_end - snapshot.window_start,
        WINDOW.as_secs_f64() * 1000.0
    );
}

#[test]
fn a_bucket_keeps_count_minimum_sum_and_maximum_so_a_spike_survives() {
    let mut recorder = started(0);
    for (milliseconds, value) in [(10, 16.0), (60, 17.0), (110, 250.0), (160, 16.5)] {
        recorder.sample(Metric::RenderMs, at(milliseconds), value);
    }
    let snapshot = recorder.snapshot(at(200)).unwrap();
    let bucket = agg(&snapshot, Metric::RenderMs, last(&snapshot));
    assert_eq!(bucket.count, 4);
    assert_eq!(bucket.min, 16.0);
    assert_eq!(bucket.max, 250.0);
    assert_eq!(bucket.sum, 299.5);
    assert_eq!(bucket.mean(), Some(299.5 / 4.0));
    assert_eq!(bucket.max_at_us, 110_000, "the peak says when it happened");
}

#[test]
fn the_summary_mean_is_weighted_by_count_not_a_mean_of_means() {
    let mut recorder = started(0);
    recorder.sample(Metric::RenderMs, at(100), 10.0);
    for milliseconds in [300, 320, 340] {
        recorder.sample(Metric::RenderMs, at(milliseconds), 20.0);
    }
    let snapshot = recorder.snapshot(at(400)).unwrap();
    let summary = snapshot.summary(Metric::RenderMs).unwrap();
    assert_eq!(summary.count, 4);
    assert_eq!(summary.mean, 17.5, "70 / 4, not (10 + 20) / 2");
    assert_eq!((summary.min, summary.max), (10.0, 20.0));
}

#[test]
fn a_bucket_with_no_sample_is_absent_not_zero_and_not_the_last_value() {
    let mut recorder = started(0);
    recorder.sample(Metric::RenderMs, at(100), 12.0);
    recorder.sample(Metric::RenderMs, at(1100), 14.0);
    let snapshot = recorder.snapshot(at(1200)).unwrap();
    let series: Vec<Option<&Agg>> = snapshot.series(Metric::RenderMs).collect();
    let bucket = |milliseconds: u64| snapshot.index_at(at(milliseconds)).unwrap();
    assert!(series[bucket(100)].is_some());
    assert!(series[bucket(600)].is_none(), "a gap is None");
    assert!(series[bucket(1100)].is_some());
    let document = snapshot.to_document();
    assert_eq!(
        document["series"]["render_ms"]["count"][bucket(600)],
        Value::Null
    );
    assert_eq!(
        document["series"]["render_ms"]["max"][bucket(600)],
        Value::Null
    );
    assert_eq!(
        document["series"]["latency_ms"]["count"][bucket(100)],
        Value::Null
    );
    assert!(snapshot.summary(Metric::LatencyMs).is_none());
}

#[test]
fn old_data_leaves_by_elapsed_time_even_when_no_new_frame_arrives() {
    let mut recorder = started(0);
    recorder.sample(Metric::RenderMs, at(1_000), 30.0);
    recorder.event(at(1_000), EventKind::ResourceLoadStarted, None);
    let early = recorder.snapshot(at(30_000)).unwrap();
    assert_eq!(early.summary(Metric::RenderMs).unwrap().count, 1);
    assert!(
        early
            .events
            .iter()
            .any(|e| e.kind == EventKind::ResourceLoadStarted)
    );
    // 70 s later, nothing was recorded since: the window moved on its own.
    let late = recorder.snapshot(at(71_000)).unwrap();
    assert!(late.summary(Metric::RenderMs).is_none());
    assert!(
        late.events
            .iter()
            .all(|e| e.kind != EventKind::ResourceLoadStarted)
    );
    // The edge: still inside at 60 s, gone one bucket later.
    let edge = recorder.snapshot(at(60_900)).unwrap();
    assert!(edge.summary(Metric::RenderMs).is_some());
    assert!(
        recorder
            .snapshot(at(61_300))
            .unwrap()
            .summary(Metric::RenderMs)
            .is_none()
    );
}

#[test]
fn a_ring_slot_reused_after_sixty_seconds_holds_only_the_new_bucket() {
    let mut recorder = started(0);
    recorder.sample(Metric::RenderMs, at(100), 99.0);
    recorder.sample(Metric::RenderMs, at(60_100), 5.0);
    let snapshot = recorder.snapshot(at(60_200)).unwrap();
    let summary = snapshot.summary(Metric::RenderMs).unwrap();
    assert_eq!((summary.count, summary.max), (1, 5.0));
}

#[test]
fn a_sample_older_than_the_window_is_dropped_and_counted() {
    let mut recorder = started(0);
    recorder.sample(Metric::RenderMs, at(100_000), 1.0);
    recorder.sample(Metric::RenderMs, at(10_000), 2.0);
    assert_eq!(recorder.counters().late_samples, 1);
    let snapshot = recorder.snapshot(at(100_100)).unwrap();
    assert_eq!(snapshot.summary(Metric::RenderMs).unwrap().count, 1);
}

#[test]
fn a_young_recording_marks_what_it_never_observed() {
    let recorder = started(10_000);
    let snapshot = recorder.snapshot(at(12_000)).unwrap();
    assert!(!snapshot.observed(0), "before the recording began");
    assert!(snapshot.observed(snapshot.index_at(at(11_000)).unwrap()));
    assert!(snapshot.observed(last(&snapshot)));
    assert_eq!(snapshot.recording_end, None);
    let document = snapshot.to_document();
    assert_eq!(document["recording"]["state"], "recording");
    assert_eq!(document["window"]["unobserved_before_ms"], 10_000.0);
}

// ------------------------------------------------------------------ intervals: rest versus stall

#[test]
fn a_rest_records_no_interval_and_a_stall_while_playing_does() {
    let mut rest = started(0);
    rest.monitor_presented(at(100));
    rest.monitor_presented(at(5_100));
    let snapshot = rest.snapshot(at(5_200)).unwrap();
    assert_eq!(snapshot.summary(Metric::MonitorFrames).unwrap().count, 2);
    assert!(
        snapshot.summary(Metric::MonitorIntervalMs).is_none(),
        "a still picture for 5 s is a rest, not a 5000 ms interval"
    );

    let mut playback = playing(0);
    playback.monitor_presented(at(40));
    playback.monitor_presented(at(80));
    playback.monitor_presented(at(5_080));
    let summary = playback
        .snapshot(at(5_100))
        .unwrap()
        .summary(Metric::MonitorIntervalMs)
        .unwrap();
    assert_eq!(summary.max, 5_000.0, "playing: the silence was a stall");
    assert_eq!(summary.count, 3);
}

#[test]
fn a_pause_is_not_an_interval_and_resuming_starts_from_the_resume() {
    let mut recorder = playing(0);
    recorder.monitor_presented(at(40));
    recorder.playback_stopped(at(100));
    recorder.playback_started(at(10_000), 50);
    recorder.monitor_presented(at(10_040));
    let snapshot = recorder.snapshot(at(10_100)).unwrap();
    let summary = snapshot.summary(Metric::MonitorIntervalMs).unwrap();
    assert_eq!(summary.max, 40.0, "no 10 s interval across the pause");
    assert_eq!(summary.count, 2);
}

#[test]
fn silence_while_playing_shows_as_an_open_interval_before_any_frame_arrives() {
    let mut recorder = playing(0);
    recorder.monitor_presented(at(40));
    let snapshot = recorder.snapshot(at(2_040)).unwrap();
    assert!(snapshot.expected_motion);
    assert_eq!(snapshot.open_monitor_interval_ms, Some(2_000.0));
    let mut resting = started(0);
    resting.monitor_presented(at(40));
    assert_eq!(
        resting
            .snapshot(at(9_000))
            .unwrap()
            .open_monitor_interval_ms,
        None
    );
}

#[test]
fn the_interface_records_only_intervals_inside_the_rest_threshold() {
    let mut recorder = started(0);
    for milliseconds in [0, 16, 33, 5_000, 5_016] {
        recorder.ui_painted(at(milliseconds));
    }
    let snapshot = recorder.snapshot(at(5_100)).unwrap();
    assert_eq!(snapshot.summary(Metric::UiFrames).unwrap().count, 5);
    let intervals = snapshot.summary(Metric::UiIntervalMs).unwrap();
    assert_eq!(intervals.count, 3, "16, 17 and 16; the 4.9 s gap is a rest");
    assert_eq!(intervals.max, 17.0);
}

// ------------------------------------------------------------------ losses are deltas

#[test]
fn losses_are_the_delta_of_a_cumulative_counter_per_run_never_added_twice() {
    let mut recorder = playing(0);
    recorder.loss_run_started(1);
    for (milliseconds, cumulative) in [(40, 0), (80, 2), (120, 2), (160, 5)] {
        recorder.loss_observed(at(milliseconds), 1, cumulative);
    }
    let total = |recorder: &Recorder| {
        recorder
            .snapshot(at(300))
            .unwrap()
            .buckets
            .iter()
            .map(|aggs| aggs[Metric::LostFrames.index()].sum)
            .sum::<f64>()
    };
    assert_eq!(total(&recorder), 5.0, "0 + 2 + 0 + 3, not 0 + 2 + 2 + 5");
    // A seek restarts the comparison: the new run starts from zero, the old total is not added again.
    recorder.loss_run_started(2);
    recorder.loss_observed(at(200), 2, 0);
    recorder.loss_observed(at(240), 2, 1);
    assert_eq!(total(&recorder), 6.0);
    let summary = recorder
        .snapshot(at(300))
        .unwrap()
        .summary(Metric::LostFrames)
        .unwrap();
    assert_eq!(summary.count, 6, "zero losses are observed zeros, not gaps");
}

#[test]
fn clearing_mid_run_rebases_the_losses_instead_of_counting_the_old_total() {
    let mut recorder = playing(0);
    recorder.loss_run_started(1);
    recorder.loss_observed(at(40), 1, 5);
    recorder.clear(at(100), true);
    recorder.loss_observed(at(140), 1, 7);
    recorder.loss_observed(at(180), 1, 8);
    let snapshot = recorder.snapshot(at(200)).unwrap();
    let sum: f64 = snapshot
        .buckets
        .iter()
        .map(|aggs| aggs[Metric::LostFrames.index()].sum)
        .sum();
    assert_eq!(
        sum, 1.0,
        "7 is the new baseline, only the step to 8 is a loss"
    );
}

#[test]
fn nothing_is_lost_outside_playback() {
    let mut recorder = started(0);
    recorder.monitor_presented(at(40));
    let snapshot = recorder.snapshot(at(100)).unwrap();
    assert!(snapshot.summary(Metric::LostFrames).is_none());
}

// ------------------------------------------------------------------ latency and render_ms

#[test]
fn latency_is_admission_to_the_accepted_picture_of_the_same_request_and_render_ms_stays_apart() {
    let mut recorder = started(0);
    recorder.request_admitted(7, at(1_000));
    recorder.image(at(1_040), &image(7, 5.0));
    let snapshot = recorder.snapshot(at(1_100)).unwrap();
    let latency = snapshot.summary(Metric::LatencyMs).unwrap();
    let render = snapshot.summary(Metric::RenderMs).unwrap();
    assert_eq!((latency.count, latency.min, latency.max), (1, 40.0, 40.0));
    assert_eq!((render.count, render.max), (1, 5.0));
    assert_eq!(recorder.pending_requests(), 0);
}

#[test]
fn a_picture_of_a_request_nobody_admitted_invents_no_latency() {
    let mut recorder = started(0);
    recorder.image(at(1_040), &image(99, 5.0));
    let snapshot = recorder.snapshot(at(1_100)).unwrap();
    assert!(snapshot.summary(Metric::LatencyMs).is_none());
    assert_eq!(snapshot.summary(Metric::RenderMs).unwrap().count, 1);
    assert_eq!(recorder.counters().unmatched_images, 1);
    let mut without_id = started(0);
    without_id.image(at(10), &Image::default());
    assert!(
        without_id
            .snapshot(at(20))
            .unwrap()
            .summary(Metric::LatencyMs)
            .is_none()
    );
}

#[test]
fn superseded_requests_are_forgotten_when_a_newer_one_is_answered() {
    let mut recorder = started(0);
    recorder.request_admitted(1, at(100));
    recorder.request_admitted(2, at(120));
    recorder.request_admitted(3, at(140));
    recorder.image(at(200), &image(3, 4.0));
    assert_eq!(
        recorder.pending_requests(),
        0,
        "1 and 2 were superseded and never answered"
    );
    recorder.image(at(210), &image(2, 4.0));
    let snapshot = recorder.snapshot(at(300)).unwrap();
    assert_eq!(snapshot.summary(Metric::LatencyMs).unwrap().count, 1);
    assert_eq!(snapshot.summary(Metric::LatencyMs).unwrap().max, 60.0);
}

#[test]
fn a_stale_response_is_dropped_counted_and_never_becomes_a_latency() {
    let mut recorder = started(0);
    recorder.request_admitted(4, at(100));
    recorder.response_stale(4);
    recorder.image(at(500), &image(4, 3.0));
    let snapshot = recorder.snapshot(at(600)).unwrap();
    assert!(snapshot.summary(Metric::LatencyMs).is_none());
    assert_eq!(snapshot.counters.stale_responses, 1);
    assert_eq!(
        snapshot.to_document()["counters"]["stale_responses_dropped"],
        1
    );
}

#[test]
fn pending_requests_are_bounded_and_the_forgotten_ones_are_counted() {
    let mut recorder = started(0);
    for id in 0..(MAX_PENDING as u64 + 10) {
        recorder.request_admitted(id, at(id));
    }
    assert_eq!(recorder.pending_requests(), MAX_PENDING);
    assert_eq!(recorder.counters().forgotten_requests, 10);
    recorder.image(at(500), &image(0, 1.0));
    assert!(
        recorder
            .snapshot(at(600))
            .unwrap()
            .summary(Metric::LatencyMs)
            .is_none()
    );
}

#[test]
fn a_failed_request_is_an_event_with_its_code_and_is_never_answered() {
    let mut recorder = started(0);
    recorder.request_admitted(8, at(100));
    recorder.request_failed(at(150), Some(8), "render_failed");
    assert_eq!(recorder.pending_requests(), 0);
    let snapshot = recorder.snapshot(at(200)).unwrap();
    let failed = snapshot
        .events
        .iter()
        .find(|e| e.kind == EventKind::RequestFailed)
        .unwrap();
    assert_eq!(failed.detail.as_deref(), Some("render_failed"));
    assert_eq!(failed.request, Some(8));
}

// ------------------------------------------------------------------ memory gauges

#[test]
fn memory_is_read_at_most_once_per_second_and_unknown_stays_absent() {
    let mut recorder = started(0);
    assert!(recorder.rss_due(at(0)));
    recorder.rss_read(at(0), Some(100));
    assert!(!recorder.rss_due(at(999)));
    assert!(recorder.rss_due(at(1_000)));
    recorder.rss_read(at(1_000), None);
    assert!(
        !recorder.rss_due(at(1_500)),
        "a failed read also waits a second"
    );
    recorder.rss_read(at(2_000), Some(300));
    let snapshot = recorder.snapshot(at(2_100)).unwrap();
    let series: Vec<Option<&Agg>> = snapshot.series(Metric::RssBytes).collect();
    let bucket = |milliseconds: u64| snapshot.index_at(at(milliseconds)).unwrap();
    assert_eq!(series[bucket(0)].unwrap().max, 100.0);
    assert!(
        series[bucket(1_000)].is_none(),
        "the failed read is a gap, not the last value"
    );
    assert_eq!(series[bucket(2_000)].unwrap().max, 300.0);
    assert_eq!(snapshot.counters.memory_unavailable, 1);
}

#[test]
fn the_texture_gauge_follows_the_pictures_at_one_hertz_and_absent_stays_absent() {
    let mut recorder = started(0);
    for step in 0..48 {
        let picture = Image {
            texture_bytes: Some(1_000 + step),
            ..image(step, 2.0)
        };
        recorder.image(at(step * 41), &picture);
    }
    let snapshot = recorder.snapshot(at(2_000)).unwrap();
    assert_eq!(
        snapshot.summary(Metric::TextureBytes).unwrap().count,
        2,
        "48 pictures, 2 s"
    );
    assert_eq!(snapshot.summary(Metric::RenderMs).unwrap().count, 48);
    let mut missing = started(0);
    missing.image(at(10), &image(1, 2.0));
    assert!(
        missing
            .snapshot(at(20))
            .unwrap()
            .summary(Metric::TextureBytes)
            .is_none()
    );
}

// ------------------------------------------------------------------ segments and sessions

#[test]
fn a_session_change_opens_a_segment_and_nothing_crosses_its_edge() {
    let mut recorder = playing(0);
    recorder.monitor_presented(at(40));
    recorder.request_admitted(5, at(60));
    recorder.session_changed(at(100), SessionKind::ProjectReplaced);
    assert_eq!(recorder.segment(), 1);
    assert_eq!(
        recorder.pending_requests(),
        0,
        "requests of the old session are forgotten"
    );
    // The answer to the old session arrives: no latency, no interval against the old frame.
    recorder.image(at(120), &image(5, 3.0));
    let snapshot = recorder.snapshot(at(200)).unwrap();
    assert!(snapshot.summary(Metric::LatencyMs).is_none());
    assert_eq!(
        snapshot.summary(Metric::MonitorIntervalMs).unwrap().max,
        40.0,
        "the first frame of segment 1 (20 ms after the edge) does not measure against the \
         frame of segment 0, and the 40 ms of segment 0 stays"
    );
    assert_eq!(
        snapshot.summary(Metric::MonitorIntervalMs).unwrap().count,
        2
    );
    let kinds: Vec<(&str, u32)> = snapshot
        .segments
        .iter()
        .map(|segment| (segment.kind.id(), segment.id))
        .collect();
    assert_eq!(kinds, vec![("recording", 0), ("project_replaced", 1)]);
    let replaced = snapshot
        .events
        .iter()
        .find(|e| e.kind == EventKind::ProjectReplaced)
        .unwrap();
    assert_eq!(replaced.segment, 1);
    assert_eq!(snapshot.segment, 1);
}

#[test]
fn the_first_frame_of_a_new_segment_has_no_interval_when_nothing_was_expected() {
    let mut recorder = started(0);
    recorder.monitor_presented(at(100));
    recorder.session_changed(at(500), SessionKind::SceneChanged);
    recorder.monitor_presented(at(600));
    let snapshot = recorder.snapshot(at(700)).unwrap();
    assert!(snapshot.summary(Metric::MonitorIntervalMs).is_none());
    assert_eq!(snapshot.summary(Metric::MonitorFrames).unwrap().count, 2);
    assert_eq!(snapshot.inspect(at(550)).unwrap().segment, Some(1));
    assert_eq!(snapshot.inspect(at(300)).unwrap().segment, Some(0));
}

// ------------------------------------------------------------------ events

#[test]
fn events_are_capped_separately_and_the_discarded_ones_are_reported() {
    let mut recorder = started(0);
    for step in 0..(MAX_EVENTS as u64 + 76) {
        recorder.event(at(step * 5), EventKind::ResourceLoadStarted, None);
    }
    // 1100 events (1 recording_started + 1099 loads) within 5.5 s; 1024 kept, 77 discarded.
    assert_eq!(recorder.events_held(), MAX_EVENTS);
    assert_eq!(recorder.dropped_events(), 77);
    let snapshot = recorder.snapshot(at(6_000)).unwrap();
    assert_eq!(snapshot.events.len(), MAX_EVENTS);
    assert_eq!(snapshot.dropped_events, 77);
    let document = snapshot.to_document();
    assert_eq!(document["events_dropped"], 77);
    assert_eq!(document["events_capacity"], 1024);
    assert_eq!(document["events"].as_array().unwrap().len(), 1024);
    // The metrics ring is untouched by the event cap.
    assert_eq!(snapshot.buckets.len(), BUCKETS);
}

#[test]
fn events_that_aged_out_are_not_counted_as_discarded() {
    let mut recorder = started(0);
    recorder.event(at(1_000), EventKind::ExportStarted, None);
    recorder.event(at(100_000), EventKind::ExportFinished, None);
    assert_eq!(recorder.dropped_events(), 0);
    assert_eq!(recorder.events_held(), 1);
}

#[test]
fn consecutive_seeks_fold_into_one_event_with_a_repeat_count() {
    let mut recorder = started(0);
    for (milliseconds, frame) in [(1_000, 10), (1_050, 20), (1_100, 30), (2_000, 40)] {
        recorder.seeked(at(milliseconds), frame);
    }
    let events = recorder.snapshot(at(2_100)).unwrap().events;
    let seeks: Vec<&Event> = events
        .iter()
        .filter(|event| event.kind == EventKind::Seek)
        .collect();
    assert_eq!(seeks.len(), 2);
    assert_eq!((seeks[0].repeat, seeks[0].frame), (3, Some(30)));
    assert_eq!((seeks[1].repeat, seeks[1].frame), (1, Some(40)));
}

#[test]
fn event_details_are_codes_never_paths_or_free_text() {
    assert_eq!(code("context_lost").as_deref(), Some("context_lost"));
    assert_eq!(code("/Users/me/secret.mp4"), None);
    assert_eq!(code("Meu projeto"), None);
    assert_eq!(code(&"a".repeat(49)), None);
    let mut recorder = started(0);
    recorder.event(
        at(10),
        EventKind::ExportFailed,
        Some("/Users/me/secret.mp4"),
    );
    recorder.event(at(20), EventKind::ExportFailed, Some("export_target_busy"));
    let events = recorder.snapshot(at(30)).unwrap().events;
    let details: Vec<Option<&str>> = events
        .iter()
        .filter(|e| e.kind == EventKind::ExportFailed)
        .map(|e| e.detail.as_deref())
        .collect();
    assert_eq!(details, vec![None, Some("export_target_busy")]);
}

#[test]
fn a_change_of_frame_size_is_an_event_and_the_same_size_is_not() {
    let mut recorder = started(0);
    let context = |width, quality: &str| Context {
        width,
        height: 540,
        quality: Some(quality.to_string()),
        ..Context::default()
    };
    recorder.set_context(at(10), context(960, "preview"));
    recorder.set_context(at(20), context(960, "preview"));
    recorder.set_context(at(30), context(1280, "original"));
    let changes = recorder
        .snapshot(at(40))
        .unwrap()
        .events
        .iter()
        .filter(|event| event.kind == EventKind::FrameFormatChanged)
        .count();
    assert_eq!(changes, 1);
    // A quality that is not a plain code is dropped, never exported.
    recorder.set_context(at(50), context(1280, "/private/path"));
    assert_eq!(recorder.context().unwrap().quality, None);
}

// ------------------------------------------------------------------ recorder lifecycle

#[test]
fn nothing_is_recorded_before_start_and_enabling_later_recovers_nothing() {
    let mut recorder = Recorder::default();
    recorder.ui_painted(at(10));
    recorder.monitor_presented(at(20));
    recorder.event(at(30), EventKind::Seek, None);
    recorder.request_admitted(1, at(40));
    assert!(recorder.snapshot(at(50)).is_none());
    assert!(!recorder.has_data());
    assert_eq!(recorder.events_held(), 0);
    assert_eq!(recorder.pending_requests(), 0);
    recorder.start(at(1_000), false);
    let snapshot = recorder.snapshot(at(1_100)).unwrap();
    assert!(snapshot.summary(Metric::UiFrames).is_none());
    assert!(snapshot.summary(Metric::MonitorFrames).is_none());
}

#[test]
fn stopping_keeps_what_was_recorded_readable_at_the_moment_it_stopped() {
    let mut recorder = started(0);
    recorder.sample(Metric::RenderMs, at(1_000), 9.0);
    assert!(recorder.stop(at(2_000)));
    assert!(!recorder.stop(at(2_100)), "stopping twice changes nothing");
    recorder.sample(Metric::RenderMs, at(2_500), 77.0);
    recorder.monitor_presented(at(2_600));
    // Even asked a minute later, the window ends where the recording ended.
    let snapshot = recorder.snapshot(at(90_000)).unwrap();
    assert_eq!(snapshot.captured_at, at(2_000));
    assert_eq!(snapshot.summary(Metric::RenderMs).unwrap().max, 9.0);
    assert!(snapshot.summary(Metric::MonitorFrames).is_none());
    assert_eq!(snapshot.recording_end, Some(at(2_000)));
    assert_eq!(snapshot.to_document()["recording"]["state"], "stopped");
    assert!(recorder.events_held() >= 2);
    assert!(
        recorder.rss_due(at(5_000)) == false,
        "a stopped recorder wants no memory reading"
    );
}

#[test]
fn starting_again_begins_a_new_recording_and_discards_the_old_one() {
    let mut recorder = started(0);
    recorder.sample(Metric::RenderMs, at(100), 9.0);
    recorder.stop(at(200));
    assert!(recorder.start(at(5_000), false));
    let snapshot = recorder.snapshot(at(5_100)).unwrap();
    assert!(snapshot.summary(Metric::RenderMs).is_none());
    assert_eq!(snapshot.recording_start, at(5_000));
    assert_eq!(snapshot.segments.len(), 1);
    assert!(
        !recorder.start(at(5_200), false),
        "already recording: no-op"
    );
}

#[test]
fn clear_is_explicit_resets_anchors_and_keeps_recording() {
    let mut recorder = playing(0);
    recorder.monitor_presented(at(40));
    recorder.request_admitted(1, at(50));
    recorder.freeze(at(60)).unwrap();
    recorder.select(Some(at(40)));
    recorder.clear(at(1_000), true);
    assert!(recorder.recording());
    assert!(
        recorder.frozen().is_none(),
        "a cleared history has nothing to keep frozen"
    );
    assert_eq!(recorder.selected(), None);
    assert_eq!(recorder.pending_requests(), 0);
    recorder.monitor_presented(at(1_040));
    let snapshot = recorder.snapshot(at(1_100)).unwrap();
    assert_eq!(snapshot.summary(Metric::MonitorFrames).unwrap().count, 1);
    assert_eq!(
        snapshot.summary(Metric::MonitorIntervalMs).unwrap().max,
        40.0,
        "measured from the clear, not from the frame of before"
    );
    assert_eq!(snapshot.recording_start, at(1_000));
    let mut stopped = started(0);
    stopped.stop(at(10));
    stopped.clear(at(20), false);
    assert!(!stopped.has_data());
    assert!(stopped.snapshot(at(30)).is_none());
}

// ------------------------------------------------------------------ freeze, selection, inspection

#[test]
fn freezing_fixes_a_bounded_snapshot_while_the_recording_continues() {
    let mut recorder = started(0);
    recorder.sample(Metric::RenderMs, at(100), 10.0);
    assert_eq!(recorder.freeze(at(500)).unwrap(), true);
    let fixed = recorder.frozen().unwrap().clone();
    assert!(fixed.frozen);
    // Collection goes on underneath.
    recorder.sample(Metric::RenderMs, at(700), 55.0);
    recorder.event(at(710), EventKind::Seek, None);
    recorder.monitor_presented(at(720));
    assert_eq!(
        recorder.frozen().unwrap(),
        &fixed,
        "the frozen snapshot is immutable"
    );
    assert_eq!(recorder.view(at(900)).unwrap(), fixed);
    let live = recorder.snapshot(at(900)).unwrap();
    assert_eq!(live.summary(Metric::RenderMs).unwrap().count, 2);
    assert!(!live.frozen);
    // One snapshot at a time: freezing again keeps the first.
    assert_eq!(recorder.freeze(at(900)).unwrap(), false);
    assert_eq!(recorder.frozen().unwrap(), &fixed);
    // Resume returns to the moving history, which kept growing.
    assert!(recorder.resume());
    assert!(!recorder.resume());
    assert_eq!(
        recorder
            .view(at(900))
            .unwrap()
            .summary(Metric::RenderMs)
            .unwrap()
            .count,
        2
    );
    assert!(recorder.recording());
}

#[test]
fn freezing_without_a_recording_is_a_typed_refusal() {
    let mut recorder = Recorder::default();
    assert_eq!(recorder.freeze(at(0)), Err(HistoryError::NoRecording));
    assert!(recorder.frozen().is_none());
    let error: CommandError = HistoryError::NoRecording.into();
    assert_eq!(error.code, "perf_history_empty");
}

#[test]
fn a_frozen_snapshot_does_not_grow_with_a_long_recording() {
    let mut recorder = started(0);
    recorder.freeze(at(1_000)).unwrap();
    let size = recorder.frozen().unwrap().buckets.len();
    for step in 0..14_400u64 {
        recorder.monitor_presented(at(1_000 + step * 42));
        recorder.event(at(1_000 + step * 42), EventKind::Seek, None);
    }
    assert_eq!(recorder.frozen().unwrap().buckets.len(), size);
    assert!(recorder.frozen().unwrap().events.len() <= MAX_EVENTS);
    assert!(recorder.events_held() <= MAX_EVENTS);
}

#[test]
fn selecting_an_instant_shows_its_numbers_and_the_events_that_explain_a_wait() {
    // Playback at 24 fps; a seek at 2.80 s; the next picture only arrives at 3.20 s.
    let mut recorder = playing(0);
    recorder.event(at(2_800), EventKind::ResourceLoadStarted, None);
    recorder.seeked(at(2_800), 100);
    recorder.monitor_presented(at(2_760));
    recorder.monitor_presented(at(3_200));
    recorder.monitor_presented(at(3_240));
    let snapshot = recorder.snapshot(at(3_300)).unwrap();
    let inspected = snapshot.inspect(at(3_210)).unwrap();
    let interval = inspected.metrics[Metric::MonitorIntervalMs.index()];
    assert_eq!(
        (interval.count, interval.max),
        (2, 440.0),
        "440 ms and the 40 ms after it"
    );
    assert_eq!(interval.max_at_us, 3_200_000);
    // The seek is 400 ms before the picture, outside the bucket, inside the wait.
    let kinds: Vec<EventKind> = inspected.events.iter().map(|e| e.kind).collect();
    assert!(kinds.contains(&EventKind::Seek), "{kinds:?}");
    assert!(kinds.contains(&EventKind::ResourceLoadStarted));
    // A quiet bucket far from it correlates with nothing.
    assert!(snapshot.inspect(at(1_000)).unwrap().events.is_empty());
    assert!(snapshot.inspect(Duration::from_secs(500)).is_none());
}

#[test]
fn selection_commands_validate_against_the_window_and_clear_with_none() {
    let mut recorder = started(0);
    recorder.sample(Metric::RenderMs, at(1_000), 12.0);
    let selected = recorder
        .apply(&HistoryCommand::Select(Some(at(1_050))), at(1_100))
        .unwrap();
    assert_eq!(selected["selected_ms"], 1050.0);
    assert_eq!(selected["inspection"]["metrics"]["render_ms"]["max"], 12.0);
    assert_eq!(recorder.selected(), Some(at(1_050)));
    let outside = recorder
        .apply(
            &HistoryCommand::Select(Some(Duration::from_secs(900))),
            at(1_100),
        )
        .unwrap_err();
    assert_eq!(outside.code, "perf_history_outside_window");
    // The largest instant a `Duration` holds must not wrap into a valid bucket of this young
    // recording (its first bucket is negative) nor overflow the index arithmetic.
    let absurd = recorder
        .apply(&HistoryCommand::Select(Some(Duration::MAX)), at(1_100))
        .unwrap_err();
    assert_eq!(absurd.code, "perf_history_outside_window");
    assert_eq!(
        recorder.selected(),
        Some(at(1_050)),
        "a refused selection changes nothing"
    );
    recorder
        .apply(&HistoryCommand::Select(None), at(1_100))
        .unwrap();
    assert_eq!(recorder.selected(), None);
    assert!(
        Recorder::default()
            .apply(&HistoryCommand::Select(Some(at(0))), at(0))
            .is_err()
    );
}

#[test]
fn visibility_and_recording_are_the_adapters_and_the_recorder_refuses_them() {
    let mut recorder = started(0);
    for command in [
        HistoryCommand::Show,
        HistoryCommand::Hide,
        HistoryCommand::Record(true),
    ] {
        let error = recorder.apply(&command, at(10)).unwrap_err();
        assert_eq!(error.code, "desktop_required");
    }
    assert!(
        recorder.recording(),
        "showing or hiding never changes recording"
    );
}

// ------------------------------------------------------------------ export

#[test]
fn the_document_has_the_schema_units_sources_origin_range_context_buckets_and_events() {
    let mut recorder = playing(1_000);
    recorder.set_context(
        at(1_010),
        Context {
            width: 960,
            height: 540,
            quality: Some("preview".to_string()),
            playing: true,
            sequence_fps: Some(24.0),
            ..Context::default()
        },
    );
    recorder.request_admitted(1, at(1_020));
    recorder.image(at(1_060), &image(1, 6.0));
    let document = recorder.export_document(at(1_100)).unwrap();
    assert_eq!(document["schema"], "tessel.perf-history");
    assert_eq!(document["schema_version"], 1);
    assert_eq!(
        document["build"]["profile"],
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
    assert!(document["build"]["version"].is_string());
    assert_eq!(document["monotonic_origin"]["unit"], "ms");
    assert_eq!(document["recording"]["started_at_ms"], 1000.0);
    assert_eq!(document["recording"]["frozen"], false);
    assert_eq!(document["window"]["bucket_ms"], 250.0);
    assert_eq!(document["window"]["buckets"], 240);
    assert_eq!(document["context"]["width"], 960);
    assert_eq!(document["context"]["sequence_target_fps"], 24.0);
    assert_eq!(
        document["context"]["ui_target_fps"],
        Value::Null,
        "the UI target is never assumed to be 60"
    );
    for metric in Metric::ALL {
        let id = metric.id();
        assert_eq!(document["metrics"][id]["unit"], metric.unit(), "{id}");
        assert!(document["metrics"][id]["source"].as_str().unwrap().len() > 20);
        for column in ["count", "min", "mean", "max"] {
            assert_eq!(
                document["series"][id][column].as_array().unwrap().len(),
                240,
                "{id}.{column}"
            );
        }
    }
    assert!(!document["events"].as_array().unwrap().is_empty());
    assert!(document["limits"].as_array().unwrap().len() >= 5);
    // Bytes and milliseconds are different series with different units.
    assert_eq!(document["metrics"]["rss_bytes"]["unit"], "bytes");
    assert_eq!(document["metrics"]["latency_ms"]["unit"], "ms");
}

#[test]
fn the_document_carries_no_path_project_text_or_credential() {
    let mut recorder = playing(0);
    recorder.event(
        at(10),
        EventKind::ExportFailed,
        Some("/Users/me/Meu Projeto/secret.mp4"),
    );
    recorder.set_context(
        at(20),
        Context {
            width: 10,
            height: 10,
            quality: Some("/Users/me/private".to_string()),
            ..Context::default()
        },
    );
    recorder.request_failed(at(30), None, "C:\\Users\\me\\token");
    let text = serde_json::to_string(&recorder.export_document(at(100)).unwrap()).unwrap();
    for forbidden in [
        "/Users",
        "secret",
        "Meu Projeto",
        "token",
        "C:\\\\",
        "password",
        "http",
    ] {
        assert!(
            !text.contains(forbidden),
            "`{forbidden}` leaked into the export"
        );
    }
}

#[test]
fn exporting_writes_a_valid_file_atomically_and_leaves_no_temporary_behind() {
    let directory = scratch("export");
    let target = directory.join("diagnostic.json");
    let mut recorder = started(0);
    recorder.sample(Metric::RenderMs, at(100), 33.0);
    let result = recorder
        .apply(&HistoryCommand::Export(target.clone()), at(200))
        .unwrap();
    assert!(result["bytes"].as_u64().unwrap() > 1000);
    let written: Value = serde_json::from_str(&std::fs::read_to_string(&target).unwrap()).unwrap();
    assert_eq!(written, recorder.export_document(at(200)).unwrap());
    let leftovers: Vec<String> = std::fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(leftovers, vec!["diagnostic.json".to_string()]);
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn export_errors_are_typed_observable_and_change_nothing() {
    let directory = scratch("errors");
    let mut recorder = started(0);
    recorder.sample(Metric::RenderMs, at(100), 1.0);
    let code_of = |recorder: &mut Recorder, path: PathBuf| {
        recorder
            .apply(&HistoryCommand::Export(path), at(200))
            .unwrap_err()
            .code
    };
    assert_eq!(
        code_of(&mut recorder, PathBuf::new()),
        "perf_history_export_path_missing"
    );
    assert_eq!(
        code_of(&mut recorder, directory.clone()),
        "perf_history_export_is_directory"
    );
    assert_eq!(
        code_of(&mut recorder, directory.join("missing").join("a.json")),
        "perf_history_export_parent_missing"
    );
    assert_eq!(
        code_of(&mut Recorder::default(), directory.join("a.json")),
        "perf_history_empty"
    );
    assert!(
        std::fs::read_dir(&directory).unwrap().next().is_none(),
        "nothing was created"
    );
    assert!(recorder.recording());
    assert_eq!(
        recorder
            .snapshot(at(200))
            .unwrap()
            .summary(Metric::RenderMs)
            .unwrap()
            .count,
        1
    );
    let _ = std::fs::remove_dir_all(&directory);
}

/// A folder that refuses the write: typed error, nothing written, no temporary left. Unix only
/// (permission bits), and skipped when the process can write anyway (root, some CI containers).
#[cfg(unix)]
#[test]
fn a_folder_that_refuses_the_write_is_a_typed_error_and_leaves_no_sibling() {
    use std::os::unix::fs::PermissionsExt;
    let directory = scratch("locked");
    let locked = directory.join("locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
    let probe = locked.join("probe");
    if std::fs::write(&probe, b"x").is_ok() {
        // The permission is not enforced for this process: nothing to prove here.
        let _ = std::fs::remove_file(&probe);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        let _ = std::fs::remove_dir_all(&directory);
        return;
    }
    let mut recorder = started(0);
    recorder.sample(Metric::RenderMs, at(100), 1.0);
    let target = locked.join("a.json");
    let refused = recorder
        .apply(&HistoryCommand::Export(target.clone()), at(200))
        .unwrap_err();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(refused.code, "perf_history_export_failed");
    assert!(refused.details.is_some());
    assert!(!target.exists(), "nothing was written");
    assert_eq!(
        std::fs::read_dir(&locked).unwrap().count(),
        0,
        "no sibling left"
    );
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn an_explicit_destination_must_be_absolute_and_new_and_nothing_is_ever_replaced() {
    let directory = scratch("destination");
    let mut recorder = started(0);
    recorder.sample(Metric::RenderMs, at(100), 1.0);
    let code_of = |recorder: &mut Recorder, path: PathBuf| {
        recorder
            .apply(&HistoryCommand::Export(path), at(200))
            .unwrap_err()
            .code
    };
    assert_eq!(
        code_of(&mut recorder, PathBuf::from("diagnostic.json")),
        "perf_history_export_relative"
    );
    assert_eq!(
        code_of(&mut recorder, PathBuf::from("../diagnostic.json")),
        "perf_history_export_relative"
    );
    // An existing file (the project, a note) is refused and left byte for byte as it was.
    let existing = directory.join("project.tessel.json");
    std::fs::write(&existing, "{\"precious\":true}").unwrap();
    assert_eq!(
        code_of(&mut recorder, existing.clone()),
        "perf_history_export_exists"
    );
    assert_eq!(
        std::fs::read_to_string(&existing).unwrap(),
        "{\"precious\":true}"
    );
    // A dangling link is also something that is there.
    let link = directory.join("link.json");
    std::os::unix::fs::symlink(directory.join("nowhere.json"), &link).unwrap();
    assert_eq!(code_of(&mut recorder, link), "perf_history_export_exists");
    assert!(!directory.join("nowhere.json").exists());
    // The refusals left no temporary behind, and the first export to a new name works once.
    let target = directory.join("new.json");
    recorder
        .apply(&HistoryCommand::Export(target.clone()), at(200))
        .unwrap();
    assert_eq!(
        code_of(&mut recorder, target.clone()),
        "perf_history_export_exists"
    );
    let mut names: Vec<String> = std::fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, vec!["link.json", "new.json", "project.tessel.json"]);
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn the_destination_commands_belong_to_the_window_not_to_the_recorder() {
    let mut recorder = started(0);
    for command in [
        HistoryCommand::Destination("/tmp/a.json".to_string()),
        HistoryCommand::ExportToDestination,
    ] {
        assert_eq!(
            recorder.apply(&command, at(10)).unwrap_err().code,
            "desktop_required"
        );
    }
}

#[test]
fn a_lost_context_is_one_event_until_it_is_restored_and_a_failed_frame_is_not_a_second_one() {
    let mut recorder = started(0);
    recorder.request_admitted(5, at(100));
    // The page says it lost the context and the frame in flight fails with the same code.
    recorder.context_lost(at(110), None);
    recorder.request_failed(at(112), Some(5), "context_lost");
    recorder.context_restored(at(400));
    recorder.request_failed(at(900), None, "context_lost");
    let kinds: Vec<EventKind> = recorder
        .snapshot(at(1_000))
        .unwrap()
        .events
        .iter()
        .map(|event| event.kind)
        .collect();
    assert_eq!(
        kinds,
        vec![
            EventKind::RecordingStarted,
            EventKind::ContextLost,
            EventKind::ContextRestored,
            EventKind::ContextLost,
        ]
    );
    assert_eq!(
        recorder.pending_requests(),
        0,
        "the failed request is not waited for"
    );
    // Another failure is still a failed request, not a context loss.
    recorder.request_failed(at(950), None, "render_failed");
    let events = recorder.snapshot(at(1_000)).unwrap().events;
    assert_eq!(
        events.last().map(|event| event.kind),
        Some(EventKind::RequestFailed)
    );
}

#[test]
fn context_events_are_not_recorded_while_not_recording() {
    let mut recorder = Recorder::default();
    recorder.context_lost(at(10), None);
    recorder.context_restored(at(20));
    assert!(!recorder.has_data());
}

fn export_job(id: &str, state: JobState, error: Option<&str>) -> ExportJob {
    ExportJob {
        id: id.to_string(),
        state,
        error_code: error.map(str::to_string),
    }
}

fn export_events(recorder: &Recorder, now: u64) -> Vec<(EventKind, Option<String>)> {
    recorder
        .snapshot(at(now))
        .unwrap()
        .events
        .iter()
        .filter(|event| {
            matches!(
                event.kind,
                EventKind::ExportStarted
                    | EventKind::ExportFinished
                    | EventKind::ExportFailed
                    | EventKind::ExportCancelled
            )
        })
        .map(|event| (event.kind, event.detail.clone()))
        .collect()
}

#[test]
fn export_jobs_become_events_only_for_what_was_seen_to_change_after_the_baseline() {
    let mut recorder = started(0);
    assert!(recorder.exports_due(at(0)));
    // The baseline: an export that already ran, and one that is running, are not announced.
    recorder.exports_observed(
        at(10),
        &[
            export_job("old", JobState::Succeeded, None),
            export_job("busy", JobState::Running, None),
        ],
    );
    assert!(!recorder.exports_due(at(500)), "at most once per second");
    assert!(recorder.exports_due(at(1_010)));
    assert!(export_events(&recorder, 1_100).is_empty());
    recorder.exports_observed(
        at(1_010),
        &[
            export_job("old", JobState::Succeeded, None),
            export_job("busy", JobState::Succeeded, None),
            export_job("new", JobState::Queued, None),
        ],
    );
    recorder.exports_observed(
        at(2_100),
        &[
            export_job("old", JobState::Succeeded, None),
            export_job("busy", JobState::Succeeded, None),
            export_job("new", JobState::Running, None),
        ],
    );
    recorder.exports_observed(
        at(3_200),
        &[
            export_job("busy", JobState::Succeeded, None),
            export_job("new", JobState::Failed, Some("encoder_failed")),
            // Seen for the first time already finished: only its end is known.
            export_job("quick", JobState::Cancelled, None),
        ],
    );
    // Looking again at the same states adds nothing.
    recorder.exports_observed(
        at(4_300),
        &[
            export_job("new", JobState::Failed, Some("encoder_failed")),
            export_job("quick", JobState::Cancelled, None),
        ],
    );
    assert_eq!(
        export_events(&recorder, 4_400),
        vec![
            (EventKind::ExportFinished, None),
            (EventKind::ExportStarted, None),
            (EventKind::ExportFailed, Some("encoder_failed".to_string())),
            (EventKind::ExportCancelled, None),
        ]
    );
}

#[test]
fn an_export_error_that_is_not_a_plain_code_is_kept_out_of_the_event() {
    let mut recorder = started(0);
    recorder.exports_observed(at(0), &[]);
    recorder.exports_observed(
        at(1_000),
        &[export_job(
            "a",
            JobState::Failed,
            Some("/Users/me/secret.mp4"),
        )],
    );
    assert_eq!(
        export_events(&recorder, 1_100),
        vec![(EventKind::ExportFailed, None)]
    );
}

#[test]
fn a_new_recording_takes_a_new_baseline_for_exports() {
    let mut recorder = started(0);
    recorder.exports_observed(at(0), &[export_job("a", JobState::Running, None)]);
    recorder.stop(at(500));
    assert!(recorder.start(at(600), false));
    recorder.exports_observed(at(700), &[export_job("a", JobState::Succeeded, None)]);
    assert!(
        export_events(&recorder, 800).is_empty(),
        "what finished between recordings is not news of this one"
    );
}

#[test]
fn exporting_a_frozen_view_writes_the_frozen_snapshot_not_the_moving_window() {
    let directory = scratch("frozen");
    let mut recorder = started(0);
    recorder.sample(Metric::RenderMs, at(100), 10.0);
    recorder.freeze(at(300)).unwrap();
    recorder.sample(Metric::RenderMs, at(400), 90.0);
    let target = directory.join("frozen.json");
    recorder
        .apply(&HistoryCommand::Export(target.clone()), at(500))
        .unwrap();
    let written: Value = serde_json::from_str(&std::fs::read_to_string(&target).unwrap()).unwrap();
    assert_eq!(written["recording"]["frozen"], true);
    assert_eq!(written["summary"]["render_ms"]["max"], 10.0);
    assert_eq!(written["recording"]["captured_at_ms"], 300.0);
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn controlled_samples_produce_the_expected_peaks_events_and_the_export_agrees_with_the_inspection()
{
    let mut recorder = playing(0);
    recorder.loss_run_started(1);
    // 24 fps for 5 s, with one 400 ms stall at 3 s after a seek, and 3 lost frames.
    let mut time = 0u64;
    let mut cumulative = 0u64;
    let mut request = 0u64;
    let mut stalled = false;
    while time < 5_000 {
        let step = if time >= 3_000 && !stalled {
            stalled = true;
            recorder.seeked(at(time), 72);
            400
        } else {
            41
        };
        request += 1;
        recorder.request_admitted(request, at(time));
        time += step;
        if step == 400 {
            cumulative += 3;
        }
        recorder.image(
            at(time),
            &image(request, if step == 400 { 120.0 } else { 6.0 }),
        );
        recorder.loss_observed(at(time), 1, cumulative);
    }
    let now = at(5_100);
    let snapshot = recorder.snapshot(now).unwrap();
    let intervals = snapshot.summary(Metric::MonitorIntervalMs).unwrap();
    assert_eq!(intervals.max, 400.0);
    assert!(
        (3_000.0..=3_450.0).contains(&intervals.peak_at_ms),
        "{}",
        intervals.peak_at_ms
    );
    assert_eq!(snapshot.summary(Metric::RenderMs).unwrap().max, 120.0);
    assert_eq!(snapshot.summary(Metric::LatencyMs).unwrap().max, 400.0);
    let lost: f64 = snapshot
        .buckets
        .iter()
        .map(|aggs| aggs[Metric::LostFrames.index()].sum)
        .sum();
    assert_eq!(lost, 3.0);
    // Select the peak: the seek that caused the wait is correlated.
    let peak = Duration::from_secs_f64(intervals.peak_at_ms / 1000.0);
    recorder.select(Some(peak));
    let inspection = recorder.inspection(now).unwrap();
    assert!(
        inspection
            .events
            .iter()
            .any(|e| e.kind == EventKind::Seek && e.frame == Some(72))
    );
    // The export holds the same numbers at that bucket as the inspection shows.
    let document = recorder.export_document(now).unwrap();
    let bucket = inspection.bucket;
    assert_eq!(
        document["series"]["monitor_interval_ms"]["max"][bucket],
        serde_json::json!(400.0)
    );
    assert_eq!(
        document["series"]["monitor_interval_ms"]["max"][bucket],
        inspection.to_json()["metrics"]["monitor_interval_ms"]["max"]
    );
    assert_eq!(document["summary"]["monitor_interval_ms"]["max"], 400.0);
    assert!(
        document["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "seek" && e["frame"] == 72)
    );
}

// ------------------------------------------------------------------ bounds

#[test]
fn ten_minutes_of_feeds_keep_every_buffer_at_its_bound() {
    let mut recorder = playing(0);
    recorder.loss_run_started(1);
    for step in 0..14_400u64 {
        let time = step * 42;
        recorder.request_admitted(step, at(time));
        recorder.image(at(time + 10), &image(step, 4.0));
        recorder.ui_painted(at(time));
        recorder.loss_observed(at(time), 1, step / 100);
        if step % 8 == 0 {
            recorder.event(at(time), EventKind::ResourceLoadStarted, None);
        }
        if recorder.rss_due(at(time)) {
            recorder.rss_read(at(time), Some(100 << 20));
        }
    }
    let snapshot = recorder.snapshot(at(14_400 * 42)).unwrap();
    assert_eq!(snapshot.buckets.len(), BUCKETS);
    assert!(recorder.events_held() <= MAX_EVENTS);
    assert!(snapshot.events.len() <= MAX_EVENTS);
    assert!(recorder.pending_requests() <= MAX_PENDING);
    assert!(snapshot.segments.len() <= MAX_SEGMENTS);
    assert!(
        recorder.dropped_events() == 0,
        "1800 events spread over 10 minutes age out, they are not capacity drops"
    );
    // Old minutes are gone: the window holds ~60 s of pictures, not 10 minutes.
    let frames = snapshot.summary(Metric::MonitorFrames).unwrap().count;
    assert!((1_400..=1_460).contains(&frames), "{frames}");
}

#[test]
fn a_target_distinct_from_sixty_fps_travels_in_the_context() {
    let mut recorder = started(0);
    recorder.set_context(
        at(10),
        Context {
            width: 1,
            height: 1,
            sequence_fps: Some(24.0),
            ..Context::default()
        },
    );
    let json = recorder.snapshot(at(20)).unwrap().compact_json();
    assert_eq!(json["context"]["sequence_target_fps"], 24.0);
    assert_eq!(json["context"]["ui_target_fps"], Value::Null);
}

#[test]
fn values_read_with_their_own_unit_and_an_empty_bucket_says_so() {
    assert_eq!(Metric::MonitorIntervalMs.format(41.0), "41,0 ms");
    assert_eq!(Metric::LatencyMs.format(412.0), "412 ms");
    assert_eq!(Metric::RssBytes.format(300.0 * 1024.0 * 1024.0), "300 MiB");
    assert_eq!(
        rate_fps(6),
        24.0,
        "6 pictures in a 250 ms bucket are 24 fps"
    );
    let mut bucket = Agg::default();
    assert_eq!(describe(Metric::RenderMs, &bucket), "sem amostra");
    bucket.add(10.0, at(0));
    bucket.add(30.0, at(10));
    assert_eq!(
        describe(Metric::RenderMs, &bucket),
        "2× · mín 10,0 ms · méd 20,0 ms · máx 30,0 ms"
    );
    let mut frames = Agg::default();
    for step in 0..6 {
        frames.add(1.0, at(step));
    }
    assert_eq!(
        describe(Metric::MonitorFrames, &frames),
        "6 quadros · 24 fps"
    );
    // Bytes and milliseconds never share a scale group.
    assert_ne!(Metric::RssBytes.group(), Metric::RenderMs.group());
    assert_ne!(Metric::RssBytes.unit(), Metric::LatencyMs.unit());
}

#[test]
fn refusing_a_busy_temporary_preserves_the_other_writers_bytes() {
    let directory = scratch("busy-temporary");
    let target = directory.join("history.json");
    let temporary = directory.join(format!(".history.json.tmp-{}", std::process::id()));
    std::fs::write(&temporary, b"other writer").unwrap();
    let mut recorder = started(0);
    recorder.sample(Metric::RenderMs, at(100), 1.0);
    let error = recorder
        .apply(&HistoryCommand::Export(target.clone()), at(200))
        .unwrap_err();
    assert_eq!(error.code, "perf_history_export_exists");
    assert!(!target.exists());
    assert_eq!(std::fs::read(&temporary).unwrap(), b"other writer");
    std::fs::remove_dir_all(directory).unwrap();
}

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use serde_json::{Value, json};

use super::*;
use crate::assets::{Asset, Declared, MediaKind};
use crate::assets::{
    Declared as DeclaredAsset, Decoded, DecodedFrames, ProxyFormat, ProxyFrames, Source,
};
use crate::backend::{BackendEvent, Capabilities, ProxyState};
use crate::fixtures::write_standard_set;
use crate::plan::{PlanRequest, PlanTime, PreviewIntent};
use crate::state::{Command, ProjectSettings};

/// What the fake saw and what it will answer; shared so a test can inspect it while the
/// session owns the backend.
#[derive(Default)]
struct Script {
    events: VecDeque<BackendEvent>,
    loaded: Vec<Id>,
    released: Vec<Id>,
    submitted: Vec<(RequestId, u64)>,
    /// `FrameRequest::inspect` of each submitted frame, in order.
    inspected: Vec<bool>,
    cancelled: Vec<RequestId>,
    fail_load: Option<BackendError>,
    layers: Vec<&'static str>,
    /// `asset.proxy` calls in order: the proxy descriptor, `None` for a detach.
    proxies: Vec<(Id, Option<Value>)>,
    /// `FrameRequest::quality` of each submitted frame, in order.
    qualities: Vec<Quality>,
    /// Optional features this fake runtime announces.
    features: Vec<&'static str>,
    /// `frame.warm` calls in order: the assets each plan requires.
    warmed: Vec<Vec<Id>>,
}

#[derive(Clone)]
struct Fake(Rc<RefCell<Script>>);

impl RenderBackend for Fake {
    fn name(&self) -> &str {
        "fake"
    }
    fn capabilities(&mut self) -> Result<Capabilities, BackendError> {
        let layers = self
            .0
            .borrow()
            .layers
            .iter()
            .map(|kind| kind.to_string())
            .collect();
        Ok(Capabilities {
            backend: "fake".into(),
            bridge_version: 1,
            runtime_version: "0".into(),
            renderer: "none".into(),
            layers,
            formats: json!({}),
            raw: json!({ "capabilities": { "features": self.0.borrow().features } }),
        })
    }
    fn load_asset(&mut self, descriptor: &AssetDescriptor) -> Result<(), BackendError> {
        let mut script = self.0.borrow_mut();
        if let Some(error) = script.fail_load.clone() {
            return Err(error);
        }
        script.loaded.push(descriptor.id);
        Ok(())
    }
    fn release_asset(&mut self, id: Id) -> Result<(), BackendError> {
        self.0.borrow_mut().released.push(id);
        Ok(())
    }
    fn set_proxy(&mut self, id: Id, proxy: Option<&Value>) -> Result<(), BackendError> {
        self.0.borrow_mut().proxies.push((id, proxy.cloned()));
        Ok(())
    }
    fn warm_scene(&mut self, plan: &RenderPlan) -> Result<(), BackendError> {
        self.0.borrow_mut().warmed.push(plan.requires.clone());
        Ok(())
    }
    fn submit(&mut self, request: &FrameRequest) -> Result<(), BackendError> {
        self.0.borrow_mut().qualities.push(request.quality);
        self.0
            .borrow_mut()
            .submitted
            .push((request.id, request.revision));
        self.0.borrow_mut().inspected.push(request.inspect);
        Ok(())
    }
    fn cancel(&mut self, id: RequestId) -> Result<(), BackendError> {
        self.0.borrow_mut().cancelled.push(id);
        Ok(())
    }
    fn resize(&mut self, _: u32, _: u32) -> Result<(), BackendError> {
        Ok(())
    }
    fn poll(&mut self, _: Duration) -> Option<BackendEvent> {
        self.0.borrow_mut().events.pop_front()
    }
    fn shutdown(&mut self) {}
}

fn frame(id: RequestId, revision: u64) -> FrameResult {
    FrameResult {
        id,
        revision,
        width: 4,
        height: 4,
        hash: "00".into(),
        mean: [0.0; 4],
        probes: vec![],
        png: None,
        render_ms: 0.0,
        total_ms: 0.0,
        detail: Default::default(),
    }
}

struct Rig {
    state: EditorState,
    session: ViewportSession<Fake>,
    script: Rc<RefCell<Script>>,
}

/// A project with one image asset on one video track, so a plan requires exactly `img`.
fn rig() -> Rig {
    // One directory per rig: tests run in parallel and one of them deletes a fixture.
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("tessel-session-{}-{n}", std::process::id()));
    let set = write_standard_set(&dir).unwrap();
    let mut state = EditorState::empty(&ProjectSettings::default()).unwrap();
    let asset = Asset::probe(
        MediaKind::Still,
        &set.image.to_string_lossy(),
        &Declared::default(),
    )
    .unwrap();
    state
        .execute(Command::AddAsset {
            id: Some("img".into()),
            name: "Img".into(),
            kind: MediaKind::Still,
            asset: Box::new(asset),
            folder: None,
        })
        .unwrap();
    state
        .execute(Command::CreateTrack {
            id: Some("v1".into()),
            name: "V1".into(),
            kind: crate::state::timeline::TrackKind::Video,
        })
        .unwrap();
    state
        .execute(Command::AddClip {
            id: Some("c1".into()),
            name: None,
            track: "v1".into(),
            asset: Some("img".into()),
            start: 0,
            duration: 48,
            source_start: None,
            text: None,
            camera: None,
            properties: vec![],
        })
        .unwrap();
    let script = Rc::new(RefCell::new(Script {
        layers: vec!["image", "video", "text", "scene", "waveform"],
        ..Script::default()
    }));
    Rig {
        state,
        session: ViewportSession::new(Fake(script.clone())),
        script,
    }
}

fn output() -> Output {
    Output {
        width: 4,
        height: 4,
        png: None,
    }
}

fn plan(rig: &Rig, frame: i64) -> RenderPlan {
    rig.state
        .plan(&PlanRequest {
            intent: PreviewIntent::Sequence,
            at: PlanTime::Frame(frame),
        })
        .unwrap()
}

fn ready(id: &str) -> BackendEvent {
    BackendEvent::Asset {
        id: Id::lit(id),
        status: AssetStatus::Ready(json!({})),
        echo: None,
    }
}

#[test]
fn assets_load_before_the_frame_is_submitted_and_only_once() {
    let mut rig = rig();
    let first = rig
        .session
        .request_frame(&rig.state, plan(&rig, 0), output(), vec![])
        .unwrap();
    assert_eq!(rig.session.status().label(), "loading");
    assert!(
        rig.script.borrow().submitted.is_empty(),
        "nothing is drawn before the asset is ready"
    );
    rig.script
        .borrow_mut()
        .events
        .extend([ready("img"), BackendEvent::FrameDone(frame(first, 1))]);
    assert_eq!(
        rig.session.pump(Duration::ZERO),
        Some(SessionEvent::Progress)
    );
    assert_eq!(rig.session.status().label(), "rendering");
    assert!(matches!(
        rig.session.pump(Duration::ZERO),
        Some(SessionEvent::Frame(_))
    ));
    // A second seek reuses the loaded asset and submits right away.
    let second = rig
        .session
        .request_frame(&rig.state, plan(&rig, 10), output(), vec![])
        .unwrap();
    assert_eq!(rig.script.borrow().loaded, vec![Id::lit("img")]);
    assert_eq!(rig.script.borrow().submitted.last(), Some(&(second, 2)));
}

#[test]
fn an_overtaken_seek_is_cancelled_and_its_late_answer_never_shown() {
    let mut rig = rig();
    rig.script.borrow_mut().events.push_back(ready("img"));
    let old = rig
        .session
        .request_frame(&rig.state, plan(&rig, 0), output(), vec![])
        .unwrap();
    rig.session.pump(Duration::ZERO);
    let new = rig
        .session
        .request_frame(&rig.state, plan(&rig, 30), output(), vec![])
        .unwrap();
    assert_eq!(rig.script.borrow().cancelled, vec![old]);
    rig.script.borrow_mut().events.extend([
        BackendEvent::FrameDone(frame(old, 1)),
        BackendEvent::FrameDone(frame(new, 2)),
    ]);
    assert_eq!(
        rig.session.pump(Duration::ZERO),
        Some(SessionEvent::Stale(old))
    );
    assert!(matches!(rig.session.status(), Status::Rendering(id) if *id == new));
    assert!(
        matches!(rig.session.pump(Duration::ZERO), Some(SessionEvent::Frame(f)) if f.id == new)
    );
    assert_eq!(
        (
            rig.session.stats.dropped_stale,
            rig.session.stats.accepted,
            rig.session.stats.cancelled
        ),
        (1, 1, 1)
    );
}

#[test]
fn cancel_drops_the_answer_and_returns_to_idle() {
    let mut rig = rig();
    rig.script.borrow_mut().events.push_back(ready("img"));
    let id = rig
        .session
        .request_frame(&rig.state, plan(&rig, 0), output(), vec![])
        .unwrap();
    rig.session.pump(Duration::ZERO);
    rig.session.cancel();
    assert_eq!(rig.session.status().label(), "idle");
    rig.script
        .borrow_mut()
        .events
        .push_back(BackendEvent::FrameDone(frame(id, 1)));
    assert_eq!(
        rig.session.pump(Duration::ZERO),
        Some(SessionEvent::Stale(id))
    );
    assert_eq!(rig.session.stats.accepted, 0);
}

#[test]
fn a_missing_file_fails_before_the_backend_is_asked_to_load_it() {
    let mut rig = rig();
    let path = match &rig
        .state
        .library
        .item("img")
        .unwrap()
        .asset
        .as_ref()
        .unwrap()
        .source
    {
        crate::assets::Source::File { path } => path.clone(),
        other => panic!("{other:?}"),
    };
    std::fs::remove_file(&path).unwrap();
    let error = rig
        .session
        .request_frame(&rig.state, plan(&rig, 0), output(), vec![])
        .unwrap_err();
    assert_eq!(error.code, "asset_unreadable");
    assert!(error.message.contains(&path), "{error:?}");
    assert!(rig.script.borrow().loaded.is_empty());
    assert_eq!(rig.session.status().label(), "failed");
    // The project is intact: restoring the file lets the next seek work.
    write_standard_set(std::path::Path::new(&path).parent().unwrap()).unwrap();
    assert!(
        rig.session
            .request_frame(&rig.state, plan(&rig, 0), output(), vec![])
            .is_ok()
    );
}

#[test]
fn asset_failures_from_the_runtime_surface_with_the_asset_id() {
    let mut rig = rig();
    rig.script
        .borrow_mut()
        .events
        .push_back(BackendEvent::Asset {
            id: Id::lit("img"),
            status: AssetStatus::Failed(BackendError::new("decode_failed", "bad PNG")),
            echo: None,
        });
    rig.session
        .request_frame(&rig.state, plan(&rig, 0), output(), vec![])
        .unwrap();
    let Some(SessionEvent::Failed(error)) = rig.session.pump(Duration::ZERO) else {
        panic!("expected a failure")
    };
    assert_eq!(error.code, "decode_failed");
    assert!(error.message.starts_with("`img`"), "{error:?}");
    assert!(
        rig.session.loaded_assets().is_empty(),
        "a failed asset is retried by the next request"
    );
}

#[test]
fn layers_the_backend_cannot_draw_are_refused_up_front() {
    let mut rig = rig();
    rig.script.borrow_mut().layers = vec!["text"];
    let error = rig
        .session
        .request_frame(&rig.state, plan(&rig, 0), output(), vec![])
        .unwrap_err();
    assert_eq!(error.code, "unsupported_format");
    assert!(rig.script.borrow().submitted.is_empty());
}

#[test]
fn context_loss_reloads_assets_and_asks_for_a_redraw() {
    let mut rig = rig();
    rig.script.borrow_mut().events.push_back(ready("img"));
    rig.session
        .request_frame(&rig.state, plan(&rig, 0), output(), vec![])
        .unwrap();
    rig.session.pump(Duration::ZERO);
    rig.script
        .borrow_mut()
        .events
        .extend([BackendEvent::ContextLost, BackendEvent::ContextRestored]);
    assert!(
        matches!(rig.session.pump(Duration::ZERO), Some(SessionEvent::Failed(e)) if e.code == "context_lost")
    );
    assert!(rig.session.loaded_assets().is_empty());
    rig.session.pump(Duration::ZERO);
    assert!(rig.session.needs_redraw);
    rig.session
        .request_frame(&rig.state, plan(&rig, 0), output(), vec![])
        .unwrap();
    assert_eq!(
        rig.script.borrow().loaded.len(),
        2,
        "the asset is uploaded again after the context returns"
    );
}

#[test]
fn a_closed_backend_reports_unavailable_and_forgets_capabilities() {
    let mut rig = rig();
    rig.script.borrow_mut().events.push_back(ready("img"));
    rig.session
        .request_frame(&rig.state, plan(&rig, 0), output(), vec![])
        .unwrap();
    rig.session.pump(Duration::ZERO);
    rig.script
        .borrow_mut()
        .events
        .push_back(BackendEvent::Closed("worker exited".into()));
    assert!(
        matches!(rig.session.pump(Duration::ZERO), Some(SessionEvent::Failed(e)) if e.code == "backend_unavailable")
    );
    assert_eq!(rig.session.status().label(), "failed");
}

#[test]
fn render_blocking_times_out_and_cancels_instead_of_hanging() {
    let mut rig = rig();
    let error = rig
        .session
        .render_blocking(
            &rig.state,
            plan(&rig, 0),
            output(),
            vec![],
            Duration::from_millis(30),
        )
        .unwrap_err();
    assert_eq!(error.code, "timeout");
    assert_eq!(
        rig.script.borrow().cancelled.len(),
        1,
        "the request is cancelled when the wait gives up"
    );
}

#[test]
fn render_blocking_skips_stale_answers_and_returns_its_own() {
    let mut rig = rig();
    rig.script.borrow_mut().events.extend([
        ready("img"),
        BackendEvent::FrameDone(frame(99, 99)),
        BackendEvent::FrameDone(frame(1, 1)),
    ]);
    let done = rig
        .session
        .render_blocking(
            &rig.state,
            plan(&rig, 0),
            output(),
            vec![],
            Duration::from_secs(1),
        )
        .unwrap();
    assert_eq!(done.id, 1);
    assert_eq!(rig.session.stats.dropped_stale, 1);
}

#[test]
fn replacing_the_project_releases_everything() {
    let mut rig = rig();
    rig.script.borrow_mut().events.push_back(ready("img"));
    rig.session
        .request_frame(&rig.state, plan(&rig, 0), output(), vec![])
        .unwrap();
    rig.session.pump(Duration::ZERO);
    rig.session.reset_project();
    assert_eq!(rig.script.borrow().released, vec![Id::lit("img")]);
    assert!(rig.session.loaded_assets().is_empty());
    assert_eq!(rig.session.status().label(), "idle");
}

// ---- interactive preview scheduling (scrub) ----

/// A rig whose asset is loaded, with `frame` 0 of the sequence rendering as an interactive
/// preview. Returns the rig and that request's ID.
fn rig_rendering_preview() -> (Rig, RequestId) {
    let mut rig = rig();
    rig.script.borrow_mut().events.push_back(ready("img"));
    let Preview::Submitted(first) = rig
        .session
        .request_preview(&rig.state, plan(&rig, 0), output())
        .unwrap()
    else {
        panic!("the first preview is submitted")
    };
    assert_eq!(
        rig.session.pump(Duration::ZERO),
        Some(SessionEvent::Progress)
    );
    assert_eq!(rig.session.status().label(), "rendering");
    (rig, first)
}

fn scrub_to(rig: &mut Rig, frame_number: i64) -> Preview {
    let plan = plan(rig, frame_number);
    rig.session
        .request_preview(&rig.state, plan, output())
        .unwrap()
}

/// The render backend finishes `id`: what the host does when `frame.done` arrives.
fn finish(rig: &mut Rig, id: RequestId, revision: u64) -> Option<SessionEvent> {
    rig.script
        .borrow_mut()
        .events
        .push_back(BackendEvent::FrameDone(frame(id, revision)));
    rig.session.pump(Duration::ZERO)
}

#[test]
fn a_scrub_lets_the_frame_in_flight_finish_and_converges_on_the_last_position() {
    let (mut rig, first) = rig_rendering_preview();
    let mut shown = Vec::new();
    // 20 playhead moves while the backend is slow: each finished frame is presented, the
    // moves in between collapse into one pending position, and nothing is ever cancelled.
    let mut in_flight = (first, rig.session.revision());
    for step in 1..=20 {
        assert_eq!(
            scrub_to(&mut rig, step * 2),
            Preview::Coalesced,
            "move {step} must not cancel the frame in flight"
        );
        if step % 5 == 0 {
            // The backend finishes the frame it was rendering.
            let Some(SessionEvent::Frame(done)) = finish(&mut rig, in_flight.0, in_flight.1) else {
                panic!("the finished frame in flight is accepted, not dropped as stale")
            };
            shown.push(done.id);
            let next = rig
                .session
                .dispatch_pending(&rig.state)
                .expect("a newer position was waiting")
                .unwrap();
            in_flight = (next, rig.session.revision());
        }
    }
    assert_eq!(
        shown.len(),
        4,
        "every frame the backend finished was presented"
    );
    assert_eq!(
        rig.session.stats.cancelled, 0,
        "no render was cancelled by the scrub"
    );
    assert_eq!(rig.session.stats.dropped_stale, 0);
    assert!(rig.script.borrow().cancelled.is_empty());

    // The last move before release, while a frame is still rendering.
    assert_eq!(scrub_to(&mut rig, 44), Preview::Coalesced);

    // Release: the frame in flight (a position from the middle of the drag) finishes, and the
    // newest position the user stopped on is rendered right after it.
    finish(&mut rig, in_flight.0, in_flight.1).expect("accepted");
    let last = rig
        .session
        .dispatch_pending(&rig.state)
        .expect("the final position was waiting")
        .unwrap();
    let submitted = rig.script.borrow().submitted.clone();
    assert_eq!(submitted.last(), Some(&(last, rig.session.revision())));
    // That last request is the final scrub position (frame 44), not an older one.
    assert!(matches!(rig.session.status(), Status::Rendering(id) if *id == last));
    let expected = plan(&rig, 44);
    assert_eq!(rig.session.active_plan(), Some(&expected));
    let revision = rig.session.revision();
    finish(&mut rig, last, revision).expect("accepted");
    assert!(
        rig.session.dispatch_pending(&rig.state).is_none(),
        "nothing left to catch up on"
    );
    assert_eq!(rig.session.stats.accepted, 6);
    assert_eq!(rig.session.stats.coalesced, 21);
}

#[test]
fn the_pending_slot_keeps_only_the_newest_position() {
    let (mut rig, first) = rig_rendering_preview();
    for frame_number in [5, 9, 12] {
        assert_eq!(scrub_to(&mut rig, frame_number), Preview::Coalesced);
    }
    finish(&mut rig, first, 1).expect("accepted");
    let next = rig.session.dispatch_pending(&rig.state).unwrap().unwrap();
    assert_eq!(
        rig.script.borrow().submitted.len(),
        2,
        "one render, not three"
    );
    // The one that was rendered is frame 12: finishing it leaves nothing pending.
    finish(&mut rig, next, 2).expect("accepted");
    assert!(rig.session.dispatch_pending(&rig.state).is_none());
}

#[test]
fn coming_back_to_the_frame_in_flight_drops_the_older_pending_position() {
    let (mut rig, first) = rig_rendering_preview();
    assert_eq!(scrub_to(&mut rig, 20), Preview::Coalesced);
    assert!(rig.session.has_pending());
    assert_eq!(scrub_to(&mut rig, 0), Preview::Coalesced);
    assert!(!rig.session.has_pending(), "frame 0 is already rendering");
    finish(&mut rig, first, 1).expect("accepted");
    assert!(rig.session.dispatch_pending(&rig.state).is_none());
}

#[test]
fn a_project_edit_supersedes_the_frame_in_flight_and_its_answer_is_dropped() {
    let (mut rig, first) = rig_rendering_preview();
    rig.state
        .execute(Command::SetTrackFlag {
            track: "v1".into(),
            flag: crate::state::timeline::TrackFlag::Hidden,
            value: true,
        })
        .unwrap();
    // The frame in flight shows a layer that is hidden now: it is obsolete, not an
    // intermediate scrub frame.
    let Preview::Submitted(second) = scrub_to(&mut rig, 4) else {
        panic!("a changed project must not coalesce")
    };
    assert_eq!(rig.script.borrow().cancelled, vec![first]);
    assert_eq!(
        finish(&mut rig, first, 1),
        Some(SessionEvent::Stale(first)),
        "the old picture is not accepted"
    );
    assert!(matches!(
        finish(&mut rig, second, 2),
        Some(SessionEvent::Frame(_))
    ));
    assert_eq!(rig.session.stats.coalesced, 0);
}

#[test]
fn another_subject_or_size_supersedes_instead_of_coalescing() {
    let (mut rig, first) = rig_rendering_preview();
    let bigger = Output {
        width: 8,
        height: 8,
        png: None,
    };
    let preview = rig
        .session
        .request_preview(&rig.state, plan(&rig, 3), bigger)
        .unwrap();
    assert!(matches!(preview, Preview::Submitted(_)));
    assert_eq!(rig.script.borrow().cancelled, vec![first]);

    let source = rig
        .state
        .plan(&PlanRequest {
            intent: PreviewIntent::Source {
                asset: Id::lit("img"),
                time: crate::time::Ratio::ZERO,
            },
            at: PlanTime::Playhead,
        })
        .unwrap();
    let preview = rig
        .session
        .request_preview(&rig.state, source, output())
        .unwrap();
    assert!(
        matches!(preview, Preview::Submitted(_)),
        "the sequence was replaced by an isolated asset"
    );
}

#[test]
fn an_explicit_request_supersedes_the_frame_in_flight_and_discards_the_pending_intent() {
    let (mut rig, first) = rig_rendering_preview();
    assert_eq!(scrub_to(&mut rig, 7), Preview::Coalesced);
    let explicit = rig
        .session
        .request_frame(&rig.state, plan(&rig, 30), output(), vec![(1, 1)])
        .unwrap();
    assert_eq!(rig.script.borrow().cancelled, vec![first]);
    assert!(
        !rig.session.has_pending(),
        "the explicit request is the newest intent"
    );
    assert_eq!(finish(&mut rig, first, 1), Some(SessionEvent::Stale(first)));
    assert!(matches!(
        finish(&mut rig, explicit, 2),
        Some(SessionEvent::Frame(_))
    ));
    assert!(rig.session.dispatch_pending(&rig.state).is_none());
}

#[test]
fn an_explicit_request_in_flight_is_never_coalesced_into() {
    let mut rig = rig();
    rig.script.borrow_mut().events.push_back(ready("img"));
    let explicit = rig
        .session
        .request_frame(&rig.state, plan(&rig, 0), output(), vec![])
        .unwrap();
    rig.session.pump(Duration::ZERO);
    let preview = scrub_to(&mut rig, 9);
    assert!(matches!(preview, Preview::Submitted(_)), "{preview:?}");
    assert_eq!(rig.script.borrow().cancelled, vec![explicit]);
}

#[test]
fn replacing_the_project_or_losing_the_context_discards_what_was_pending() {
    let (mut rig, first) = rig_rendering_preview();
    assert_eq!(scrub_to(&mut rig, 11), Preview::Coalesced);
    rig.session.reset_project();
    assert!(!rig.session.has_pending());
    assert_eq!(finish(&mut rig, first, 1), Some(SessionEvent::Stale(first)));

    let (mut rig, _) = rig_rendering_preview();
    assert_eq!(scrub_to(&mut rig, 11), Preview::Coalesced);
    rig.script
        .borrow_mut()
        .events
        .push_back(BackendEvent::ContextLost);
    assert!(
        matches!(rig.session.pump(Duration::ZERO), Some(SessionEvent::Failed(e)) if e.code == "context_lost")
    );
    assert!(
        !rig.session.has_pending(),
        "the picture and its textures are gone"
    );
    assert!(rig.session.dispatch_pending(&rig.state).is_none());
}

#[test]
fn a_failed_frame_is_reported_and_does_not_start_the_pending_one_behind_its_back() {
    let (mut rig, first) = rig_rendering_preview();
    assert_eq!(scrub_to(&mut rig, 11), Preview::Coalesced);
    rig.script
        .borrow_mut()
        .events
        .push_back(BackendEvent::FrameFailed {
            id: first,
            revision: 1,
            error: BackendError::new("render_failed", "boom"),
        });
    assert!(matches!(
        rig.session.pump(Duration::ZERO),
        Some(SessionEvent::Failed(e)) if e.code == "render_failed"
    ));
    assert!(rig.session.dispatch_pending(&rig.state).is_none());
    assert_eq!(rig.session.status().label(), "failed");
}

#[test]
fn only_interactive_frames_skip_the_readback_and_pixel_requests_never_do() {
    let mut rig = rig();
    rig.script.borrow_mut().events.push_back(ready("img"));
    let preview = rig
        .session
        .request_preview(&rig.state, plan(&rig, 0), output())
        .unwrap();
    assert!(matches!(preview, Preview::Submitted(_)));
    rig.session.pump(Duration::ZERO);
    assert_eq!(rig.script.borrow().inspected, vec![false]);

    rig.session
        .request_frame(&rig.state, plan(&rig, 1), output(), vec![])
        .unwrap();
    rig.session
        .request_frame(&rig.state, plan(&rig, 2), output(), vec![(1, 1)])
        .unwrap();
    let with_png = Output {
        png: Some("/tmp/proof.png".into()),
        ..output()
    };
    rig.session
        .request_frame(&rig.state, plan(&rig, 3), with_png, vec![])
        .unwrap();
    assert_eq!(
        rig.script.borrow().inspected,
        vec![false, true, true, true],
        "explicit requests keep the hash, the probes and the PNG"
    );

    rig.session.set_preview_inspection(true);
    rig.session
        .request_preview(&rig.state, plan(&rig, 4), output())
        .unwrap();
    assert_eq!(
        rig.script.borrow().inspected.last(),
        Some(&true),
        "measurement mode"
    );
}

#[test]
fn assets_a_finishing_frame_draws_are_not_released_by_a_coalesced_intent() {
    let (mut rig, _) = rig_rendering_preview();
    rig.session.set_retained_assets(0);
    assert_eq!(scrub_to(&mut rig, 3), Preview::Coalesced);
    assert!(
        rig.script.borrow().released.is_empty(),
        "retention applies when a request starts, never under a frame that is drawing"
    );
}

#[test]
fn tracing_is_off_by_default_and_costs_the_summary_nothing() {
    let mut rig = rig();
    rig.script.borrow_mut().events.push_back(ready("img"));
    let id = rig
        .session
        .request_frame(&rig.state, plan(&rig, 0), output(), vec![])
        .unwrap();
    rig.session.pump(Duration::ZERO);
    rig.script
        .borrow_mut()
        .events
        .push_back(BackendEvent::FrameDone(frame(id, 1)));
    rig.session.pump(Duration::ZERO);
    assert_eq!(
        rig.session.summary()["trace"],
        json!({ "enabled": false }),
        "nothing recorded, nothing reported"
    );
}

#[test]
fn a_traced_frame_is_recorded_once_and_stale_or_cancelled_ones_are_only_counted() {
    let mut rig = rig();
    rig.session.trace().set_enabled(true);
    rig.script.borrow_mut().events.push_back(ready("img"));
    let old = rig
        .session
        .request_frame(&rig.state, plan(&rig, 0), output(), vec![])
        .unwrap();
    rig.session.pump(Duration::ZERO);
    let new = rig
        .session
        .request_frame(&rig.state, plan(&rig, 30), output(), vec![])
        .unwrap();
    rig.script.borrow_mut().events.extend([
        BackendEvent::FrameDone(frame(old, 1)),
        BackendEvent::FrameDone(frame(new, 2)),
        BackendEvent::FramePresented {
            id: new,
            revision: 2,
            after_done_ms: 6.0,
        },
    ]);
    for _ in 0..3 {
        rig.session.pump(Duration::ZERO);
    }
    let trace = rig.session.summary()["trace"].clone();
    assert_eq!(
        trace["frames_total"], 1,
        "only the accepted frame is a frame"
    );
    assert_eq!(
        trace["dropped"], 2,
        "the cancelled one and its stale answer"
    );
    assert_eq!(trace["recent"][0]["id"], new);
    assert_eq!(trace["recent"][0]["presented_after_done_ms"], 6.0);
    // `presented` is a signal about a picture already accepted: it never counts as a new frame
    // nor makes the session report anything stale.
    assert_eq!(rig.session.stats.accepted, 1);
    assert_eq!(rig.session.stats.dropped_stale, 1);
}

// ---- Preview proxies: policy, attachment, quality and refine ----

fn frames_with_source(id: RequestId, revision: u64, proxy_layers: u64) -> FrameResult {
    let mut result = frame(id, revision);
    result.detail.source = Some(json!({ "quality": "preview", "proxy_layers": proxy_layers,
        "original_layers": 0 }));
    result
}

/// A project whose only clip is a container video that was decoded and, when `proxy` is set,
/// has a 540-row proxy on disk. The files are empty: the fake backend never reads them.
fn video_rig(proxy: bool) -> Rig {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("tessel-proxy-{}-{n}", std::process::id()));
    let set = write_standard_set(&dir).unwrap();
    let mut asset = Asset::probe_frame_sequence(
        &set.frames_pattern.to_string_lossy(),
        &DeclaredAsset {
            rate: crate::time::Rate::new(8, 1),
            frames: Some(set.frame_count),
            ..DeclaredAsset::default()
        },
    )
    .unwrap();
    asset.source = Source::File {
        path: dir.join("clip.mp4").to_string_lossy().into_owned(),
    };
    asset.meta.width = Some(1920);
    asset.meta.height = Some(1080);
    let frames = DecodedFrames {
        pattern: set.frames_pattern.to_string_lossy().into_owned(),
        first: 0,
        count: set.frame_count,
    };
    let proxy = proxy.then(|| {
        let folder = dir.join("proxy");
        std::fs::create_dir_all(&folder).unwrap();
        for index in 0..set.frame_count {
            std::fs::write(folder.join(format!("p{index:06}.jpg")), b"").unwrap();
        }
        ProxyFrames {
            pattern: folder.join("p%06d.jpg").to_string_lossy().into_owned(),
            first: 0,
            count: set.frame_count,
            width: 960,
            height: 540,
            format: ProxyFormat::Jpeg,
            source_frames: frames.pattern.clone(),
        }
    });
    asset.decoded = Some(Decoded {
        frames: Some(frames),
        audio: Vec::new(),
        proxy,
    });
    let mut state = EditorState::empty(&ProjectSettings::default()).unwrap();
    state
        .execute(Command::AddAsset {
            id: Some("vid".into()),
            name: "Vid".into(),
            kind: MediaKind::Video,
            asset: Box::new(asset),
            folder: None,
        })
        .unwrap();
    state
        .execute(Command::CreateTrack {
            id: Some("v1".into()),
            name: "V1".into(),
            kind: crate::state::timeline::TrackKind::Video,
        })
        .unwrap();
    state
        .execute(Command::AddClip {
            id: Some("c1".into()),
            name: None,
            track: "v1".into(),
            asset: Some("vid".into()),
            start: 0,
            duration: 24,
            source_start: None,
            text: None,
            camera: None,
            properties: vec![],
        })
        .unwrap();
    let script = Rc::new(RefCell::new(Script {
        layers: vec!["image", "video", "text", "scene", "waveform"],
        features: vec!["asset.proxy", "frame.quality"],
        ..Script::default()
    }));
    Rig {
        state,
        session: ViewportSession::new(Fake(script.clone())),
        script,
    }
}

fn scrub_video(rig: &mut Rig, frame_number: i64) -> Preview {
    let plan = plan(rig, frame_number);
    rig.session
        .request_preview(&rig.state, plan, output())
        .unwrap()
}

#[test]
fn a_scrub_asks_for_preview_quality_and_attaches_the_proxy_before_the_frame() {
    let mut rig = video_rig(true);
    let Preview::Submitted(first) = scrub_video(&mut rig, 0) else {
        panic!("the first scrub frame is submitted");
    };
    assert!(
        rig.script.borrow().proxies.is_empty(),
        "nothing is attached before the asset is loaded"
    );
    rig.script.borrow_mut().events.push_back(ready("vid"));
    rig.session.pump(Duration::ZERO);
    let script = rig.script.borrow();
    assert_eq!(script.proxies.len(), 1, "attached once, before the frame");
    let (id, descriptor) = &script.proxies[0];
    assert_eq!(id.as_str(), "vid");
    let descriptor = descriptor.as_ref().unwrap();
    assert_eq!(
        (descriptor["height"].as_u64(), descriptor["format"].as_str()),
        (Some(540), Some("jpeg"))
    );
    assert_eq!(script.qualities, vec![Quality::Preview]);
    drop(script);
    // Another position of the same video attaches nothing again.
    rig.script
        .borrow_mut()
        .events
        .push_back(BackendEvent::FrameDone(frames_with_source(first, 1, 1)));
    rig.session.pump(Duration::ZERO);
    scrub_video(&mut rig, 3);
    assert_eq!(rig.script.borrow().proxies.len(), 1);
}

#[test]
fn explicit_requests_and_pixel_requests_never_use_a_proxy() {
    let mut rig = video_rig(true);
    rig.session
        .request_frame(&rig.state, plan(&rig, 0), output(), vec![])
        .unwrap();
    rig.script.borrow_mut().events.push_back(ready("vid"));
    rig.session.pump(Duration::ZERO);
    assert_eq!(rig.script.borrow().qualities, vec![Quality::Original]);
    // A scrub that asks for a PNG is a request for pixels: original too.
    let mut with_png = output();
    with_png.png = Some("/tmp/x.png".into());
    let plan = plan(&rig, 1);
    rig.session
        .request_preview(&rig.state, plan, with_png)
        .unwrap();
    assert_eq!(
        rig.script.borrow().qualities.last(),
        Some(&Quality::Original)
    );
}

#[test]
fn the_original_policy_uses_no_proxy_and_detaches_one_that_was_attached() {
    let mut rig = video_rig(true);
    scrub_video(&mut rig, 0);
    rig.script.borrow_mut().events.push_back(ready("vid"));
    rig.session.pump(Duration::ZERO);
    assert_eq!(rig.script.borrow().proxies.len(), 1);

    rig.session
        .set_preview_policy(crate::preview::PreviewPolicy {
            mode: crate::preview::PreviewMode::Original,
            ..crate::preview::PreviewPolicy::default()
        });
    rig.state.preview_policy = rig.session.preview_policy();
    scrub_video(&mut rig, 1);
    let script = rig.script.borrow();
    assert_eq!(
        script.proxies.last(),
        Some(&(Id::lit("vid"), None)),
        "detached"
    );
    assert_eq!(script.qualities.last(), Some(&Quality::Original));
}

#[test]
fn a_runtime_without_proxy_support_is_never_sent_one() {
    let mut rig = video_rig(true);
    rig.script.borrow_mut().features.clear();
    scrub_video(&mut rig, 0);
    rig.script.borrow_mut().events.push_back(ready("vid"));
    rig.session.pump(Duration::ZERO);
    assert!(rig.script.borrow().proxies.is_empty());
    assert_eq!(
        rig.script.borrow().submitted.len(),
        1,
        "the frame still draws, from the original"
    );
}

#[test]
fn a_proxy_with_a_missing_file_is_not_attached() {
    let mut rig = video_rig(true);
    let proxy = rig
        .state
        .library
        .item("vid")
        .and_then(|item| item.asset.as_ref())
        .and_then(|asset| asset.decoded.as_ref())
        .and_then(|decoded| decoded.proxy.clone())
        .unwrap();
    let last = crate::assets::expand_pattern(&proxy.pattern, proxy.count - 1).unwrap();
    std::fs::remove_file(last).unwrap();
    scrub_video(&mut rig, 0);
    rig.script.borrow_mut().events.push_back(ready("vid"));
    rig.session.pump(Duration::ZERO);
    assert!(rig.script.borrow().proxies.is_empty());
}

#[test]
fn a_proxy_the_runtime_refuses_is_counted_and_not_sent_again() {
    let mut rig = video_rig(true);
    scrub_video(&mut rig, 0);
    rig.script.borrow_mut().events.extend([
        ready("vid"),
        BackendEvent::Proxy {
            id: Id::lit("vid"),
            state: ProxyState::Failed(BackendError::new("invalid_proxy", "wrong count")),
        },
    ]);
    rig.session.pump(Duration::ZERO);
    rig.session.pump(Duration::ZERO);
    assert_eq!(rig.session.stats.proxy_failed, 1);
    // The next frame draws (from the original, the runtime dropped it) and retries nothing.
    scrub_video(&mut rig, 2);
    assert_eq!(rig.script.borrow().proxies.len(), 1);
}

#[test]
fn a_released_asset_gets_its_proxy_again_when_it_is_reloaded() {
    let mut rig = video_rig(true);
    scrub_video(&mut rig, 0);
    rig.script.borrow_mut().events.push_back(ready("vid"));
    rig.session.pump(Duration::ZERO);
    rig.session.reset_project();
    scrub_video(&mut rig, 0);
    rig.script.borrow_mut().events.push_back(ready("vid"));
    rig.session.pump(Duration::ZERO);
    assert_eq!(rig.script.borrow().proxies.len(), 2);
}

#[test]
fn a_video_drawn_before_it_was_decoded_is_reloaded_from_its_frames_and_takes_the_proxy() {
    let mut rig = video_rig(true);
    let decoded = rig
        .state
        .library
        .item(Id::lit("vid"))
        .and_then(|item| item.asset.as_ref())
        .and_then(|asset| asset.decoded.clone())
        .unwrap();
    // The project was just opened: the first frame is drawn from the container itself.
    let container =
        std::env::temp_dir().join(format!("tessel-container-{}.mp4", std::process::id()));
    std::fs::write(&container, b"").unwrap();
    rig.state
        .library
        .items
        .iter_mut()
        .find(|item| item.id == "vid")
        .and_then(|item| item.asset.as_mut())
        .unwrap()
        .source = Source::File {
        path: container.to_string_lossy().into_owned(),
    };
    assert!(rig.state.set_decoded("vid", None));
    let Preview::Submitted(first) = scrub_video(&mut rig, 0) else {
        panic!("submitted");
    };
    rig.script.borrow_mut().events.push_back(ready("vid"));
    rig.session.pump(Duration::ZERO);
    assert!(rig.script.borrow().proxies.is_empty());
    settle(&mut rig, first, 1, 0);
    // The decode finishes; the same asset ID now has frames (and a proxy) behind it.
    assert!(rig.state.set_decoded("vid", Some(decoded)));
    let Preview::Submitted(second) = scrub_video(&mut rig, 1) else {
        panic!("submitted");
    };
    {
        let script = rig.script.borrow();
        assert_eq!(
            script.released,
            vec![Id::lit("vid")],
            "the container is let go"
        );
        assert_eq!(script.loaded, vec![Id::lit("vid"), Id::lit("vid")]);
    }
    rig.script.borrow_mut().events.push_back(ready("vid"));
    rig.session.pump(Duration::ZERO);
    assert_eq!(rig.script.borrow().proxies.len(), 1);
    settle(&mut rig, second, 2, 1);
    // Nothing changed afterwards: no more reloads while scrubbing.
    scrub_video(&mut rig, 2);
    assert_eq!(rig.script.borrow().loaded.len(), 2);
}

fn settle(rig: &mut Rig, request: RequestId, revision: u64, proxy_layers: u64) {
    rig.script
        .borrow_mut()
        .events
        .push_back(BackendEvent::FrameDone(frames_with_source(
            request,
            revision,
            proxy_layers,
        )));
    assert!(matches!(
        rig.session.pump(Duration::ZERO),
        Some(SessionEvent::Frame(_))
    ));
}

#[test]
fn a_proxy_picture_is_replaced_by_the_original_once_the_scrub_settles() {
    let mut rig = video_rig(true);
    let Preview::Submitted(first) = scrub_video(&mut rig, 2) else {
        panic!("submitted");
    };
    rig.script.borrow_mut().events.push_back(ready("vid"));
    rig.session.pump(Duration::ZERO);
    settle(&mut rig, first, 1, 1);
    assert!(rig.session.refine_pending());

    let now = Instant::now();
    assert!(
        rig.session.dispatch_refine(&rig.state, now).is_none(),
        "not before the monitor has been still"
    );
    let later = now + REFINE_DELAY + Duration::from_millis(10);
    let refined = rig
        .session
        .dispatch_refine(&rig.state, later)
        .unwrap()
        .unwrap();
    assert_eq!(
        rig.script.borrow().qualities.last(),
        Some(&Quality::Original)
    );
    assert!(!rig.session.refine_pending());
    assert_eq!(rig.session.stats.refined, 1);
    // The refined frame is a normal answer and does not ask for another refine.
    settle(&mut rig, refined, 2, 0);
    assert!(!rig.session.refine_pending());
    assert!(rig.session.dispatch_refine(&rig.state, later).is_none());
}

#[test]
fn a_new_intent_cancels_a_refine_instead_of_waiting_for_it() {
    let mut rig = video_rig(true);
    let Preview::Submitted(first) = scrub_video(&mut rig, 2) else {
        panic!("submitted");
    };
    rig.script.borrow_mut().events.push_back(ready("vid"));
    rig.session.pump(Duration::ZERO);
    settle(&mut rig, first, 1, 1);
    let later = Instant::now() + REFINE_DELAY + Duration::from_millis(10);
    let refined = rig
        .session
        .dispatch_refine(&rig.state, later)
        .unwrap()
        .unwrap();
    let Preview::Submitted(next) = scrub_video(&mut rig, 5) else {
        panic!("a refine in flight is superseded, not coalesced into");
    };
    assert!(rig.script.borrow().cancelled.contains(&refined));
    assert_ne!(next, refined);
}

#[test]
fn a_frame_drawn_from_the_original_has_nothing_to_refine() {
    let mut rig = video_rig(false);
    let Preview::Submitted(first) = scrub_video(&mut rig, 1) else {
        panic!("submitted");
    };
    rig.script.borrow_mut().events.push_back(ready("vid"));
    rig.session.pump(Duration::ZERO);
    assert!(rig.script.borrow().proxies.is_empty(), "no proxy built yet");
    settle(&mut rig, first, 1, 0);
    assert!(!rig.session.refine_pending());
}

#[test]
fn an_edit_or_an_explicit_frame_drops_the_pending_refine() {
    let mut rig = video_rig(true);
    let Preview::Submitted(first) = scrub_video(&mut rig, 2) else {
        panic!("submitted");
    };
    rig.script.borrow_mut().events.push_back(ready("vid"));
    rig.session.pump(Duration::ZERO);
    settle(&mut rig, first, 1, 1);
    assert!(rig.session.refine_pending());
    rig.session
        .request_frame(&rig.state, plan(&rig, 2), output(), vec![])
        .unwrap();
    assert!(!rig.session.refine_pending());
}

/// `rig()` plus a second still on the same track, starting at `start`: a cut the playhead has not
/// reached yet.
fn rig_with_cut(start: i64) -> Rig {
    let mut rig = rig();
    let image = rig
        .state
        .library
        .item("img")
        .unwrap()
        .asset
        .clone()
        .unwrap();
    rig.state
        .execute(Command::AddAsset {
            id: Some("next".into()),
            name: "Next".into(),
            kind: MediaKind::Still,
            asset: Box::new(image),
            folder: None,
        })
        .unwrap();
    rig.state
        .execute(Command::AddClip {
            id: Some("c2".into()),
            name: None,
            track: "v1".into(),
            asset: Some("next".into()),
            start,
            duration: 24,
            source_start: None,
            text: None,
            camera: None,
            properties: vec![],
        })
        .unwrap();
    rig
}

#[test]
fn assets_ahead_lists_clips_that_start_inside_the_window_only() {
    let mut rig = rig_with_cut(60);
    assert!(
        rig.state.assets_ahead(24).is_empty(),
        "frame 60 is past 0 + 24"
    );
    assert_eq!(rig.state.assets_ahead(60), vec![Id::lit("next")]);
    assert!(rig.state.assets_ahead(0).is_empty());
    // A clip that is already on screen is the current frame's own business, not "ahead".
    rig.state
        .execute(Command::SetPlayhead {
            frame: crate::Raw::Number(60.0),
        })
        .unwrap();
    assert!(rig.state.assets_ahead(24).is_empty());
    rig.state
        .execute(Command::SetPlayhead {
            frame: crate::Raw::Number(40.0),
        })
        .unwrap();
    assert_eq!(rig.state.assets_ahead(24), vec![Id::lit("next")]);
}

#[test]
fn assets_ahead_is_empty_for_a_source_preview() {
    let mut rig = rig_with_cut(60);
    rig.state.preview = PreviewIntent::Source {
        asset: Id::lit("img"),
        time: crate::time::Ratio::new(0, 1).unwrap(),
    };
    assert!(rig.state.assets_ahead(120).is_empty());
}

#[test]
fn prefetch_loads_ahead_without_a_frame_and_the_frame_that_needs_it_does_not_load_again() {
    let mut rig = rig_with_cut(60);
    rig.session.capabilities().unwrap();
    let ahead = vec![Id::lit("next")];
    assert_eq!(rig.session.prefetch(&rig.state, &ahead), 1);
    assert_eq!(
        rig.session.prefetch(&rig.state, &ahead),
        0,
        "already loading"
    );
    assert_eq!(rig.script.borrow().loaded, vec![Id::lit("next")]);
    assert_eq!(rig.session.status().label(), "idle", "no frame, no status");
    assert_eq!(rig.session.revision(), 0);
    assert_eq!(rig.session.stats.requests, 0);
    assert_eq!(rig.session.stats.prefetched, 1);

    // The answer arrives with nothing waiting for it.
    rig.script.borrow_mut().events.push_back(ready("next"));
    assert_eq!(
        rig.session.pump(Duration::ZERO),
        Some(SessionEvent::Progress)
    );
    assert_eq!(rig.session.loaded_assets(), vec![Id::lit("next")]);

    // The frame at the cut finds it ready: only the first clip's asset is loaded, and the frame
    // is submitted as soon as that one is ready.
    let id = rig
        .session
        .request_frame(&rig.state, plan(&rig, 60), output(), vec![])
        .unwrap();
    assert_eq!(rig.script.borrow().loaded, vec![Id::lit("next")]);
    assert_eq!(rig.script.borrow().submitted.last(), Some(&(id, 1)));
}

#[test]
fn a_traced_prefetch_says_where_each_load_spent_its_time_on_the_calling_thread() {
    let mut quiet = rig_with_cut(60);
    quiet.session.capabilities().unwrap();
    assert_eq!(quiet.session.prefetch(&quiet.state, &[Id::lit("next")]), 1);
    assert_eq!(
        quiet.session.trace().events_since(0, 16)["events"]
            .as_array()
            .map(Vec::len),
        Some(0),
        "no trace, no event"
    );

    let mut traced = rig_with_cut(60);
    traced.session.trace().set_enabled(true);
    traced.session.capabilities().unwrap();
    assert_eq!(
        traced.session.prefetch(&traced.state, &[Id::lit("next")]),
        1
    );
    let events = traced.session.trace().events_since(0, 16);
    let load = events["events"]
        .as_array()
        .and_then(|items| items.iter().find(|e| e["kind"] == "prefetch_asset"))
        .expect("a load started, an event says how long handing it over took");
    assert!(
        load["fields"].get("asset").is_none(),
        "what is loaded is told by kind and size, never by name"
    );
    assert!(load["fields"]["kind"].is_string());
    assert!(load["fields"]["fs_ms"].is_number() && load["fields"]["send_ms"].is_number());
    assert_eq!(load["fields"]["ok"], true);
}

#[test]
fn a_frame_waiting_on_a_prefetch_in_flight_does_not_start_a_second_load() {
    let mut rig = rig_with_cut(60);
    rig.session.capabilities().unwrap();
    rig.session.prefetch(&rig.state, &[Id::lit("next")]);
    let id = rig
        .session
        .request_frame(&rig.state, plan(&rig, 60), output(), vec![])
        .unwrap();
    assert_eq!(rig.session.status().label(), "loading");
    assert_eq!(rig.script.borrow().loaded, vec![Id::lit("next")]);
    rig.script
        .borrow_mut()
        .events
        .extend([ready("next"), BackendEvent::FrameDone(frame(id, 1))]);
    rig.session.pump(Duration::ZERO);
    assert_eq!(rig.session.status().label(), "rendering");
    assert!(matches!(
        rig.session.pump(Duration::ZERO),
        Some(SessionEvent::Frame(_))
    ));
}

// ---- BL129: scene MSAA by motion (playback, scrub) and the refine to the final count ----

/// A frame answer as the runtime reports a scene layer drawn with `samples` MSAA samples, of which
/// `reduced` layers are below the final count.
fn scene_frame(id: RequestId, revision: u64, samples: u64, reduced: u64) -> FrameResult {
    let mut result = frame(id, revision);
    result.detail.source = Some(json!({ "quality": "preview", "proxy_layers": 0,
        "original_layers": 0, "result_hits": 0,
        "scene_samples": samples, "reduced_scene_layers": reduced }));
    result
}

fn finish_scene(rig: &mut Rig, id: RequestId, samples: u64, reduced: u64) {
    let revision = rig.session.revision();
    rig.script
        .borrow_mut()
        .events
        .push_back(BackendEvent::FrameDone(scene_frame(
            id, revision, samples, reduced,
        )));
    assert!(matches!(
        rig.session.pump(Duration::ZERO),
        Some(SessionEvent::Frame(_))
    ));
}

/// `rig_with_cut` plus a third still ("later") that starts at `start`, with the same file.
fn rig_with_two_cuts(first: i64, start: i64) -> Rig {
    let mut rig = rig_with_cut(first);
    let image = rig
        .state
        .library
        .item("img")
        .unwrap()
        .asset
        .clone()
        .unwrap();
    rig.state
        .execute(Command::AddAsset {
            id: Some("later".into()),
            name: "Later".into(),
            kind: MediaKind::Still,
            asset: Box::new(image),
            folder: None,
        })
        .unwrap();
    rig.state
        .execute(Command::AddClip {
            id: Some("c3".into()),
            name: None,
            track: "v1".into(),
            asset: Some("later".into()),
            start,
            duration: 24,
            source_start: None,
            text: None,
            camera: None,
            properties: vec![],
        })
        .unwrap();
    rig
}

#[test]
fn a_burst_of_prefetches_stops_at_the_byte_budget_and_a_frame_still_loads_what_was_skipped() {
    let mut rig = rig_with_two_cuts(60, 90);
    rig.session.capabilities().unwrap();
    let file_bytes: u64 = AssetDescriptor::from_item(rig.state.library.item("img").unwrap(), None)
        .unwrap()
        .files()
        .iter()
        .map(|file| std::fs::metadata(file).unwrap().len())
        .sum();
    assert!(file_bytes > 0);
    // Room for exactly one of the two.
    rig.session.set_prefetch_budget(file_bytes);
    let both = vec![Id::lit("next"), Id::lit("later")];
    assert_eq!(rig.session.prefetch(&rig.state, &both), 1);
    assert_eq!(rig.session.stats.prefetch_skipped, 1);
    assert_eq!(rig.script.borrow().loaded, vec![Id::lit("next")]);

    // Skipped is not failed: the frame that needs it loads it on demand.
    rig.script.borrow_mut().events.push_back(ready("next"));
    rig.session.pump(Duration::ZERO);
    rig.session
        .request_frame(&rig.state, plan(&rig, 90), output(), vec![])
        .unwrap();
    assert!(
        rig.script.borrow().loaded.contains(&Id::lit("later")),
        "the frame loads the asset the prefetch skipped"
    );
}

#[test]
fn the_first_prefetch_always_fits_and_a_used_one_frees_the_budget() {
    let mut rig = rig_with_two_cuts(60, 90);
    rig.session.capabilities().unwrap();
    rig.session.set_prefetch_budget(1);
    // One scene bigger than the whole budget still prefetches: the budget limits a burst.
    assert_eq!(rig.session.prefetch(&rig.state, &[Id::lit("next")]), 1);
    assert_eq!(rig.session.prefetch(&rig.state, &[Id::lit("later")]), 0);
    assert_eq!(rig.session.stats.prefetch_skipped, 1);
    // A frame that uses the prefetched asset turns it into an ordinary one: the budget is free again.
    rig.script.borrow_mut().events.push_back(ready("next"));
    rig.session.pump(Duration::ZERO);
    rig.session
        .request_frame(&rig.state, plan(&rig, 60), output(), vec![])
        .unwrap();
    assert_eq!(rig.session.prefetch(&rig.state, &[Id::lit("later")]), 1);
}

fn file_bytes_of_img(rig: &Rig) -> u64 {
    AssetDescriptor::from_item(rig.state.library.item("img").unwrap(), None)
        .unwrap()
        .files()
        .iter()
        .map(|file| std::fs::metadata(file).unwrap().len())
        .sum()
}

/// Draws frame `at` and completes the loads it waits for, as the monitor does while playing.
fn show(rig: &mut Rig, at: i64, assets: &[&str]) {
    let id = rig
        .session
        .request_frame(&rig.state, plan(rig, at), output(), vec![])
        .unwrap();
    for asset in assets {
        rig.script.borrow_mut().events.push_back(ready(asset));
        rig.session.pump(Duration::ZERO);
    }
    settle(rig, id, rig.session.revision(), 0);
}

#[test]
fn past_the_byte_budget_the_least_recently_used_heavy_asset_is_released_first() {
    let mut rig = rig_with_two_cuts(60, 90);
    rig.session.capabilities().unwrap();
    rig.session.min_evict_bytes = 0;
    let file = file_bytes_of_img(&rig);
    rig.session.set_retained_bytes(2 * file);
    show(&mut rig, 0, &["img"]);
    show(&mut rig, 60, &["next"]);
    assert!(rig.script.borrow().released.is_empty(), "two fit");
    // The third asset takes the total past the budget: `img` was used longest ago.
    rig.session
        .request_frame(&rig.state, plan(&rig, 90), output(), vec![])
        .unwrap();
    assert_eq!(rig.script.borrow().released, vec![Id::lit("img")]);
    assert_eq!(rig.session.stats.evicted, 1);
    assert!(rig.script.borrow().loaded.contains(&Id::lit("later")));
    assert_eq!(rig.session.loaded_assets(), vec![Id::lit("next")]);
}

#[test]
fn the_byte_budget_never_releases_what_the_frame_needs_or_a_prefetch_holds() {
    let mut rig = rig_with_two_cuts(60, 90);
    rig.session.capabilities().unwrap();
    rig.session.min_evict_bytes = 0;
    rig.session.set_retained_bytes(1);
    show(&mut rig, 0, &["img"]);
    assert_eq!(rig.session.prefetch(&rig.state, &[Id::lit("later")]), 1);
    rig.script.borrow_mut().events.push_back(ready("later"));
    rig.session.pump(Duration::ZERO);
    rig.session
        .request_frame(&rig.state, plan(&rig, 60), output(), vec![])
        .unwrap();
    // Only `img` is neither required (`next`) nor prefetched (`later`).
    assert_eq!(rig.script.borrow().released, vec![Id::lit("img")]);
    assert!(rig.session.assets.contains_key(&Id::lit("later")));
    assert!(rig.session.assets.contains_key(&Id::lit("next")));
}

#[test]
fn without_a_byte_budget_or_for_light_assets_nothing_is_released() {
    let mut rig = rig_with_two_cuts(60, 90);
    rig.session.capabilities().unwrap();
    show(&mut rig, 0, &["img"]);
    show(&mut rig, 60, &["next"]);
    show(&mut rig, 90, &["later"]);
    assert!(rig.script.borrow().released.is_empty(), "no budget set");
    // A budget alone does not release assets under the heavy threshold: they free nothing worth a reload.
    rig.session.set_retained_bytes(1);
    rig.session
        .request_frame(&rig.state, plan(&rig, 0), output(), vec![])
        .unwrap();
    assert!(rig.script.borrow().released.is_empty());
    assert_eq!(rig.session.stats.evicted, 0);
}

#[test]
fn a_prefetch_is_best_effort_and_never_a_failure() {
    let mut rig = rig_with_cut(60);
    // Before the runtime announced itself there is nothing to load into.
    assert_eq!(rig.session.prefetch(&rig.state, &[Id::lit("next")]), 0);
    rig.session.capabilities().unwrap();
    assert_eq!(
        rig.session
            .prefetch(&rig.state, &[Id::lit("not-in-the-library")]),
        0
    );
    rig.script.borrow_mut().fail_load = Some(BackendError::new("asset_unreadable", "gone"));
    assert_eq!(rig.session.prefetch(&rig.state, &[Id::lit("next")]), 0);
    assert_eq!(rig.session.status().label(), "idle");
    // A runtime that refuses the load later is ignored unless a frame needs the asset.
    rig.script.borrow_mut().fail_load = None;
    assert_eq!(rig.session.prefetch(&rig.state, &[Id::lit("next")]), 1);
    rig.script
        .borrow_mut()
        .events
        .push_back(BackendEvent::Asset {
            id: Id::lit("next"),
            status: AssetStatus::Failed(BackendError::new("decode_failed", "bad")),
            echo: None,
        });
    assert_eq!(
        rig.session.pump(Duration::ZERO),
        Some(SessionEvent::Progress)
    );
    assert_eq!(rig.session.status().label(), "idle");
    assert!(rig.session.loaded_assets().is_empty());
}

/// A frame answer of a splat scene drawn at the final MSAA with `stale` layers painted in the order of
/// an earlier camera's sort (BL130).
fn sorted_frame(id: RequestId, revision: u64, stale: u64) -> FrameResult {
    let mut result = frame(id, revision);
    result.detail.source = Some(json!({ "quality": "preview", "proxy_layers": 0,
        "original_layers": 0, "result_hits": 0, "scene_samples": 4,
        "reduced_scene_layers": 0, "stale_sort_layers": stale }));
    result
}

fn finish_sorted(rig: &mut Rig, id: RequestId, stale: u64) {
    let revision = rig.session.revision();
    rig.script
        .borrow_mut()
        .events
        .push_back(BackendEvent::FrameDone(sorted_frame(id, revision, stale)));
    assert!(matches!(
        rig.session.pump(Duration::ZERO),
        Some(SessionEvent::Frame(_))
    ));
}

fn finish_at_current_revision(rig: &mut Rig, id: RequestId) -> Option<SessionEvent> {
    let revision = rig.session.revision();
    finish(rig, id, revision)
}

#[test]
fn playback_frames_ask_for_the_preview_quality_and_everything_else_for_the_final_one() {
    let (mut rig, first) = rig_rendering_preview();
    assert_eq!(rig.script.borrow().qualities, vec![Quality::Preview]);
    finish_scene(&mut rig, first, 2, 1);
    // An explicit frame (`render.frame`, a proof, export) is the final quality by definition.
    rig.session
        .request_frame(&rig.state, plan(&rig, 3), output(), vec![])
        .unwrap();
    assert_eq!(
        rig.script.borrow().qualities.last(),
        Some(&Quality::Original)
    );
    // So is a frame that asks for pixels, even when the monitor is the one asking.
    let with_png = Output {
        png: Some("/tmp/x.png".into()),
        ..output()
    };
    rig.session
        .request_preview(&rig.state, plan(&rig, 4), with_png)
        .unwrap();
    assert_eq!(
        rig.script.borrow().qualities.last(),
        Some(&Quality::Original)
    );
}

#[test]
fn a_scene_drawn_with_fewer_samples_is_refined_when_the_playhead_stops() {
    let (mut rig, first) = rig_rendering_preview();
    let mut in_flight = (first, rig.session.revision());
    // Playback: a new frame every tick; each one is drawn reduced and none is refined while the
    // playhead keeps moving (a refine is only due after REFINE_DELAY without a new intent).
    for step in 1..=6 {
        finish_scene(&mut rig, in_flight.0, 2, 1);
        assert!(
            rig.session.refine_pending(),
            "step {step}: a reduced picture is on screen"
        );
        let Preview::Submitted(next) = scrub_to(&mut rig, step * 2) else {
            panic!("nothing in flight: the next frame is submitted")
        };
        assert!(
            !rig.session.refine_pending(),
            "a new intent drops the refine of the picture it replaces"
        );
        in_flight = (next, rig.session.revision());
    }
    assert_eq!(rig.session.stats.refined, 0);
    finish_scene(&mut rig, in_flight.0, 2, 1);
    assert!(rig.session.refine_pending());

    // Stop: not before the monitor has been still, then one final-quality frame of the very
    // position on screen, without the user asking.
    let now = Instant::now();
    assert!(rig.session.dispatch_refine(&rig.state, now).is_none());
    let later = now + REFINE_DELAY + Duration::from_millis(10);
    let on_screen = rig.session.presented;
    let refined = rig
        .session
        .dispatch_refine(&rig.state, later)
        .expect("the stopped monitor refines")
        .unwrap();
    assert_eq!(
        rig.script.borrow().qualities.last(),
        Some(&Quality::Original)
    );
    assert_eq!(rig.session.stats.refined, 1);
    assert_eq!(
        rig.session.active_plan().and_then(|plan| plan.frame),
        on_screen.and_then(|(_, frame)| frame),
        "the refine redraws the position that is on screen"
    );
    // The refined picture is the final one: nothing left to refine, nothing more is drawn.
    finish_scene(&mut rig, refined, 4, 0);
    assert!(!rig.session.refine_pending());
    assert!(rig.session.dispatch_refine(&rig.state, later).is_none());
    assert!(!rig.session.has_pending());
}

#[test]
fn a_new_intent_during_the_refine_supersedes_it_and_the_monitor_never_stays_reduced() {
    let (mut rig, first) = rig_rendering_preview();
    finish_scene(&mut rig, first, 2, 1);
    let later = Instant::now() + REFINE_DELAY + Duration::from_millis(10);
    let refined = rig
        .session
        .dispatch_refine(&rig.state, later)
        .unwrap()
        .unwrap();
    // Playback resumes while the final-quality frame is still drawing: it is cancelled, not waited for.
    let Preview::Submitted(next) = scrub_to(&mut rig, 5) else {
        panic!("a refine in flight is superseded, not coalesced into");
    };
    assert!(rig.script.borrow().cancelled.contains(&refined));
    // That frame is reduced again, so stopping again refines again.
    finish_scene(&mut rig, next, 2, 1);
    assert!(rig.session.refine_pending());
    // The new intent stamped its own time: wait out the delay from *now*, not from before it.
    let after_stop = Instant::now() + REFINE_DELAY + Duration::from_millis(10);
    assert!(
        rig.session
            .dispatch_refine(&rig.state, after_stop)
            .is_some()
    );
    assert_eq!(rig.session.stats.refined, 2);
}

// ---- BL130: the Spark sort of a moving playhead and the refine to the exact order ----

#[test]
fn splats_painted_in_an_earlier_cameras_order_are_refined_when_the_playhead_stops() {
    let (mut rig, first) = rig_rendering_preview();
    // Full MSAA: the stale order is the only thing that makes this picture differ from the final one.
    finish_sorted(&mut rig, first, 1);
    assert!(
        rig.session.refine_pending(),
        "a picture with a reused order is on screen"
    );
    let later = Instant::now() + REFINE_DELAY + Duration::from_millis(10);
    let on_screen = rig.session.presented;
    let refined = rig
        .session
        .dispatch_refine(&rig.state, later)
        .expect("the stopped monitor refines")
        .unwrap();
    assert_eq!(
        rig.script.borrow().qualities.last(),
        Some(&Quality::Original),
        "the refine waits for the sort of its own camera"
    );
    assert_eq!(
        rig.session.active_plan().and_then(|plan| plan.frame),
        on_screen.and_then(|(_, frame)| frame),
        "the position on screen is the one redrawn"
    );
    // The exact picture: nothing is left to refine.
    finish_sorted(&mut rig, refined, 0);
    assert!(!rig.session.refine_pending());
    assert!(rig.session.dispatch_refine(&rig.state, later).is_none());
}

#[test]
fn a_frame_whose_order_was_its_own_has_nothing_to_refine() {
    let (mut rig, first) = rig_rendering_preview();
    // The first frame of a scene (or a cut) waits for its own sort: preview quality, exact order.
    finish_sorted(&mut rig, first, 0);
    assert!(!rig.session.refine_pending());
}

#[test]
fn an_explicit_frame_is_never_a_candidate_for_the_sort_refine() {
    let (mut rig, first) = rig_rendering_preview();
    finish_sorted(&mut rig, first, 2);
    assert!(rig.session.refine_pending());
    // `render.frame`, export and probes ask for the final quality: even an answer that claims a stale
    // order (it cannot, the runtime waits for the sort) is not something the monitor would refine.
    let id = rig
        .session
        .request_frame(&rig.state, plan(&rig, 3), output(), vec![])
        .unwrap();
    assert_eq!(
        rig.script.borrow().qualities.last(),
        Some(&Quality::Original)
    );
    assert!(!rig.session.refine_pending());
    let revision = rig.session.revision();
    rig.script
        .borrow_mut()
        .events
        .push_back(BackendEvent::FrameDone(sorted_frame(id, revision, 2)));
    rig.session.pump(Duration::ZERO);
    assert!(!rig.session.refine_pending());
}

#[test]
fn a_scene_at_the_final_samples_has_nothing_to_refine() {
    let (mut rig, first) = rig_rendering_preview();
    // Preview quality, but nothing was reduced: a runtime asked for 4 samples, a host that lowers
    // every frame, or a frame without a scene layer. The monitor does not redraw for nothing.
    finish_scene(&mut rig, first, 4, 0);
    assert!(!rig.session.refine_pending());
    let Preview::Submitted(next) = scrub_to(&mut rig, 2) else {
        panic!("submitted");
    };
    finish_scene(&mut rig, next, 0, 0);
    assert!(!rig.session.refine_pending());
    // A runtime that predates the field reports nothing: never reduced.
    let Preview::Submitted(last) = scrub_to(&mut rig, 3) else {
        panic!("submitted");
    };
    assert!(matches!(
        finish_at_current_revision(&mut rig, last),
        Some(SessionEvent::Frame(_))
    ));
    assert!(!rig.session.refine_pending());
}

#[test]
fn an_explicit_frame_or_an_edit_drops_the_refine_of_a_reduced_picture() {
    let (mut rig, first) = rig_rendering_preview();
    finish_scene(&mut rig, first, 2, 1);
    assert!(rig.session.refine_pending());
    let explicit = rig
        .session
        .request_frame(&rig.state, plan(&rig, 2), output(), vec![])
        .unwrap();
    assert!(!rig.session.refine_pending());
    // The explicit answer is the final quality and is never a reason to refine.
    finish_scene(&mut rig, explicit, 4, 0);
    assert!(!rig.session.refine_pending());
}

#[test]
fn the_original_policy_draws_the_final_samples_while_playing() {
    let mut rig = rig();
    rig.session.set_preview_policy(PreviewPolicy {
        mode: crate::preview::PreviewMode::Original,
        ..PreviewPolicy::default()
    });
    rig.script.borrow_mut().events.push_back(ready("img"));
    rig.session
        .request_preview(&rig.state, plan(&rig, 0), output())
        .unwrap();
    rig.session.pump(Duration::ZERO);
    assert_eq!(rig.script.borrow().qualities, vec![Quality::Original]);
}

/// `rig_with_cut`, but the clip that starts at `start` is a scene (and its splat), so the plan at the cut has a
/// scene layer and requires `scn` and `cloud`.
fn rig_with_scene_cut(start: i64) -> Rig {
    let mut rig = rig();
    rig.script.borrow_mut().features = vec!["scene.warm"];
    // One directory per call: the tests that use it run in parallel, and writing a fixture set truncates
    // the files another test is probing.
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "tessel-session-scene-{}-{start}-{n}",
        std::process::id()
    ));
    let set = write_standard_set(&dir).unwrap();
    for (id, kind, path) in [
        ("cloud", MediaKind::Splat, &set.splat),
        ("scn", MediaKind::Scene, &set.scene),
    ] {
        let asset = Asset::probe(kind, &path.to_string_lossy(), &Declared::default()).unwrap();
        rig.state
            .execute(Command::AddAsset {
                id: Some(id.into()),
                name: id.into(),
                kind,
                asset: Box::new(asset),
                folder: None,
            })
            .unwrap();
    }
    rig.state
        .execute(Command::AddClip {
            id: Some("cs".into()),
            name: None,
            track: "v1".into(),
            asset: Some("scn".into()),
            start,
            duration: 24,
            source_start: None,
            text: None,
            camera: None,
            properties: vec![],
        })
        .unwrap();
    rig
}

#[test]
fn a_cut_into_a_scene_is_warmed_once_its_assets_are_loaded_and_not_before() {
    let mut rig = rig_with_scene_cut(60);
    rig.session.capabilities().unwrap();
    let plans = rig.state.plans_ahead(60);
    assert_eq!(plans.len(), 1);
    let ahead = rig.state.assets_ahead(60);
    assert_eq!(ahead.len(), 2, "the scene and its splat: {ahead:?}");

    // Nothing is loaded yet: there is nothing to sort.
    assert_eq!(rig.session.warm(&plans), 0);
    assert_eq!(rig.session.prefetch(&rig.state, &ahead), 2);
    assert_eq!(rig.session.warm(&plans), 0, "still loading");
    // One of the two is not enough: a warm never makes the runtime wait for a file.
    rig.script.borrow_mut().events.push_back(ready("scn"));
    rig.session.pump(Duration::ZERO);
    assert_eq!(rig.session.warm(&plans), 0);
    rig.script.borrow_mut().events.push_back(ready("cloud"));
    rig.session.pump(Duration::ZERO);

    assert_eq!(rig.session.warm(&plans), 1);
    assert_eq!(rig.script.borrow().warmed.len(), 1);
    assert_eq!(rig.session.stats.warmed, 1);
    assert_eq!(rig.session.warm(&plans), 0, "once per cut");
    // Nothing about a warm is a frame: no request, no status, no revision.
    assert_eq!(rig.session.stats.requests, 0);
    assert_eq!(rig.session.status().label(), "idle");
    assert_eq!(rig.session.revision(), 0);
}

#[test]
fn a_cut_is_warmed_again_after_it_was_passed_released_or_the_context_was_lost() {
    let mut rig = rig_with_scene_cut(60);
    rig.session.capabilities().unwrap();
    let plans = rig.state.plans_ahead(60);
    let ahead = rig.state.assets_ahead(60);
    rig.session.prefetch(&rig.state, &ahead);
    for id in ["scn", "cloud"] {
        rig.script.borrow_mut().events.push_back(ready(id));
        rig.session.pump(Duration::ZERO);
    }
    assert_eq!(rig.session.warm(&plans), 1);

    // The playhead passed the cut (it is no longer ahead) and came back before it.
    assert_eq!(rig.session.warm(&[]), 0);
    assert_eq!(rig.session.warm(&plans), 1, "a later pass warms it again");

    // Its assets were released: their stage went with them.
    rig.session.release_except(&HashSet::new());
    rig.session.prefetch(&rig.state, &ahead);
    for id in ["scn", "cloud"] {
        rig.script.borrow_mut().events.push_back(ready(id));
        rig.session.pump(Duration::ZERO);
    }
    assert_eq!(rig.session.warm(&plans), 1);

    // A lost context took every stage.
    rig.script
        .borrow_mut()
        .events
        .push_back(BackendEvent::ContextLost);
    rig.session.pump(Duration::ZERO);
    rig.session.prefetch(&rig.state, &ahead);
    for id in ["scn", "cloud"] {
        rig.script.borrow_mut().events.push_back(ready(id));
        rig.session.pump(Duration::ZERO);
    }
    assert_eq!(rig.session.warm(&plans), 1);
    assert_eq!(rig.script.borrow().warmed.len(), 4);
}

#[test]
fn a_runtime_without_the_warm_and_a_cut_without_a_scene_are_never_sent_one() {
    // Still image cut, runtime that announces it.
    let mut rig = rig_with_cut(60);
    rig.script.borrow_mut().features = vec!["scene.warm"];
    rig.session.capabilities().unwrap();
    let plans = rig.state.plans_ahead(60);
    rig.session
        .prefetch(&rig.state, &rig.state.assets_ahead(60));
    rig.script.borrow_mut().events.push_back(ready("next"));
    rig.session.pump(Duration::ZERO);
    assert_eq!(rig.session.warm(&plans), 0, "an image has no sort");

    // Scene cut, runtime that predates the warm.
    let mut rig = rig_with_scene_cut(60);
    rig.script.borrow_mut().features = vec![];
    rig.session.capabilities().unwrap();
    let plans = rig.state.plans_ahead(60);
    rig.session
        .prefetch(&rig.state, &rig.state.assets_ahead(60));
    for id in ["scn", "cloud"] {
        rig.script.borrow_mut().events.push_back(ready(id));
        rig.session.pump(Duration::ZERO);
    }
    assert_eq!(rig.session.warm(&plans), 0);
    assert!(rig.script.borrow().warmed.is_empty());
}

#[test]
fn a_cut_is_never_warmed_when_the_host_turned_the_warm_off() {
    let mut rig = rig_with_scene_cut(60);
    rig.session.set_warm(false);
    rig.session.capabilities().unwrap();
    let plans = rig.state.plans_ahead(60);
    let ahead = rig.state.assets_ahead(60);
    rig.session.prefetch(&rig.state, &ahead);
    for id in ["scn", "cloud"] {
        rig.script.borrow_mut().events.push_back(ready(id));
        rig.session.pump(Duration::ZERO);
    }
    assert_eq!(
        rig.session.warm(&plans),
        0,
        "the runtime announces it, the host does not use it"
    );
    assert!(rig.script.borrow().warmed.is_empty());
    assert_eq!(rig.session.stats.warmed, 0);
    // The same session, switched on, warms the same cut: nothing else differed.
    rig.session.set_warm(true);
    assert_eq!(rig.session.warm(&plans), 1);
}

fn cancelled_load(id: &str) -> BackendEvent {
    BackendEvent::Asset {
        id: Id::lit(id),
        status: AssetStatus::Failed(BackendError::new(
            "cancelled",
            "Asset was released while loading",
        )),
        echo: None,
    }
}

#[test]
fn a_frame_that_timed_out_gives_up_its_pending_load_and_asks_for_it_again() {
    let mut rig = rig();
    let first = rig
        .session
        .request_frame(&rig.state, plan(&rig, 0), output(), vec![])
        .unwrap();
    assert_eq!(rig.script.borrow().loaded, vec![Id::lit("img")]);
    assert_eq!(rig.session.status().label(), "loading");

    // The host gave up on the frame while the asset was still loading.
    rig.session.cancel_timed_out();
    assert_eq!(
        rig.script.borrow().released,
        vec![Id::lit("img")],
        "the runtime ends that load"
    );
    assert_eq!(rig.session.stats.reloaded, 1);
    assert_eq!(rig.script.borrow().cancelled, vec![first]);

    // The next request loads the asset anew instead of waiting for the answer that never came.
    let second = rig
        .session
        .request_frame(&rig.state, plan(&rig, 0), output(), vec![])
        .unwrap();
    assert_eq!(
        rig.script.borrow().loaded,
        vec![Id::lit("img"), Id::lit("img")]
    );

    // The runtime reports the old load as cancelled: that is not the answer of the new one.
    rig.script
        .borrow_mut()
        .events
        .push_back(cancelled_load("img"));
    assert_eq!(
        rig.session.pump(Duration::ZERO),
        Some(SessionEvent::Progress)
    );
    assert_eq!(
        rig.session.status().label(),
        "loading",
        "the new load and its frame are still waiting"
    );
    assert!(rig.script.borrow().submitted.is_empty());

    rig.script
        .borrow_mut()
        .events
        .extend([ready("img"), BackendEvent::FrameDone(frame(second, 2))]);
    assert_eq!(
        rig.session.pump(Duration::ZERO),
        Some(SessionEvent::Progress)
    );
    assert_eq!(rig.session.status().label(), "rendering");
    assert!(matches!(
        rig.session.pump(Duration::ZERO),
        Some(SessionEvent::Frame(_))
    ));
}

#[test]
fn a_plain_cancel_a_finished_asset_and_a_prefetch_are_never_given_up() {
    // A plain cancel keeps waiting for the same load: the first timeout is a slow load, not a stuck one.
    let mut rig = rig();
    rig.session
        .request_frame(&rig.state, plan(&rig, 0), output(), vec![])
        .unwrap();
    rig.session.cancel();
    assert!(rig.script.borrow().released.is_empty());
    rig.session
        .request_frame(&rig.state, plan(&rig, 0), output(), vec![])
        .unwrap();
    assert_eq!(
        rig.script.borrow().loaded.len(),
        1,
        "the same load is waited for"
    );

    // An asset that finished loading is not reloaded because a later frame timed out.
    rig.script.borrow_mut().events.push_back(ready("img"));
    rig.session.pump(Duration::ZERO);
    rig.session.cancel_timed_out();
    assert!(rig.script.borrow().released.is_empty());
    assert_eq!(rig.session.stats.reloaded, 0);

    // A load nobody's frame waits for (a prefetch) is not given up with someone else's frame.
    let mut rig = rig_with_cut(60);
    rig.session
        .request_frame(&rig.state, plan(&rig, 0), output(), vec![])
        .unwrap();
    rig.session
        .prefetch(&rig.state, &rig.state.assets_ahead(60));
    let before = rig.script.borrow().loaded.len();
    assert!(
        before >= 2,
        "the frame's asset and the prefetched one: {before}"
    );
    rig.session.cancel_timed_out();
    assert_eq!(
        rig.script.borrow().released,
        vec![Id::lit("img")],
        "only what the frame was waiting for"
    );
}

#[test]
fn a_cancelled_load_nobody_replaced_is_still_just_progress() {
    // An old load the session released and no frame asked for again: its `cancelled` changes nothing.
    let mut rig = rig();
    rig.script
        .borrow_mut()
        .events
        .push_back(cancelled_load("img"));
    assert_eq!(
        rig.session.pump(Duration::ZERO),
        Some(SessionEvent::Progress)
    );
    assert_eq!(rig.session.status().label(), "idle");
}

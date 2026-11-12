use super::*;

#[test]
fn decodes_frames_and_rejects_other_versions() {
    let done = json!({"v":1,"type":"frame.done","id":7,"revision":3,"width":4,"height":2,
        "hash":"abcd","mean":[1,2,3,255],"probes":[[1,2,3,4]],"png":null,
        "timings":{"render_ms":1.5,"total_ms":2.5}});
    let BackendEvent::FrameDone(frame) = decode_event(&done).unwrap() else {
        panic!("expected a frame");
    };
    assert_eq!((frame.id, frame.revision, frame.width), (7, 3, 4));
    assert_eq!(frame.probes, vec![[1, 2, 3, 4]]);
    assert_eq!(frame.render_ms, 1.5);
    let wrong = json!({"v":2,"type":"ready"});
    assert_eq!(decode_event(&wrong).unwrap_err().code, "version_mismatch");
    assert_eq!(
        decode_event(&json!({"v":1,"type":"nope"}))
            .unwrap_err()
            .code,
        "protocol_error"
    );
    let failed = json!({"v":1,"type":"frame.failed","id":1,"revision":1,
        "error":{"code":"render_failed","message":"boom"}});
    assert!(matches!(
        decode_event(&failed).unwrap(),
        BackendEvent::FrameFailed { id: 1, .. }
    ));
}

#[test]
fn asset_states_cover_loading_ready_and_failed() {
    let ready = json!({"v":1,"type":"asset.state","id":"a","state":"ready","meta":{"width":2}});
    assert!(matches!(
        decode_event(&ready).unwrap(),
        BackendEvent::Asset {
            status: AssetStatus::Ready(_),
            ..
        }
    ));
    let failed = json!({"v":1,"type":"asset.state","id":"a","state":"failed",
        "error":{"code":"decode_failed","message":"bad png"}});
    let BackendEvent::Asset {
        status: AssetStatus::Failed(error),
        ..
    } = decode_event(&failed).unwrap()
    else {
        panic!("expected a failure");
    };
    assert_eq!(error.code, "decode_failed");
    assert!(
        decode_event(&json!({"v":1,"type":"asset.state","id":"bad id","state":"ready"})).is_err()
    );
}

fn scene_load(slot: &str, asset: &str, generation: u64) -> Value {
    json!({"v":1,"type":"asset.load","asset":{"id":slot,"kind":"still",
        "source":{"type":"file","path":"/a.png"},"meta":{},
        "scene":{"asset_id":asset,"revision_id":format!("{asset}-r1"),
                 "content_hash":format!("h-{asset}"),"generation":generation}}})
}

fn echo(slot: &str, state: &str, request: u64, asset: &str, generation: u64) -> Value {
    json!({"v":1,"type":"asset.state","id":slot,"state":state,"request":request,
        "scene":{"asset_id":asset,"revision_id":format!("{asset}-r1"),
                 "content_hash":format!("h-{asset}"),"generation":generation}})
}

#[test]
fn a_stamped_load_is_answered_only_by_the_echo_of_its_own_request() {
    let mut old = scene_load("slot", "red", 1);
    let old_identity = stamp_asset_load(&mut old, 7);
    let mut new = scene_load("slot", "green", 1);
    let new_identity = stamp_asset_load(&mut new, 8);
    assert_eq!(old["asset"]["request"], 7);
    assert_eq!(new_identity.request, 8);

    let old_ready = echo("slot", "ready", 7, "red", 1);
    assert!(old_identity.answered_by(&old_ready));
    assert!(
        !new_identity.answered_by(&old_ready),
        "the late answer of the load that held the slot never confirms the load that replaced it"
    );
    assert!(new_identity.answered_by(&echo("slot", "ready", 8, "green", 1)));
}

#[test]
fn the_same_picture_loaded_again_into_a_slot_is_a_different_request() {
    let mut first = scene_load("slot", "red", 1);
    let first_identity = stamp_asset_load(&mut first, 1);
    let mut again = scene_load("slot", "red", 1);
    let again_identity = stamp_asset_load(&mut again, 2);
    let first_ready = echo("slot", "ready", 1, "red", 1);
    assert!(first_identity.answered_by(&first_ready));
    assert!(!again_identity.answered_by(&first_ready));
}

#[test]
fn a_missing_or_partial_echo_answers_nothing_once_the_feature_is_negotiated() {
    let mut load = scene_load("slot", "red", 1);
    let identity = stamp_asset_load(&mut load, 3);
    assert!(!identity.answered_by(&json!({"type":"asset.state","id":"slot","state":"ready"})));
    assert!(
        !identity
            .answered_by(&json!({"type":"asset.state","id":"slot","state":"ready","request":3}))
    );
    assert!(!identity.answered_by(&echo("slot", "ready", 3, "red", 2)));
    assert!(!identity.answered_by(&echo("slot", "ready", 3, "blue", 1)));
    let mut changed = echo("slot", "ready", 3, "red", 1);
    changed["scene"]["content_hash"] = json!("another");
    assert!(!identity.answered_by(&changed));
}

#[test]
fn a_load_without_a_scene_identity_is_matched_on_the_request_alone() {
    let mut load = json!({"v":1,"type":"asset.load","asset":{"id":"a","kind":"still",
        "source":{"type":"file","path":"/a.png"},"meta":{}}});
    let identity = stamp_asset_load(&mut load, 5);
    assert_eq!(identity.scene, None);
    assert!(identity.answered_by(&json!({"type":"asset.state","id":"a","request":5})));
    assert!(!identity.answered_by(&json!({"type":"asset.state","id":"a","request":6})));
}

#[test]
fn decode_carries_the_echo_of_an_asset_state_and_nothing_else() {
    // The echo is the identity fields as received; a path or other field never travels with it.
    let mut message = echo("slot", "ready", 9, "red", 1);
    message["meta"] = json!({"width": 2});
    message["source"] = json!({"path": "/secret.png"});
    let BackendEvent::Asset { echo, status, .. } = decode_event(&message).unwrap() else {
        panic!("an asset event");
    };
    assert_eq!(status, AssetStatus::Ready(json!({"width": 2})));
    let echo = echo.expect("the identity was echoed");
    assert_eq!(
        echo,
        json!({"request": 9, "scene": {"asset_id":"red","revision_id":"red-r1",
            "content_hash":"h-red","generation":1}})
    );
    // What was decoded is judged by the same pure rule as the raw message.
    let mut load = scene_load("slot", "red", 1);
    let identity = stamp_asset_load(&mut load, 9);
    assert!(identity.answered_by(&echo));
    assert!(!stamp_asset_load(&mut load, 10).answered_by(&echo));
}

#[test]
fn decode_of_a_runtime_without_the_echo_has_none() {
    for message in [
        json!({"v":1,"type":"asset.state","id":"slot","state":"ready"}),
        json!({"v":1,"type":"asset.state","id":"slot","state":"loading","request":null}),
    ] {
        let BackendEvent::Asset { echo, .. } = decode_event(&message).unwrap() else {
            panic!("an asset event");
        };
        assert_eq!(echo, None, "{message}");
    }
    let request_only = json!({"v":1,"type":"asset.state","id":"slot","state":"failed","request":4,
            "error":{"code":"cancelled","message":"x"}});
    let BackendEvent::Asset { echo, .. } = decode_event(&request_only).unwrap() else {
        panic!("an asset event");
    };
    assert_eq!(echo, Some(json!({"request": 4})));
}

#[test]
fn the_identity_field_is_additive_to_the_plain_load_message() {
    let plain = scene_load("slot", "red", 1);
    let mut stamped = plain.clone();
    stamp_asset_load(&mut stamped, 9);
    let mut without = stamped.clone();
    without["asset"].as_object_mut().unwrap().remove("request");
    assert_eq!(without, plain, "only `asset.request` was added");
    // The decoder of the ordinary session ignores the echo: the same event as without it.
    assert!(matches!(
        decode_event(&echo("slot", "ready", 9, "red", 1)).unwrap(),
        BackendEvent::Asset {
            status: AssetStatus::Ready(_),
            ..
        }
    ));
}

#[test]
fn hello_defaults_stay_byte_identical_and_options_are_additive() {
    assert_eq!(
        hello("tessel-editor"),
        json!({"v":1,"type":"hello","host":"tessel-editor"})
    );
    let with = hello_with(
        "tessel-editor",
        HelloOptions {
            retain: Some(false),
            presented: true,
            timing: Some(PageTiming { gpu_finish: true }),
            result_cache: Some(false),
            scene_samples: Some(2),
            preview_scene_samples: Some(0),
            preview_sort: Some(PreviewSort::Exact),
            preupload_textures: true,
            warm_draw: true,
        },
    );
    assert_eq!(with["retain"], false);
    assert_eq!(with["warm_draw"], true);
    assert_eq!(with["preupload_textures"], true);
    assert_eq!(with["preview_scene_samples"].as_u64(), Some(0));
    assert_eq!(with["preview_sort"], "exact");
    assert_eq!(with["telemetry"]["presented"], true);
    assert_eq!(with["telemetry"]["timing"], true);
    assert_eq!(with["telemetry"]["gpu_finish"], true);
    // Each flag alone says only itself, and neither leaves the message as it always was: the timing without the GPU wait.
    let timing_only = hello_with(
        "tessel-editor",
        HelloOptions {
            timing: Some(PageTiming::default()),
            ..HelloOptions::default()
        },
    );
    assert_eq!(timing_only["telemetry"], json!({"timing": true}));
    assert_eq!(
        (
            with["result_cache"].as_bool(),
            with["scene_samples"].as_u64()
        ),
        (Some(false), Some(2))
    );
    assert_eq!(with["host"], "tessel-editor");
    // The preview count is not in the message unless the host asks for it.
    let plain = hello_with("tessel-editor", HelloOptions::default());
    assert!(plain.get("preview_scene_samples").is_none());
    assert!(plain.get("preview_sort").is_none());
    // The texture pre-upload (BL157.3) is not in the message unless the host asks for it, and says only itself.
    assert!(plain.get("preupload_textures").is_none());
    let preupload = hello_with(
        "tessel-editor",
        HelloOptions {
            preupload_textures: true,
            ..HelloOptions::default()
        },
    );
    assert_eq!(
        preupload,
        json!({"v":1,"type":"hello","host":"tessel-editor","preupload_textures":true})
    );
    // The warm draw (BL157.3, the other experiment) is likewise absent unless asked for, and says only itself.
    assert!(plain.get("warm_draw").is_none());
    let draw = hello_with(
        "tessel-editor",
        HelloOptions {
            warm_draw: true,
            ..HelloOptions::default()
        },
    );
    assert_eq!(
        draw,
        json!({"v":1,"type":"hello","host":"tessel-editor","warm_draw":true})
    );
}

#[test]
fn the_preview_sort_names_round_trip_and_anything_else_is_refused() {
    for sort in [PreviewSort::Latest, PreviewSort::Exact] {
        assert_eq!(PreviewSort::parse(sort.as_str()), Some(sort));
    }
    assert_eq!(PreviewSort::parse(" Exact "), Some(PreviewSort::Exact));
    assert_eq!(PreviewSort::parse("fast"), None);
    assert_eq!(PreviewSort::parse(""), None);
}

#[test]
fn frame_done_keeps_additive_telemetry_and_presented_decodes() {
    let done = json!({"v":1,"type":"frame.done","id":1,"revision":1,"width":2,"height":2,
        "hash":null,"mean":null,"probes":[],"png":null,
        "timings":{"render_ms":1.0,"total_ms":2.0,"prepare_ms":0.5,"draw_ms":0.25,"flush_ms":0.125},
        "cache":{"texture_reused":1}});
    let BackendEvent::FrameDone(frame) = decode_event(&done).unwrap() else {
        panic!("expected a frame");
    };
    assert_eq!(
        (
            frame.detail.prepare_ms,
            frame.detail.draw_ms,
            frame.detail.flush_ms
        ),
        (0.5, 0.25, 0.125)
    );
    assert_eq!(frame.detail.cache.unwrap()["texture_reused"], 1);
    // A runtime that predates the telemetry answers with neither field.
    let old = json!({"v":1,"type":"frame.done","id":1,"revision":1,"width":2,"height":2,
        "hash":"ab","timings":{"render_ms":1.0,"total_ms":2.0}});
    let BackendEvent::FrameDone(frame) = decode_event(&old).unwrap() else {
        panic!("expected a frame");
    };
    assert_eq!(frame.detail, FrameDetail::default());
    let presented = json!({"v":1,"type":"frame.presented","id":3,"revision":2,"after_done_ms":4.5});
    assert_eq!(
        decode_event(&presented).unwrap(),
        BackendEvent::FramePresented {
            id: 3,
            revision: 2,
            after_done_ms: 4.5
        }
    );
}

struct Loopback(Vec<Value>);

impl Transport for Loopback {
    fn send(&mut self, message: &Value) -> Result<(), BackendError> {
        self.0.push(message.clone());
        Ok(())
    }
    fn recv(&mut self, _: Duration) -> Option<Value> {
        None
    }
    fn close(&mut self) {}
}

#[test]
fn the_bridge_reports_what_a_submit_spent_once() {
    use crate::plan::PlanRequest;
    use crate::state::{EditorState, ProjectSettings};
    let state = EditorState::empty(&ProjectSettings::default()).unwrap();
    let mut backend = BridgeBackend::new("t", Loopback(Vec::new()), Duration::ZERO);
    assert!(backend.take_submit_spans().is_none());
    let request = FrameRequest {
        id: 1,
        revision: 1,
        plan: state.plan(&PlanRequest::of(&state)).unwrap(),
        output: crate::backend::Output {
            width: 2,
            height: 2,
            png: None,
        },
        probes: Vec::new(),
        inspect: false,
        quality: Quality::Original,
    };
    backend.submit(&request).unwrap();
    assert!(backend.take_submit_spans().is_some());
    assert!(backend.take_submit_spans().is_none(), "taken, not repeated");
    assert_eq!(backend.transport_mut().0.len(), 1);
}

#[test]
fn a_warm_is_the_plan_alone_and_the_backend_sends_it_without_a_frame() {
    use crate::plan::PlanRequest;
    use crate::state::{EditorState, ProjectSettings};
    let state = EditorState::empty(&ProjectSettings::default()).unwrap();
    let plan = state.plan(&PlanRequest::of(&state)).unwrap();
    let message = frame_warm(&plan);
    assert_eq!(message["type"], "frame.warm");
    assert_eq!(message["plan"], plan.to_json());
    assert!(
        message.get("id").is_none() && message.get("output").is_none(),
        "no picture is asked for, so nothing to identify or size"
    );
    let mut backend = BridgeBackend::new("t", Loopback(Vec::new()), Duration::ZERO);
    backend.warm_scene(&plan).unwrap();
    assert_eq!(backend.transport_mut().0.len(), 1);
    assert_eq!(backend.transport_mut().0[0]["type"], "frame.warm");
    // The warm of the splat scenes is the message it always was: nothing says `textures` or `scenes`.
    assert!(message.get("textures").is_none() && message.get("scenes").is_none());

    // BL157.3: the same message for the textures only; it does not ask for the splat warm.
    let textures = frame_warm_textures(&plan);
    assert_eq!(textures["type"], "frame.warm");
    assert_eq!(textures["plan"], plan.to_json());
    assert_eq!(
        (textures["textures"].as_bool(), textures["scenes"].as_bool()),
        (Some(true), Some(false))
    );
    backend.warm_textures(&plan).unwrap();
    assert_eq!(backend.transport_mut().0[1], textures);
}

#[test]
fn quality_and_proxy_messages_are_additive_and_decode() {
    use crate::plan::PlanRequest;
    use crate::state::{EditorState, ProjectSettings};
    let state = EditorState::empty(&ProjectSettings::default()).unwrap();
    let mut request = FrameRequest {
        id: 1,
        revision: 1,
        plan: state.plan(&PlanRequest::of(&state)).unwrap(),
        output: crate::backend::Output {
            width: 2,
            height: 2,
            png: None,
        },
        probes: Vec::new(),
        inspect: true,
        quality: Quality::Original,
    };
    assert!(
        frame_render(&request).get("quality").is_none(),
        "an original frame is the message that predates the field"
    );
    request.quality = Quality::Preview;
    assert_eq!(frame_render(&request)["quality"], "preview");

    let id = Id::lit("vid");
    let attach = asset_proxy(
        id,
        Some(&json!({ "pattern": "/p/p%06d.jpg", "first": 0, "count": 3 })),
    );
    assert_eq!(
        (attach["type"].as_str(), attach["proxy"]["count"].as_i64()),
        (Some("asset.proxy"), Some(3))
    );
    assert!(asset_proxy(id, None)["proxy"].is_null());

    let state_of = |state: &str| {
        decode_event(&json!({"v":1,"type":"asset.proxy","id":"vid","state":state,
            "error":{"code":"invalid_proxy","message":"wrong count"}}))
        .unwrap()
    };
    assert!(matches!(
        state_of("attached"),
        BackendEvent::Proxy {
            state: ProxyState::Attached,
            ..
        }
    ));
    assert!(matches!(
        state_of("detached"),
        BackendEvent::Proxy {
            state: ProxyState::Detached,
            ..
        }
    ));
    let BackendEvent::Proxy {
        state: ProxyState::Failed(error),
        ..
    } = state_of("failed")
    else {
        panic!("a failed proxy keeps its error");
    };
    assert_eq!(error.code, "invalid_proxy");
    assert!(decode_event(&json!({"v":1,"type":"asset.proxy","id":"vid","state":"odd"})).is_err());

    let done = json!({"v":1,"type":"frame.done","id":1,"revision":1,"width":2,"height":2,
        "timings":{"render_ms":1.0,"total_ms":2.0},
        "source":{"quality":"preview","proxy_layers":2,"original_layers":1}});
    let BackendEvent::FrameDone(frame) = decode_event(&done).unwrap() else {
        panic!("expected a frame");
    };
    assert_eq!(frame.detail.proxy_layers(), 2);
    assert_eq!(frame.to_json()["source"]["original_layers"], 1);
    assert!(
        frame.detail.is_reduced(),
        "a proxy picture is a reduced one"
    );
    assert_eq!(frame.detail.scene_samples(), None);
}

#[test]
fn frame_done_reports_reduced_scene_msaa_and_old_runtimes_report_none() {
    let decode = |source: Value| {
        let done = json!({"v":1,"type":"frame.done","id":1,"revision":1,"width":2,"height":2,
            "timings":{"render_ms":1.0,"total_ms":2.0}, "source": source});
        let BackendEvent::FrameDone(frame) = decode_event(&done).unwrap() else {
            panic!("expected a frame");
        };
        frame.detail
    };
    let reduced = decode(
        json!({"quality":"preview","proxy_layers":0,"original_layers":0,
        "result_hits":0,"scene_samples":2,"reduced_scene_layers":1}),
    );
    assert_eq!(
        (reduced.scene_samples(), reduced.reduced_scene_layers()),
        (Some(2), 1)
    );
    assert!(reduced.is_reduced());
    let final_quality = decode(json!({"quality":"original","proxy_layers":0,
        "scene_samples":4,"reduced_scene_layers":0}));
    assert_eq!(final_quality.scene_samples(), Some(4));
    assert!(!final_quality.is_reduced());
    // A runtime that predates the field, and a frame without a scene layer, are never reduced.
    assert!(!decode(json!({"quality":"preview","proxy_layers":0})).is_reduced());
    assert!(!FrameDetail::default().is_reduced());
}

#[test]
fn a_splat_order_taken_from_an_earlier_camera_makes_the_picture_a_reduced_one() {
    let decode = |source: Value| {
        let done = json!({"v":1,"type":"frame.done","id":1,"revision":1,"width":2,"height":2,
            "timings":{"render_ms":1.0,"total_ms":2.0}, "source": source});
        let BackendEvent::FrameDone(frame) = decode_event(&done).unwrap() else {
            panic!("expected a frame");
        };
        frame.detail
    };
    // Full MSAA, no proxy: the order alone is what the final picture differs in.
    let stale = decode(
        json!({"quality":"preview","proxy_layers":0,"scene_samples":4,
        "reduced_scene_layers":0,"stale_sort_layers":2}),
    );
    assert_eq!(stale.stale_sort_layers(), 2);
    assert!(
        stale.is_reduced(),
        "the monitor refines it when the playhead stops"
    );
    let exact = decode(
        json!({"quality":"preview","proxy_layers":0,"scene_samples":4,
        "reduced_scene_layers":0,"stale_sort_layers":0}),
    );
    assert!(!exact.is_reduced());
    // A runtime that predates the field always waited for its sort: never stale.
    let old = decode(
        json!({"quality":"preview","proxy_layers":0,"scene_samples":4,"reduced_scene_layers":0}),
    );
    assert_eq!(old.stale_sort_layers(), 0);
    assert!(!old.is_reduced());
    assert_eq!(FrameDetail::default().stale_sort_layers(), 0);
}

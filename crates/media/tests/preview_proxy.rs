//! Preview proxies: derived from the decoded frames by a job, applied as one transaction that is
//! not an edit, never an export input, and invalidated by anything that changes what they show.

mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tessel_editor_core::assets::{Asset, ProxyFormat};
use tessel_editor_core::assets::{DecodedFrames, ProxyFrames};
use tessel_editor_core::backend::AssetDescriptor;
use tessel_editor_core::jobs::{JobError, JobKind, JobRegistry};
use tessel_editor_core::{CommandError, EditorState, headless};
use tessel_editor_media::derive::Cache;
use tessel_editor_media::host::{MediaHost, RendererFactory};
use tessel_editor_media::tools::Tools;

use common::mock::{Mock, Settings};
use common::{temp, tools, video_with_audio};

fn interactive() -> MediaHost {
    let factory: Arc<RendererFactory> = Arc::new(|| Ok(Box::new(Mock::new(Settings::default()))));
    MediaHost::new()
        .with_renderer(factory)
        .with_tools(tools())
        .interactive()
}

fn run(
    host: &mut MediaHost,
    state: &mut EditorState,
    request: Value,
) -> Result<Value, CommandError> {
    headless::execute_ext(state, None, Some(host), &request)
}

fn ok(host: &mut MediaHost, state: &mut EditorState, request: Value) -> Value {
    run(host, state, request.clone()).unwrap_or_else(|error| panic!("{request}: {error:?}"))
}

fn open_project(host: &mut MediaHost, state: &mut EditorState, dir: &Path, name: &str) {
    ok(
        host,
        state,
        json!({"op":"project.create","name":name,"sequence":"main","rate":24,
            "width":320,"height":180,"length":48,"dir":dir.join(name)}),
    );
}

fn poll_for(
    host: &mut MediaHost,
    state: &mut EditorState,
    mut done: impl FnMut(&EditorState) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(180);
    while !done(state) {
        host.poll(state);
        assert!(Instant::now() < deadline, "the condition was never met");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn the_asset(state: &EditorState) -> &Asset {
    state.library.items[0].asset.as_ref().unwrap()
}

fn has_proxy(state: &EditorState) -> bool {
    state
        .library
        .items
        .first()
        .and_then(|item| item.asset.as_ref())
        .and_then(|asset| asset.decoded.as_ref())
        .is_some_and(|decoded| decoded.proxy.is_some())
}

/// A project with the 320x180 clip imported and decoded, the policy asking for 144 rows (the
/// smallest it allows) so the 180-row source is "tall enough" without a large fixture.
fn decoded_project(
    name: &str,
    policy: Option<Value>,
) -> (MediaHost, EditorState, std::path::PathBuf) {
    let dir = temp(name);
    let video = video_with_audio(&tools(), &dir, "clip.mp4");
    let mut host = interactive();
    let mut state = EditorState::demo();
    open_project(&mut host, &mut state, &dir, "p");
    if let Some(policy) = policy {
        ok(&mut host, &mut state, policy);
    }
    ok(
        &mut host,
        &mut state,
        json!({"op":"media.import","paths":[video]}),
    );
    poll_for(&mut host, &mut state, |state| {
        state
            .library
            .items
            .first()
            .and_then(|item| item.asset.as_ref())
            .is_some_and(|asset| {
                asset
                    .decoded
                    .as_ref()
                    .is_some_and(|decoded| decoded.frames.is_some())
            })
    });
    (host, state, dir)
}

/// `Cache::preview_frames` needs a job context, so it runs as an inline job like the CLI's.
fn derive(
    cache: &Cache,
    tools: &Tools,
    asset: &Asset,
    frames: &DecodedFrames,
    height: u32,
    format: ProxyFormat,
) -> Result<(ProxyFrames, bool), JobError> {
    let mut outcome = None;
    let _ = JobRegistry::new(1).run_inline(JobKind::Proxy, "test", |ctx| {
        outcome = Some(cache.preview_frames(tools, asset, frames, height, format, ctx));
        Ok(Value::Null)
    });
    outcome.expect("the body ran")
}

fn policy_144() -> Value {
    json!({"op":"preview.policy","mode":"auto","max_height":144,"format":"jpeg"})
}

#[test]
fn the_host_builds_the_proxy_after_the_media_is_decoded_and_it_is_not_an_edit() {
    let (mut host, mut state, _dir) = decoded_project("proxy-auto", Some(policy_144()));
    poll_for(&mut host, &mut state, has_proxy);
    let revision = state.project.revision;
    let decoded = the_asset(&state).decoded.clone().unwrap();
    let (frames, proxy) = (decoded.frames.unwrap(), decoded.proxy.unwrap());
    assert_eq!((proxy.count, proxy.first), (frames.count, frames.first));
    assert_eq!((proxy.width, proxy.height), (256, 144), "16:9 at 144 rows");
    assert_eq!(proxy.format, ProxyFormat::Jpeg);
    assert_eq!(proxy.source_frames, frames.pattern);
    for index in [proxy.first, proxy.first + proxy.count - 1] {
        let file = tessel_editor_core::assets::expand_pattern(&proxy.pattern, index).unwrap();
        assert!(Path::new(&file).is_file(), "{file}");
    }
    // Nothing else is scheduled once it exists, and applying it left the project untouched.
    let before = host.registry().to_json()["jobs"].as_array().unwrap().len();
    for _ in 0..5 {
        host.poll(&mut state);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        host.registry().to_json()["jobs"].as_array().unwrap().len(),
        before
    );
    assert_eq!(state.project.revision, revision);
}

#[test]
fn the_original_policy_builds_nothing() {
    let (mut host, mut state, _dir) = decoded_project(
        "proxy-original",
        Some(json!({"op":"preview.policy","mode":"original","max_height":144})),
    );
    for _ in 0..20 {
        host.poll(&mut state);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!has_proxy(&state));
    let reply = run(&mut host, &mut state, json!({"op":"media.preview_proxy"})).unwrap_err();
    assert_eq!(reply.code, "policy_original");
}

#[test]
fn a_video_not_taller_than_the_policy_needs_no_proxy_and_says_why() {
    // The default policy asks for 540 rows; the clip has 180.
    let (mut host, mut state, _dir) = decoded_project("proxy-small", None);
    for _ in 0..20 {
        host.poll(&mut state);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!has_proxy(&state));
    let report = ok(&mut host, &mut state, json!({"op":"preview.policy"}));
    assert_eq!(report["assets"][0]["reason"], "small_enough");
}

#[test]
fn the_explicit_operation_builds_the_proxy_and_the_cache_is_reused_then_rebuilt_for_other_parameters()
 {
    let (mut host, mut state, dir) = decoded_project("proxy-explicit", Some(policy_144()));
    let answer = ok(
        &mut host,
        &mut state,
        json!({"op":"media.preview_proxy","wait":true}),
    );
    assert!(
        answer.get("job").is_some() || answer["applied"].is_array(),
        "{answer}"
    );
    poll_for(&mut host, &mut state, has_proxy);

    // The cache itself: same parameters reuse, other ones build beside it.
    let cache = Cache::new(&dir.join("p"));
    let tools = tools();
    let asset = the_asset(&state).clone();
    let frames = asset.decoded.clone().unwrap().frames.unwrap();
    let (first, reused) = derive(&cache, &tools, &asset, &frames, 144, ProxyFormat::Jpeg).unwrap();
    assert!(reused, "the job already built it");
    let (png, reused) = derive(&cache, &tools, &asset, &frames, 144, ProxyFormat::Png).unwrap();
    assert!(!reused);
    assert_ne!(first.pattern, png.pattern);
    assert_eq!(png.count, first.count);
    let tall = derive(&cache, &tools, &asset, &frames, 180, ProxyFormat::Jpeg);
    assert_eq!(
        tall.unwrap_err().code,
        "unsupported_format",
        "no proxy that is not smaller"
    );
}

#[test]
fn export_and_the_runtime_descriptor_keep_reading_the_original_frames() {
    let (mut host, mut state, _dir) = decoded_project("proxy-original-source", Some(policy_144()));
    poll_for(&mut host, &mut state, has_proxy);
    let item = &state.library.items[0];
    let decoded = the_asset(&state).decoded.clone().unwrap();
    let descriptor = AssetDescriptor::from_item(item, None).unwrap();
    assert_eq!(
        descriptor.source["pattern"].as_str(),
        Some(decoded.frames.unwrap().pattern.as_str()),
        "`asset.load` is the original; the proxy travels in `asset.proxy`"
    );
    let original = the_asset(&state).source.clone();
    assert_eq!(
        tessel_editor_media::derive::for_export(the_asset(&state)),
        match &original {
            tessel_editor_core::assets::Source::File { path } => Some(path.as_str()),
            _ => None,
        }
    );
}

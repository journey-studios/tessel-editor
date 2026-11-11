//! F060: a point light in a real export. Lib + headless host, no window: real Node + headless
//! Chrome + Three.js frames, real FFmpeg, over the synthetic corpus of
//! `scripts/directed-scene-proof`.
//!
//! What this proves, through the JSONL surface only (`scene.attach`, `scene.edit`, `scene.range`,
//! `scene.render`, `scene.export`, `history.undo`):
//!
//! - the export of a lit scene is not the export of the unlit one, and every export frame hash
//!   equals what `scene.render` reports for the same edit frame (preview, headless render and
//!   export are one path), for a light parented to a node that moves across the frames;
//! - the radius the numeric `scene.edit` writes and the one `scene.range` writes (the command the
//!   gizmo's knob shares its function with) draw the same picture; undo gives the lit pictures back;
//! - the export adds nothing to the history and no timeline clip.
//!
//! What it does not prove: the editor's marker and range sphere being absent from a frame is
//! proved by the overlay living on its own 2D canvas (`runtime/test/directed-scene-light.test.mjs`);
//! no WKWebView, no pointer, no performance.
//!
//! Skipped (reason on stderr) without Chrome, the runtime bundle or FFmpeg; set
//! `TESSEL_REQUIRE_RUNTIME=1` to turn the skip into a failure. Run under `with-gpu-lock.py`.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use tessel_editor_core::state::ProjectSettings;
use tessel_editor_core::{CommandError, EditorState, headless};
use tessel_editor_media::host::MediaHost;
use tessel_editor_media::tools::Tools;
use tessel_editor_render::RuntimeConfig;
use tessel_editor_render::scene_host::SceneHost;

const LAMP: &str = "scene/scene-a/light/lamp";

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/directed-scene-proof/corpus")
}

struct Session {
    state: EditorState,
    host: SceneHost,
    work: PathBuf,
}

impl Session {
    fn run(&mut self, request: Value) -> Result<Value, CommandError> {
        headless::execute_ext(&mut self.state, None, Some(&mut self.host), &request)
    }

    fn ok(&mut self, request: Value) -> Value {
        self.run(request.clone())
            .unwrap_or_else(|error| panic!("{request}: {error:?}"))
    }

    fn export(&mut self, scene: &str, name: &str, from: i64, frames: i64) -> Vec<String> {
        let destination = self.work.join(name);
        let result = self.ok(json!({"op":"scene.export","scene":scene,"preset":"fullhd",
            "from":from,"frames":frames,"destination":destination}));
        assert_eq!(result["verification"]["ok"], true);
        result["frame_hashes"]["list"]
            .as_array()
            .expect("a short export lists every frame hash")
            .iter()
            .map(|hash| hash.as_str().unwrap().to_string())
            .collect()
    }

    fn rendered_hash(&mut self, scene: &str, frame: i64) -> String {
        self.ok(json!({"op":"scene.render","scene":scene,"frame":frame,
            "width":1920,"height":1080}))["frame"]["hash"]
            .as_str()
            .unwrap()
            .to_string()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.work);
    }
}

fn session() -> Option<Session> {
    let skip = |reason: &str| {
        assert!(
            std::env::var("TESSEL_REQUIRE_RUNTIME").as_deref() != Ok("1"),
            "the runtime is required but unavailable: {reason}"
        );
        eprintln!("SKIPPED (runtime unavailable): {reason}");
        None
    };
    let config = match RuntimeConfig::discover() {
        Ok(config) => config,
        Err(setup) => return skip(&setup.message()),
    };
    if let Err(error) = Tools::discover() {
        return skip(&error.message);
    }
    let work = std::env::temp_dir().join(format!("te-light-{}", std::process::id()));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).unwrap();
    let mut session = Session {
        state: EditorState::empty(&ProjectSettings::default()).unwrap(),
        host: SceneHost::new(MediaHost::new(), config),
        work,
    };
    let mut bundle: Value =
        serde_json::from_str(&fs::read_to_string(corpus().join("bundle.json")).unwrap()).unwrap();
    session.ok(json!({"op":"scene.attach","id":"plain","bundle":bundle.clone()}));
    bundle["sceneDefinitions"][0]["lighting"] = json!({"ambient": 1, "lights": [{
        "id": "lamp", "parent": "hero", "position": {"x": 0.0, "y": 1.5, "z": 2.0},
        "color": {"r": 1.0, "g": 0.8, "b": 0.5}, "intensity": 3.0, "radius": 5.0, "flicker": 0.0,
    }]});
    session.ok(json!({"op":"scene.attach","id":"street","bundle":bundle}));
    for scene in ["plain", "street"] {
        session.ok(json!({"op":"scene.resources.resolve","scene":scene,"base":corpus()}));
    }
    Some(session)
}

#[test]
fn a_lit_export_is_what_render_draws_and_the_numeric_range_and_undo_reach_the_picture() {
    let Some(mut session) = session() else {
        return;
    };
    // The corpus animates the hero across these frames, so the parented lamp moves with it.
    let (from, frames) = (0, 3);
    let plain = session.export("plain", "plain.mp4", from, frames);
    let lit = session.export("street", "lit.mp4", from, frames);
    let depth = |session: &mut Session| {
        session.ok(json!({"op":"history.status"}))["history"]["undo_depth"].clone()
    };
    let depth_before = depth(&mut session);
    for index in 0..frames as usize {
        assert_ne!(
            lit[index], plain[index],
            "frame {index}: the light changes the export"
        );
        assert_eq!(
            lit[index],
            session.rendered_hash("street", from + index as i64),
            "export frame {index} is not what scene.render draws"
        );
        assert_eq!(
            plain[index],
            session.rendered_hash("plain", from + index as i64),
            "the unlit export frame {index} is not what scene.render draws"
        );
    }
    assert_ne!(lit[0], lit[2], "the picture follows the parented lamp");

    // The numeric edit and the range command write the same radius, so they draw the same picture.
    let by_edit = {
        session.ok(json!({"op":"scene.edit","scene":"street","ops":[
            {"set": format!("{LAMP}/radius"), "value": 6.5}]}));
        session.rendered_hash("street", 0)
    };
    assert_ne!(by_edit, lit[0], "a larger radius changes the picture");
    session.ok(json!({"op":"history.undo"}));
    assert_eq!(
        session.rendered_hash("street", 0),
        lit[0],
        "undo restores it"
    );
    session.ok(json!({"op":"scene.range","scene":"street","entity":LAMP,"frame":0,"delta":1.5}));
    assert_eq!(
        session.rendered_hash("street", 0),
        by_edit,
        "scene.range 5 + 1.5 draws what radius 6.5 draws"
    );
    session.ok(json!({"op":"history.undo"}));

    // The exports and renders added nothing the user would undo.
    assert_eq!(depth(&mut session), depth_before);
    assert_eq!(
        session.state.timeline.clips.len(),
        0,
        "never a timeline clip"
    );
    assert_eq!(session.export("street", "lit2.mp4", from, frames), lit);
}

use serde_json::json;

use super::*;

#[test]
fn preview_intent_round_trips_and_rejects_incomplete_requests() {
    let source = PreviewIntent::Source {
        asset: Id::lit("img"),
        time: Ratio::new(1, 2).unwrap(),
    };
    assert_eq!(PreviewIntent::from_json(&source.to_json()).unwrap(), source);
    assert_eq!(
        PreviewIntent::from_json(&json!({"mode":"sequence"})).unwrap(),
        PreviewIntent::Sequence
    );
    assert_eq!(
        PreviewIntent::from_json(&json!({"mode":"source"}))
            .unwrap_err()
            .code,
        "invalid_request"
    );
    assert!(PreviewIntent::from_json(&json!({"mode":"wat"})).is_err());
}

#[test]
fn demonstration_entries_without_media_cannot_be_planned_as_sources() {
    let state = EditorState::demo();
    let error = state
        .plan(&PlanRequest {
            intent: PreviewIntent::Source {
                asset: Id::lit("rain"),
                time: Ratio::ZERO,
            },
            at: PlanTime::Playhead,
        })
        .unwrap_err();
    assert!(
        matches!(error.code, "asset_unavailable" | "unknown_element"),
        "{error:?}"
    );
}

// ---- Occlusion: which layers a layer above hides completely --------------------------------

mod occlusion {
    use super::*;
    use crate::assets::{Identity, Meta};
    use crate::state::{Command, ProjectSettings};

    fn asset(path: &str, width: u32, height: u32) -> Asset {
        Asset {
            source: Source::File { path: path.into() },
            meta: Meta {
                width: Some(width),
                height: Some(height),
                rate: Rate::new(24, 1),
                duration: Ratio::new(10, 1),
                ..Meta::default()
            },
            identity: Identity::default(),
            decoded: None,
        }
    }

    /// A 1280×720 canvas with `clip.mp4` (1280×720), `photo.jpg` (1280×720), `art.png`
    /// (1280×720), `wide.jpg` (1440×1080, 4:3) in the library.
    fn state() -> EditorState {
        let mut state = EditorState::empty(&ProjectSettings::default()).unwrap();
        for (id, kind, path, width, height) in [
            ("clip", MediaKind::Video, "/m/clip.mp4", 1280, 720),
            ("photo", MediaKind::Still, "/m/photo.jpg", 1280, 720),
            ("art", MediaKind::Still, "/m/art.png", 1280, 720),
            ("fourthree", MediaKind::Still, "/m/wide.JPG", 1440, 1080),
            ("mov", MediaKind::Video, "/m/clip.mov", 1280, 720),
        ] {
            state
                .execute(Command::AddAsset {
                    id: Some(id.into()),
                    name: id.into(),
                    kind,
                    asset: Box::new(asset(path, width, height)),
                    folder: None,
                })
                .unwrap();
        }
        state
    }

    fn layer(id: &str, content: Content) -> Layer {
        Layer {
            id: format!("layer:{id}"),
            clip: None,
            asset: Some(Id::lit(id)),
            content,
            transform: Transform::default(),
            opacity: 1.0,
            blend: "normal".to_string(),
            occluded: false,
        }
    }

    fn video(id: &str) -> Layer {
        layer(
            id,
            Content::Video {
                source_time: Ratio::ZERO,
                source_frame: 0,
            },
        )
    }

    fn flags(state: &EditorState, mut layers: Vec<Layer>) -> Vec<bool> {
        // The fixtures assume a 1280×720 canvas whatever the empty project defaults to.
        state.mark_occluded(&mut layers, 1280, 720);
        layers.iter().map(|layer| layer.occluded).collect()
    }

    #[test]
    fn an_opaque_full_frame_layer_hides_everything_below_it_and_only_below() {
        let state = state();
        let below = layer("art", Content::Image);
        assert_eq!(
            flags(&state, vec![below.clone(), video("clip")]),
            vec![true, false]
        );
        assert_eq!(
            flags(&state, vec![below.clone(), layer("photo", Content::Image)]),
            vec![true, false]
        );
        // The cover is itself covered by one above it; the rest below stay hidden.
        assert_eq!(
            flags(
                &state,
                vec![
                    below.clone(),
                    video("clip"),
                    layer("photo", Content::Image),
                    below.clone()
                ]
            ),
            vec![true, true, false, false],
            "a PNG on top covers nothing: only what is under the JPEG is hidden"
        );
        assert_eq!(flags(&state, vec![video("clip")]), vec![false]);
        assert!(flags(&state, vec![]).is_empty());
    }

    #[test]
    fn a_scene_below_a_cover_is_hidden_but_a_scene_never_covers() {
        let state = state();
        let scene = layer(
            "art",
            Content::Scene {
                camera: Camera::default(),
                time: Ratio::ZERO,
                orientation: [0.0; 3],
            },
        );
        assert_eq!(
            flags(&state, vec![scene.clone(), video("clip")]),
            vec![true, false]
        );
        assert_eq!(
            flags(&state, vec![video("clip"), scene]),
            vec![false, false]
        );
    }

    #[test]
    fn nothing_that_could_show_through_or_miss_a_pixel_covers() {
        let state = state();
        let base = layer("art", Content::Image);
        let covers = |edit: &dyn Fn(&mut Layer)| {
            let mut top = video("clip");
            edit(&mut top);
            flags(&state, vec![base.clone(), top])[0]
        };
        assert!(covers(&|_| {}), "the control covers");
        assert!(!covers(&|top| top.opacity = 0.99), "translucent");
        assert!(
            !covers(&|top| top.blend = "multiply".into()),
            "another blend mode"
        );
        assert!(!covers(&|top| top.transform.rotation = 5.0), "rotated");
        assert!(
            covers(&|top| top.transform.rotation = 360.0),
            "a full turn is no turn"
        );
        assert!(!covers(&|top| top.transform.scale = 0.99), "shrunk");
        assert!(covers(&|top| top.transform.scale = 1.2), "enlarged");
        assert!(
            !covers(&|top| top.transform.x = 1.0),
            "moved with nothing to spare"
        );
        assert!(
            covers(&|top| {
                top.transform.scale = 1.2;
                top.transform.x = 5.0;
            }),
            "moved within the margin the scale gives"
        );
        assert!(
            !covers(&|top| top.transform.crop = [0.0, 0.5, 0.0, 0.0]),
            "cropped"
        );
        assert!(!covers(&|top| top.opacity = f64::NAN), "not a number");
        assert!(!covers(&|top| top.transform.scale = f64::NAN));
    }

    #[test]
    fn only_formats_without_alpha_are_trusted_and_the_picture_must_reach_the_edges() {
        let state = state();
        let base = layer("art", Content::Image);
        let top_of = |top: Layer| flags(&state, vec![base.clone(), top])[0];
        assert!(top_of(video("clip")), "mp4");
        assert!(!top_of(video("mov")), "a mov can carry alpha (ProRes 4444)");
        assert!(!top_of(layer("art", Content::Image)), "png");
        assert!(
            !top_of(layer(
                "clip",
                Content::Text(TextSpec::new("T", 48.0, "#ffffff").unwrap())
            )),
            "text"
        );
        assert!(
            !top_of(layer("nothing", Content::Image)),
            "an asset that is not in the library"
        );

        // 4:3 in a 16:9 canvas is letterboxed until it is enlarged by the aspect ratio.
        let four_three = |scale: f64, x: f64| {
            let mut top = layer("fourthree", Content::Image);
            top.transform.scale = scale;
            top.transform.x = x;
            top_of(top)
        };
        assert!(!four_three(1.0, 0.0), "letterboxed");
        assert!(!four_three(1.33, 0.0), "just short");
        assert!(
            four_three(1.34, 0.0),
            "uppercase extension; 960 × 1.34 ≥ 1280 and 720 × 1.34 ≥ 720"
        );
        assert!(!four_three(1.34, 3.0), "pushed off one edge");
    }

    #[test]
    fn a_real_sequence_plan_flags_the_covered_clip_and_keeps_its_requirements() {
        use crate::state::timeline::TrackKind;
        let mut state = state();
        for track in ["v1", "v2"] {
            state
                .execute(Command::CreateTrack {
                    id: Some(track.into()),
                    name: track.into(),
                    kind: TrackKind::Video,
                })
                .unwrap();
        }
        for (clip, track, asset) in [("under", "v1", "art"), ("over", "v2", "clip")] {
            state
                .execute(Command::AddClip {
                    id: Some(clip.into()),
                    name: None,
                    track: track.into(),
                    asset: Some(asset.into()),
                    start: 0,
                    duration: 48,
                    source_start: None,
                    text: None,
                    camera: None,
                    properties: vec![],
                })
                .unwrap();
        }
        // The empty project is 1920×1080 and the media 1280×720: same 16:9, so the fit covers.
        let plan = state
            .plan(&PlanRequest {
                intent: PreviewIntent::Sequence,
                at: PlanTime::Frame(10),
            })
            .unwrap();
        assert_eq!(plan.layers.len(), 2);
        let flags: Vec<bool> = plan.layers.iter().map(|layer| layer.occluded).collect();
        assert_eq!(flags.len(), 2);
        assert!(flags.contains(&true) && flags.contains(&false), "{flags:?}");
        assert!(plan.layers.last().is_some_and(|top| !top.occluded));
        assert_eq!(
            plan.requires.len(),
            2,
            "the covered asset is still required"
        );
        assert_eq!(plan.to_json()["layers"][0]["occluded"], true);
    }

    #[test]
    fn the_flag_reaches_the_plan_json_only_when_set() {
        let state = state();
        let mut layers = vec![layer("art", Content::Image), video("clip")];
        state.mark_occluded(&mut layers, 1280, 720);
        assert_eq!(layer_json(&layers[0])["occluded"], true);
        assert!(layer_json(&layers[1]).get("occluded").is_none());
    }
}

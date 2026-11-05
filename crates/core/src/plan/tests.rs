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

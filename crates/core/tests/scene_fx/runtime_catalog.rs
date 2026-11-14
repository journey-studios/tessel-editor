//! Contract shared with the runtime fixture; no runtime or GPU is started here.

use std::fs;

use serde_json::{Value, json};
use tessel_editor_core::directed_scene::fx;

/// The slice of the catalog the runtime draws from (`runtime/test/fixtures/fx-catalog.json`): the
/// runtime's own test pins `OUTPUT_FX_EFFECTS` to the same file, so a range, a kind or the parameter order
/// cannot change on one side only. `TESSEL_UPDATE_FX_CONTRACT=1` rewrites the file from the catalog.
#[test]
fn the_runtime_contract_fixture_is_the_catalog() {
    let projection = json!({
        "catalog_version": fx::FX_CATALOG_VERSION,
        "effects": fx::catalog().iter().map(|spec| json!({
            "id": spec.id,
            "params": spec.params.iter().map(|param| match param.kind {
                fx::ParamKind::Number { min, max, .. } => json!({"id": param.id, "kind": "number", "min": min, "max": max}),
                // `parse_param` reads each channel in 0..=1 (a constant of the rule, not of the spec).
                fx::ParamKind::Color { .. } => json!({"id": param.id, "kind": "color", "min": 0, "max": 1}),
                fx::ParamKind::Point { min, max, .. } => json!({"id": param.id, "kind": "vec2", "min": min, "max": max}),
                fx::ParamKind::Bool { .. } => json!({"id": param.id, "kind": "bool"}),
                fx::ParamKind::Choice { .. } => unreachable!("only the transition table has choices"),
            }).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    });
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../runtime/test/fixtures/fx-catalog.json"
    );
    if std::env::var_os("TESSEL_UPDATE_FX_CONTRACT").is_some() {
        fs::write(
            path,
            serde_json::to_string_pretty(&projection).unwrap() + "\n",
        )
        .unwrap();
    }
    let pinned: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(
        pinned, projection,
        "the runtime fixture drifted from the catalog; regenerate with TESSEL_UPDATE_FX_CONTRACT=1"
    );
    // And the ranges are the ones the parser enforces: just inside passes, just outside is refused.
    for spec in fx::catalog() {
        for param in spec.params {
            if let fx::ParamKind::Number { min, max, .. } = param.kind {
                assert!(fx::parse_param(param, &json!(min)).is_ok());
                assert!(fx::parse_param(param, &json!(max)).is_ok());
                assert!(fx::parse_param(param, &json!(min - 0.001)).is_err());
                assert!(fx::parse_param(param, &json!(max + 0.001)).is_err());
            }
        }
    }
}

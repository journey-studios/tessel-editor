//! M4 compat: the Rust capability list grew by one (`fxTransition`), the runtime's did not (S2
//! owns that), and every capability the runtime does list is still one this build knows. A plan with
//! a transition asks for it, so an old runtime refuses the plan by name instead of drawing one deck.

use core::prelude::v1::test;

use serde_json::json;
use tessel_editor_core::scene_plan::{Capability, capabilities};

use crate::support::*;

const RUNTIME: &str = include_str!("../../../../runtime/src/directed-scene.js");

/// The strings of `export const SCENE_CAPABILITIES = [ ... ];`, read as text (no JS is executed).
fn runtime_capabilities() -> Vec<String> {
    let start = RUNTIME
        .find("export const SCENE_CAPABILITIES = [")
        .expect("the runtime exports its capabilities");
    let body = &RUNTIME[start..];
    let body = &body[body.find('[').unwrap() + 1..];
    let body = &body[..body.find(']').unwrap()];
    body.split(',')
        .map(|entry| entry.trim().trim_matches('\''))
        .filter(|entry| !entry.is_empty())
        .map(str::to_string)
        .collect()
}

#[test]
fn the_runtime_list_is_a_valid_subset_and_does_not_claim_the_transition() {
    let listed = runtime_capabilities();
    assert_eq!(listed.len(), 13, "{listed:?}");
    assert!(!listed.iter().any(|name| name == "fxTransition"));
    for name in &listed {
        assert!(
            Capability::parse(name).is_some(),
            "the runtime lists `{name}`, which this build does not know"
        );
    }
    // Everything Rust knows but the runtime lacks is exactly the transition, for now.
    let lacking: Vec<&str> = Capability::ALL
        .iter()
        .map(|capability| capability.as_str())
        .filter(|name| !listed.iter().any(|listed| listed == name))
        .collect();
    assert_eq!(lacking, ["fxTransition"]);
}

#[test]
fn a_runtime_with_the_current_list_refuses_a_window_plan_and_serves_the_others() {
    let listed: Vec<Capability> = runtime_capabilities()
        .iter()
        .filter_map(|name| Capability::parse(name))
        .collect();
    let scene = spec_scene(vec![tr(json!({}))]);
    let window = plan_at(&scene, 44);
    assert_eq!(
        window.missing_capabilities(&listed),
        [Capability::FxTransition],
        "an old runtime must name what it lacks"
    );
    assert!(window.require_capabilities(&listed).is_err());
    for frame in [0, 30, 41, 54, 100] {
        let plain = plan_at(&scene, frame);
        assert!(
            plain.missing_capabilities(&listed).is_empty(),
            "frame {frame}"
        );
    }
}

#[test]
fn this_build_plans_every_capability_it_lists() {
    assert_eq!(capabilities().supported, Capability::ALL);
}

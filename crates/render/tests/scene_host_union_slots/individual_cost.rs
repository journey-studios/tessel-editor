//! Isolate a picture's own cost from the total: the second resource truncates to zero.
//! These are typed bookkeeping probes over tiny approved files, not enormous allocations.

use super::*;
use tessel_editor_core::backend::BackendError;

fn prepare_picture(width: u64, height: f64) -> Result<(), BackendError> {
    let fixture = Fixture::new("individual-cost", &Content::sky());
    let session = fixture.approve();
    let mut plan = fixture.plan(PLAIN);
    let map = approved_map(&plan, PROJECT_SCENE, &session).unwrap();
    plan.resources[0].width_px = width;
    plan.resources[0].height_px = height;
    plan.resources[1].width_px = 1;
    plan.resources[1].height_px = 1.0e-10; // Existing cost model truncates this to zero.
    plan.validate().unwrap();
    let mut slots = SceneSlots::new(32, u64::MAX);
    let result = slots.prepare(&plan, &map);
    if result.is_err() {
        assert_eq!((slots.occupied(), slots.generation()), (0, 0));
    }
    result.map(|_| ())
}

fn overflow(error: BackendError) {
    assert_eq!(error.code, "resource_budget");
    assert_eq!(error.details.unwrap()["overflow"], true);
}

#[test]
fn a_nonfinite_individual_cost_is_refused_even_without_an_overflowing_sum() {
    // Both input dimensions are finite; their decoded byte cost is infinite.
    overflow(prepare_picture(u64::MAX, 1.0e300).unwrap_err());
}

#[test]
fn two_to_the_64_is_refused_but_the_preceding_float_cost_is_accepted() {
    // Width * 1 * 4: exactly 2^64 is not representable in u64.
    overflow(prepare_picture(1_u64 << 62, 1.0).unwrap_err());
    // This width is exact in f64: its cost is 2^64 - 2048, the preceding f64.
    prepare_picture((1_u64 << 62) - 512, 1.0).unwrap();
}

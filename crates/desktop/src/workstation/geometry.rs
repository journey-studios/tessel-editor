//! Element bounds measured during paint.
//!
//! GPUI mouse events carry window coordinates but not the bounds of the element
//! they hit. A [`Geometry`] probe records them each frame so handlers can turn a
//! pointer position into a slider fraction, a timeline frame or a lane.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::*;

#[derive(Clone, Default)]
pub struct Geometry(Rc<RefCell<HashMap<String, Bounds<Pixels>>>>);

impl Geometry {
    /// An invisible element that fills its positioned parent and records its
    /// bounds under `id` every time the window paints.
    pub fn probe(&self, id: impl Into<String>) -> impl IntoElement {
        let store = self.0.clone();
        let id = id.into();
        canvas(
            move |bounds, _, _| {
                store.borrow_mut().insert(id, bounds);
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top(px(0.))
        .left(px(0.))
        .size_full()
    }

    pub fn get(&self, id: &str) -> Option<Bounds<Pixels>> {
        self.0.borrow().get(id).copied()
    }

    /// `[x, y, width, height]` of every measured region, for the automation tree.
    pub fn regions(&self) -> HashMap<String, [f32; 4]> {
        self.0
            .borrow()
            .iter()
            .filter(|(id, _)| id.starts_with("region:"))
            .map(|(id, bounds)| {
                (
                    id.clone(),
                    [
                        f32::from(bounds.origin.x),
                        f32::from(bounds.origin.y),
                        f32::from(bounds.size.width),
                        f32::from(bounds.size.height),
                    ],
                )
            })
            .collect()
    }
}

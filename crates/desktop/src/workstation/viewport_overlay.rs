//! The gizmo overlay as the viewport host sees it (F052): the page draws it on a canvas of its
//! own over the picture, and this side only (a) switches it on for a directed scene, (b) sends
//! the core's `Overlay` JSON when it changed and (c) counts what the page reports back.
//!
//! WHY here and not in the GPUI tree: a native WebView paints above every GPUI element, so a
//! handle drawn by GPUI over the monitor would be hidden by it (or, worse, drawn in a place the
//! picture does not match). The page owns the pixels and the pointer; the core owns every
//! decision (`scene.pointer.*`). Nothing in this file knows what a handle is.

use super::*;

/// The feature a runtime announces when its page can draw and report the overlay. A runtime that
/// predates it is never sent `overlay.*` messages.
pub const FEATURE: &str = "overlay.gizmo";

/// Set to `1` to have the page acknowledge every `overlay.set` with what it drew (a probe of the
/// pixels at each handle). Off by default: it reads back from the canvas. For native proofs.
pub const ACK_ENV: &str = "TESSEL_VIEWPORT_OVERLAY_ACK";

#[derive(Default)]
pub(super) struct OverlayHost {
    pub enabled: bool,
    ack: bool,
    /// The last `overlay.set` payload sent, so an unchanged overlay is not sent again.
    last_sent: Option<String>,
    layout: Option<Value>,
    drawn: Option<Value>,
    pointer_events: u64,
    /// Pointer messages and acks that answered an overlay other than the one the core would
    /// draw now (an older page, project, revision or frame): counted, never applied.
    stale: u64,
    /// The stamp of the overlay last sent: an ack is only recorded when it echoes this one.
    last_stamp: Option<Value>,
    /// Tests only: a page that announced the feature without a WebView behind it.
    #[cfg(test)]
    pretend: bool,
    sets: u64,
    last_phase: Option<String>,
}

impl OverlayHost {
    pub fn from_env() -> Self {
        Self {
            ack: std::env::var(ACK_ENV).is_ok_and(|value| value == "1"),
            ..Self::default()
        }
    }
}

impl ViewportHost {
    /// Whether the page announced it can draw the overlay.
    pub fn overlay_supported(&self) -> bool {
        #[cfg(test)]
        if self.overlay.pretend {
            return true;
        }
        self.is_ready()
            && self
                .session
                .known_capabilities()
                .is_some_and(|capabilities| capabilities.supports(FEATURE))
    }

    fn overlay_send(&mut self, mut message: Value) {
        message["v"] = json!(1);
        let _ = self.session.backend_mut().transport_mut().send(&message);
    }

    /// Tests only: the page announced `overlay.gizmo` (a test window has no WebView).
    #[cfg(test)]
    pub fn pretend_overlay_supported(&mut self, supported: bool) {
        self.overlay.pretend = supported;
    }

    /// Counts a pointer message that answered an overlay the core no longer draws.
    pub fn overlay_note_stale(&mut self) {
        self.overlay.stale += 1;
    }

    /// Switches the page's overlay on or off. Returns whether anything was sent.
    pub fn overlay_set_enabled(&mut self, enabled: bool) -> bool {
        if enabled == self.overlay.enabled || (enabled && !self.overlay_supported()) {
            return false;
        }
        self.overlay.enabled = enabled;
        self.overlay.last_sent = None;
        if self.overlay_supported() {
            self.overlay_send(json!({ "type": "overlay.enable", "enabled": enabled }));
        }
        true
    }

    /// Sends the overlay if the page does not already have exactly this one.
    pub fn overlay_show(&mut self, overlay: &Value) -> bool {
        if !self.overlay.enabled || !self.overlay_supported() {
            return false;
        }
        let text = overlay.to_string();
        if self.overlay.last_sent.as_deref() == Some(text.as_str()) {
            return false;
        }
        self.overlay.last_sent = Some(text);
        self.overlay.last_stamp = Some(overlay["stamp"].clone());
        self.overlay.sets += 1;
        let ack = self.overlay.ack;
        self.overlay_send(json!({ "type": "overlay.set", "overlay": overlay, "ack": ack }));
        true
    }

    /// The page is gone or reloaded: it starts with the overlay off and nothing drawn.
    pub fn overlay_forget(&mut self) {
        let ack = self.overlay.ack;
        #[cfg(test)]
        let pretend = self.overlay.pretend;
        self.overlay = OverlayHost {
            ack,
            #[cfg(test)]
            pretend,
            ..OverlayHost::default()
        };
    }

    /// Counts and keeps what the page reports about the overlay. `true` if `message` was one.
    pub fn overlay_note(&mut self, message: &Value) -> bool {
        match message["type"].as_str() {
            Some("overlay.layout") => self.overlay.layout = Some(message.clone()),
            // An ack of an overlay that is no longer the current one says nothing about what is
            // on the page now (a reload or a replaced project may have happened since).
            Some("overlay.drawn") => {
                if self.overlay.last_stamp.as_ref() == Some(&message["stamp"]) {
                    self.overlay.drawn = Some(message.clone());
                } else {
                    self.overlay.stale += 1;
                }
            }
            Some("overlay.pointer") => {
                self.overlay.pointer_events += 1;
                self.overlay.last_phase = message["phase"].as_str().map(str::to_owned);
            }
            _ => return false,
        }
        true
    }

    pub fn overlay_report(&self) -> Value {
        json!({
            "supported": self.overlay_supported(),
            "enabled": self.overlay.enabled,
            "sets": self.overlay.sets,
            "pointer_events": self.overlay.pointer_events,
            "stale": self.overlay.stale,
            "last_phase": self.overlay.last_phase,
            "layout": self.overlay.layout,
            "drawn": self.overlay.drawn,
        })
    }
}

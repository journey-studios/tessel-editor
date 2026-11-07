//! The gizmo overlay as the viewport host sees it (F055): the page draws it on a canvas of its
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
    failure: Option<Value>,
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
    /// What the adapter spent on the overlay (M2 of the PR #25 review), all in `overlay_report`:
    /// `computed` = asked the core for `scene.gizmo.state` (plans the scene), `adopted` = took the
    /// overlay a gizmo command already answered with (no second plan), `memo_hits` = the page
    /// already had the overlay of this state, `held` = playback runs and the overlay is cleared.
    computed: u64,
    adopted: u64,
    memo_hits: u64,
    held: bool,
    /// The key (`EditorState::gizmo_overlay_key`) of the overlay the page has: the same key means
    /// the same overlay, so the core is not asked again. Reset with `last_sent`.
    memo: Option<String>,
    /// The incarnation of the page the overlay is for: bumped every time the page is forgotten
    /// (reload, runtime gone). The adapter puts it in the stamp of every overlay, so a message
    /// of the page before the reload never equals the stamp of the page after it, even when the
    /// core's own stamp (scene, session, revision, frame) is the same (M4 of the PR #25 review).
    epoch: u64,
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

    /// The incarnation of the page (see `OverlayHost::epoch`): part of every stamp the adapter sends.
    pub fn overlay_epoch(&self) -> u64 {
        self.overlay.epoch
    }

    /// Whether the page's overlay is on (it needs a runtime that announced the feature, so this
    /// also says "supported"): the gate before any planning of the scene for it.
    pub fn overlay_enabled(&self) -> bool {
        self.overlay.enabled && self.overlay_supported()
    }

    /// The page already has the overlay `key` stands for: the core need not be asked. Counted.
    pub fn overlay_memo_hit(&mut self, key: &str) -> bool {
        let hit = self.overlay.enabled
            && self.overlay_supported()
            && self.overlay.memo.as_deref() == Some(key);
        if hit {
            self.overlay.memo_hits += 1;
        }
        hit
    }

    /// The core was asked for the overlay `key` stands for (and the answer is being sent).
    pub fn overlay_computed(&mut self, key: String) {
        self.overlay.computed += 1;
        self.overlay.held = false;
        self.overlay.memo = Some(key);
    }

    /// The overlay `key` stands for came with the answer of a gizmo command: no second plan.
    pub fn overlay_adopted(&mut self, key: String) {
        self.overlay.adopted += 1;
        self.overlay.held = false;
        self.overlay.memo = Some(key);
    }

    /// Playback runs: the picture moves every frame, so a gizmo laid out for one of them would be
    /// drawn on the wrong place and planning it per tick would double the cost of a frame. The
    /// page's overlay is cleared once and nothing is planned until the clock stops.
    pub fn overlay_hold(&mut self) -> bool {
        if self.overlay.held || !self.overlay.enabled || !self.overlay_supported() {
            return false;
        }
        self.overlay.held = true;
        self.overlay.memo = None;
        self.overlay.last_sent = None;
        self.overlay.last_stamp = None;
        self.overlay_send(json!({ "type": "overlay.clear" }));
        true
    }

    /// Switches the page's overlay on or off. Returns whether anything was sent.
    pub fn overlay_set_enabled(&mut self, enabled: bool) -> bool {
        if enabled == self.overlay.enabled || (enabled && !self.overlay_supported()) {
            return false;
        }
        self.overlay.enabled = enabled;
        self.overlay.last_sent = None;
        self.overlay.memo = None;
        self.overlay.held = false;
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
        let epoch = self.overlay.epoch + 1;
        #[cfg(test)]
        let pretend = self.overlay.pretend;
        self.overlay = OverlayHost {
            ack,
            epoch,
            #[cfg(test)]
            pretend,
            ..OverlayHost::default()
        };
    }

    /// Counts and keeps what the page reports about the overlay. `true` if `message` was one.
    pub fn overlay_note(&mut self, message: &Value) -> bool {
        match message["type"].as_str() {
            Some("overlay.error") => self.overlay.failure = Some(message["error"].clone()),
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
            "computed": self.overlay.computed,
            "adopted": self.overlay.adopted,
            "memo_hits": self.overlay.memo_hits,
            "held": self.overlay.held,
            "epoch": self.overlay.epoch,
            "last_phase": self.overlay.last_phase,
            "error": self.overlay.failure,
            "layout": self.overlay.layout,
            "drawn": self.overlay.drawn,
        })
    }
}

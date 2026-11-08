//! The debug performance meter (F049), window side: the shortcut, the floating card, the strip
//! that replaces it when no free corner exists, and the feeds that give the core's [`Meter`] its
//! frames.
//!
//! The rules (rates, worst interval, dropped playback frames, what each state reads like, where
//! the card may sit) live in `tessel_editor_core::perf` and need no window. This file only
//! translates: it records what the window paints, lays the card out, and answers the automation
//! tree.
//!
//! **Hidden means nothing.** While hidden no element is added to the tree, no timer exists, no
//! frame is recorded and no readout is computed; every feed returns on a single `bool`. The meter
//! never asks the window or the monitor for a frame of its own while frames are being produced:
//!
//! - UI frames are counted where they are *painted* (a zero-size `canvas` whose paint callback
//!   runs once per `Window::draw`), never from a timer tick;
//! - the text is recomputed from the frames that are painting anyway, at most every
//!   [`REFRESH_EVERY`] (4 Hz) and never because the meter asked;
//! - the only timer is a one-shot, armed only after something was painted, that fires once when
//!   the window came to rest to replace the numbers by "parado". The redraw it causes is marked
//!   as the meter's own and is not counted as a UI frame. Hiding the meter drops (cancels) that
//!   timer: a hidden meter never wakes the executor.
//!
//! Every time the meter reads is on the executor's clock ([`PerfClock`]): the wall clock in the
//! app, the controlled one in the GPUI tests, so the 4 Hz and rest rules are tested without
//! sleeping.
//!
//! The native WebView paints above every GPUI element, so a card over the picture would be
//! invisible. The card is placed by `perf::place` away from the WebView's rectangle; when no
//! corner is free (the monitor maximized) the meter becomes one line in the status bar.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::component::ActiveTheme;
use gpui_kit::component::h_flex;
use gpui_kit::*;
use serde_json::{Value, json};
use tessel_editor_core::CommandError;
use tessel_editor_core::perf::{
    self, CARD_HEIGHT, CARD_MARGIN, CARD_WIDTH, FrameContext, Meter, PerfMeterCommand, Placement,
    REFRESH_EVERY, Readout, Rect,
};

use super::tokens::Tokens;
use super::{Activity, Source, Workstation};

actions!(tessel_editor, [TogglePerfMeter]);

/// Geometry key of the rectangle the card floats over (the dock area, under the header).
const AREA: &str = "perf.area";
/// Geometry key of the native view's rectangle (the picture frame of the monitor).
const VIEWPORT: &str = "viewport.frame";

/// The workstation the shortcut talks to. Set by `Workstation::new`; the action is global so it
/// works with the keyboard focus in any panel, the header or nowhere in particular.
pub(super) struct PerfTarget(pub WeakEntity<Workstation>);

impl Global for PerfTarget {}

/// Binds the shortcut and its handler. Call once after `gpui_kit::init`.
pub fn install_perf_meter(cx: &mut App) {
    cx.bind_keys([KeyBinding::new(perf::SHORTCUT, TogglePerfMeter, None)]);
    cx.on_action(|_: &TogglePerfMeter, cx| {
        let Some(target) = cx
            .try_global::<PerfTarget>()
            .and_then(|target| target.0.upgrade())
        else {
            return;
        };
        target.update(cx, |workstation, cx| {
            let _ = workstation.execute_perf(PerfMeterCommand::Toggle, Source::Gesture, cx);
        });
    });
}

/// The shortcut as people read it on this platform.
pub fn shortcut_label() -> &'static str {
    if cfg!(target_os = "macos") {
        perf::SHORTCUT_LABEL_MAC
    } else {
        perf::SHORTCUT_LABEL_OTHER
    }
}

/// Time since the meter's epoch on the executor's clock. `BackgroundExecutor::now` is the wall
/// clock in the app and the controlled clock in GPUI tests (`advance_clock`), so the meter, its
/// rest timer and the tests all share one notion of "now".
#[derive(Clone)]
pub(super) struct PerfClock {
    source: ClockSource,
    epoch: Instant,
}

#[derive(Clone)]
enum ClockSource {
    Executor(BackgroundExecutor),
    /// For tests of pieces that run without an `App` (the viewport host's feed).
    #[cfg(test)]
    Wall,
}

impl ClockSource {
    fn now(&self) -> Instant {
        match self {
            Self::Executor(executor) => executor.now(),
            #[cfg(test)]
            Self::Wall => Instant::now(),
        }
    }
}

impl PerfClock {
    pub fn new(cx: &App) -> Self {
        let source = ClockSource::Executor(cx.background_executor().clone());
        let epoch = source.now();
        Self { source, epoch }
    }

    #[cfg(test)]
    pub fn wall() -> Self {
        Self {
            source: ClockSource::Wall,
            epoch: Instant::now(),
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.source.now().saturating_duration_since(self.epoch)
    }
}

pub(super) struct PerfState {
    meter: Rc<RefCell<Meter>>,
    clock: PerfClock,
    visible: bool,
    /// The next paint is the meter's own redraw (the "parado" transition): not a UI frame.
    own_redraw: Rc<Cell<bool>>,
    /// The one-shot rest timer is waiting.
    wake_armed: bool,
    /// The rest timer's task. Dropping it cancels the timer: hiding the meter does.
    wake_task: Option<Task<()>>,
    /// What the card shows: recomputed at most every `REFRESH_EVERY`.
    shown: Option<(Duration, Readout)>,
    placement: Placement,
    /// How many times the text was recomputed (the 4 Hz rule is about this number).
    #[cfg(test)]
    refreshes: u64,
    /// How many times the rest timer fired.
    #[cfg(test)]
    wakes: u64,
}

impl PerfState {
    pub fn new(cx: &App) -> Self {
        Self {
            meter: Rc::new(RefCell::new(Meter::default())),
            clock: PerfClock::new(cx),
            visible: false,
            own_redraw: Rc::new(Cell::new(false)),
            wake_armed: false,
            wake_task: None,
            shown: None,
            placement: Placement::Floating { y: CARD_MARGIN },
            #[cfg(test)]
            refreshes: 0,
            #[cfg(test)]
            wakes: 0,
        }
    }

    pub fn meter(&self) -> Rc<RefCell<Meter>> {
        self.meter.clone()
    }

    pub fn clock(&self) -> PerfClock {
        self.clock.clone()
    }

    pub fn elapsed(&self) -> Duration {
        self.clock.elapsed()
    }

    pub fn visible(&self) -> bool {
        self.visible
    }

    #[cfg(test)]
    pub fn wake_armed(&self) -> bool {
        self.wake_armed
    }

    #[cfg(test)]
    pub fn refreshes(&self) -> u64 {
        self.refreshes
    }

    #[cfg(test)]
    pub fn wakes(&self) -> u64 {
        self.wakes
    }

    #[cfg(test)]
    pub fn own_redraw_pending(&self) -> bool {
        self.own_redraw.get()
    }

    #[cfg(test)]
    pub fn wake_pending(&self) -> bool {
        self.wake_task.is_some()
    }
}

/// What `render_perf` hands to the workstation's render.
#[derive(Default)]
pub(super) struct PerfParts {
    pub area_probe: Option<AnyElement>,
    pub overlay: Option<AnyElement>,
    pub recorder: Option<AnyElement>,
    pub strip: Option<String>,
}

impl Workstation {
    pub fn perf_visible(&self) -> bool {
        self.perf.visible
    }

    /// Show, hide or toggle the meter. The shortcut, the header button and `ui invoke|set` all
    /// end here.
    pub fn execute_perf(
        &mut self,
        command: PerfMeterCommand,
        source: Source,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        let before = self.perf.visible;
        let visible = command.apply(before);
        if visible != before {
            self.perf.visible = visible;
            self.perf.shown = None;
            self.perf.placement = Placement::Floating { y: CARD_MARGIN };
            // A pending "the next paint is my own redraw" mark belongs to the state that set it:
            // left behind, it would swallow the first frame the window really paints after the
            // meter is shown again, since a hidden meter paints no recorder to spend it.
            self.perf.own_redraw.set(false);
            if visible {
                // Only a clock that is running counts: a paused playback is a rest (F049).
                let playing = self.playback.running_frame();
                let on_screen = self.viewport.presented_frame();
                self.perf
                    .meter
                    .borrow_mut()
                    .enable(self.perf.elapsed(), playing, on_screen);
            } else {
                self.perf.meter.borrow_mut().disable();
                // The rest timer is dropped with the meter: no wake-up while hidden.
                self.perf.wake_task = None;
                self.perf.wake_armed = false;
            }
        }
        self.activity = Some(Activity {
            source,
            text: format!(
                "Medidor de desempenho {} ({})",
                if visible { "ligado" } else { "desligado" },
                shortcut_label()
            ),
            error: false,
        });
        cx.notify();
        Ok(json!({
            "target": perf::TARGET, "visible": visible, "changed": visible != before,
            "shortcut": perf::SHORTCUT,
        }))
    }

    // ---- feeds from playback (single bool check while hidden)

    pub(super) fn perf_playback_started(&self, frame: i64) {
        self.perf.meter.borrow_mut().playback_started(
            self.perf.elapsed(),
            frame,
            self.viewport.presented_frame(),
        );
    }

    pub(super) fn perf_playback_clock(&self, frame: i64) {
        self.perf.meter.borrow_mut().playback_clock(frame);
    }

    pub(super) fn perf_playback_seeked(&self, frame: i64) {
        self.perf
            .meter
            .borrow_mut()
            .playback_seeked(frame, self.viewport.presented_frame());
    }

    pub(super) fn perf_playback_stopped(&self) {
        self.perf
            .meter
            .borrow_mut()
            .playback_stopped(self.perf.elapsed());
    }

    // ---- readout

    fn perf_readout(&self, now: Duration) -> Readout {
        let frame = self.viewport.last_frame().map(FrameContext::of);
        let rss = memory_stats::memory_stats().map(|usage| usage.physical_mem as u64);
        self.perf
            .meter
            .borrow()
            .readout(now, self.viewport.monitor_available(), frame, rss)
    }

    /// The meter's node: `visible`, the shortcut and, while shown, the live readout (computed for
    /// this request; the card itself refreshes at 4 Hz).
    pub(super) fn adopt_perf(&self, node: &mut Value) {
        if node["id"] != perf::TARGET {
            return;
        }
        node["label"] = json!(format!("Medidor de desempenho ({})", shortcut_label()));
        node["state"]["shortcut_label"] = json!(shortcut_label());
        node["state"]["placement"] = if !self.perf.visible {
            Value::Null
        } else {
            match self.perf.placement {
                Placement::Floating { y } => json!({ "kind": "floating", "y": y }),
                Placement::Strip => json!({ "kind": "status_bar_strip" }),
            }
        };
        node["state"]["readout"] = if self.perf.visible {
            self.perf_readout(self.perf.elapsed()).to_json()
        } else {
            // Hidden: nothing is collected, so there is nothing to read.
            Value::Null
        };
    }

    /// Wakes once when the window and the monitor came to rest, so "parado" replaces the numbers.
    pub(super) fn perf_wake(&mut self, cx: &mut Context<Self>) {
        #[cfg(test)]
        {
            self.perf.wakes += 1;
        }
        self.perf.wake_armed = false;
        if !self.perf.visible {
            return;
        }
        let now = self.perf.elapsed();
        let next = self.perf.meter.borrow().next_wakeup(now);
        match next {
            // Something painted meanwhile: wait for the new deadline, no redraw yet.
            Some(wait) => self.perf_arm(wait, cx),
            // Rest reached: one redraw, which is ours and is not counted as a UI frame.
            None => {
                self.perf.own_redraw.set(true);
                cx.notify();
            }
        }
    }

    fn perf_arm(&mut self, wait: Duration, cx: &mut Context<Self>) {
        if self.perf.wake_armed {
            return;
        }
        self.perf.wake_armed = true;
        // Kept, not detached: hiding the meter drops the task and so cancels the timer.
        self.perf.wake_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let _ = this.update(cx, |this, cx| this.perf_wake(cx));
        }));
    }

    // ---- drawing

    /// Everything the meter adds to this frame; nothing at all while hidden.
    pub(super) fn render_perf(&mut self, t: &Tokens, cx: &mut Context<Self>) -> PerfParts {
        if !self.perf.visible {
            return PerfParts::default();
        }
        let now = self.perf.elapsed();
        let stale = self
            .perf
            .shown
            .as_ref()
            .is_none_or(|(at, _)| now.saturating_sub(*at) >= REFRESH_EVERY);
        if stale {
            let readout = self.perf_readout(now);
            self.perf.shown = Some((now, readout));
            #[cfg(test)]
            {
                self.perf.refreshes += 1;
            }
        }
        let wake = self.perf.meter.borrow().next_wakeup(now);
        if let Some(wait) = wake {
            self.perf_arm(wait, cx);
        }

        // Where the native view is: the card must not hide behind it.
        let area = self.geometry.get(AREA);
        let viewport = self
            .viewport
            .on_screen
            .then(|| self.geometry.get(VIEWPORT))
            .flatten();
        self.perf.placement = match area {
            Some(area) => {
                let blocked: Vec<Rect> = viewport
                    .into_iter()
                    .map(|bounds| {
                        Rect::new(
                            f32::from(bounds.origin.x - area.origin.x),
                            f32::from(bounds.origin.y - area.origin.y),
                            f32::from(bounds.size.width),
                            f32::from(bounds.size.height),
                        )
                    })
                    .collect();
                perf::place(
                    (f32::from(area.size.width), f32::from(area.size.height)),
                    &blocked,
                )
            }
            None => Placement::Floating { y: CARD_MARGIN },
        };

        let readout = self
            .perf
            .shown
            .as_ref()
            .map(|(_, readout)| readout.clone())
            .expect("the readout was just computed");
        let overlay = match self.perf.placement {
            Placement::Floating { y } => {
                Some(card(t, cx.theme().mono_font_family.clone(), &readout, y).into_any_element())
            }
            Placement::Strip => None,
        };
        let strip = matches!(self.perf.placement, Placement::Strip).then(|| readout.strip());

        // Counts the frames the window really paints, and notices (in prepaint, after every probe
        // of this frame ran) that the layout moved under the card: that asks for one more frame
        // so the card lands where the native view is not.
        let meter = self.perf.meter.clone();
        let clock = self.perf.clock();
        let own = self.perf.own_redraw.clone();
        let geometry = self.geometry.clone();
        let used = (area, viewport);
        let on_screen = self.viewport.on_screen;
        let recorder = canvas(
            move |_, window, _| {
                let current = (
                    geometry.get(AREA),
                    on_screen.then(|| geometry.get(VIEWPORT)).flatten(),
                );
                if current != used {
                    window.request_animation_frame();
                }
            },
            move |_, _, _, _| {
                // The meter's own "parado" redraw is not a frame anyone asked for.
                if !own.replace(false) {
                    meter.borrow_mut().ui_painted(clock.elapsed());
                }
            },
        )
        .absolute()
        .size(px(0.));

        PerfParts {
            area_probe: Some(self.geometry.probe(AREA).into_any_element()),
            overlay,
            recorder: Some(recorder.into_any_element()),
            strip,
        }
    }

    /// The status-bar form of the meter, one monospace line.
    pub(super) fn perf_strip(
        &self,
        t: &Tokens,
        text: String,
        cx: &App,
    ) -> impl IntoElement + use<> {
        // It may shrink below its text (`min_w(0)`) and `truncate` then cuts it. The activity
        // and the jobs line do the same (`status_bar.rs`) with the same shrink factor, and the
        // save state never shrinks: everything stays on one line inside the window, even at
        // 960 px with the monitor maximized and a long error in the activity.
        div()
            .debug_selector(|| "perf-strip".to_string())
            .flex_shrink(1.)
            .min_w(px(0.))
            .truncate()
            .font_family(cx.theme().mono_font_family.clone())
            .text_color(t.text)
            .child(text)
    }
}

/// Width of the label column: `render_ms`, the longest label, is nine monospace glyphs.
const LABEL_WIDTH: f32 = 58.0;

/// The card: fixed size, monospace, and transparent to the pointer (no id, no listeners, so
/// GPUI inserts no hitbox and a click goes to whatever is under it).
pub(super) fn card(t: &Tokens, mono: SharedString, readout: &Readout, y: f32) -> Div {
    let rows = readout.rows().into_iter().map(|(label, value)| {
        h_flex()
            .gap(px(4.))
            .whitespace_nowrap()
            .child(
                div()
                    .w(px(LABEL_WIDTH))
                    .flex_shrink_0()
                    .text_color(t.dim)
                    .child(label),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .truncate()
                    .text_color(t.text)
                    .child(value),
            )
    });
    div()
        .absolute()
        .top(px(y))
        .right(px(CARD_MARGIN))
        .w(px(CARD_WIDTH))
        .h(px(CARD_HEIGHT))
        .px(px(6.))
        .py(px(6.))
        .flex()
        .flex_col()
        .gap(px(2.))
        .overflow_hidden()
        .rounded(px(4.))
        // Opaque: the panel it floats over must not show through the numbers.
        .bg(t.raised)
        .border_1()
        .border_color(t.border_strong)
        .font_family(mono)
        .text_size(px(10.))
        // 8 lines of 14 px and 7 gaps of 2 px fit the card (`CARD_HEIGHT`) with its padding.
        .line_height(px(14.))
        .child(
            div()
                .text_color(t.faint)
                .child(format!("depuração · {} fecha", shortcut_label())),
        )
        .children(rows)
}

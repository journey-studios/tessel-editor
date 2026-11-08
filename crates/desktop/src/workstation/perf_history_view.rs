//! The rolling performance history (F049 / BL145.2), window side: the commands, the diagnostic
//! panel and its native charts.
//!
//! What the numbers mean (buckets, gaps, rest versus stall, delta losses, latency) lives in
//! `tessel_editor_core::perf_history` and needs no window. This file only translates: it applies
//! [`HistoryCommand`]s to the recorder that [`super::perf_meter`] feeds, opens the panel and draws
//! one immutable [`Snapshot`] of the recorder.
//!
//! **Recording and visibility are separate.** The panel is a Root dialog (the Kit's own surface
//! for interactive controls; the floating card stays pass-through). Closing it never stops the
//! recording and starting a recording never opens it. While it is open the native WebView is
//! hidden like for any dialog (`Workflow::history_open` feeds `has_dialog`): the monitor keeps
//! producing pictures and the history keeps recording them (the context says whether the monitor
//! was on screen).
//!
//! **Cost.** The panel's only timer lives inside the open panel, runs at most at
//! [`REFRESH_EVERY`] (4 Hz) and only while the recording is moving: a frozen, stopped or closed
//! panel has none. The redraw it causes is marked as the diagnostic's own, so it is not counted
//! as a UI frame (`PerfState::own_redraw`). Hidden or closed, nothing here runs.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::button::Button;
use gpui_kit::component::dialog::DialogButtonProps;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, Disableable as _, Sizable, WindowExt as _, h_flex, v_flex};
use gpui_kit::*;
use serde_json::{Value, json};
use tessel_editor_core::CommandError;
use tessel_editor_core::perf::REFRESH_EVERY;
use tessel_editor_core::perf_history::{
    self, Agg, BUCKETS, EventKind, HistoryCommand, Inspection, Metric, Snapshot, describe, rate_fps,
};

use super::tokens::Tokens;
use super::{Activity, Source, Workstation};

/// Width of one 250 ms bucket on screen.
const BAR: f32 = 1.5;
const CHART_WIDTH: f32 = BAR * BUCKETS as f32;
const ROW_HEIGHT: f32 = 24.0;
const LABEL_WIDTH: f32 = 190.0;
const SUMMARY_WIDTH: f32 = 250.0;
const DIALOG_WIDTH: f32 = 900.0;

/// What the workstation keeps about the panel: the open one, if any.
#[derive(Default)]
pub(super) struct HistoryState {
    view: Option<Entity<HistoryView>>,
    /// The destination of the JSON export as typed or picked (a path entry, nothing more). Empty
    /// until the user gives one: nothing is exported to a place nobody chose.
    destination: String,
    /// The last export and how it ended. The file is written off the UI thread, so the command
    /// only says it started; this is where the end is observable (`get perf.history` and the panel).
    export: ExportStatus,
    /// Exports started so far: the id of the next one.
    exports_started: u64,
}

/// Where the export stands. Never carries the destination: the path lives only in the field.
#[derive(Clone, Default, PartialEq)]
pub(super) enum ExportStatus {
    #[default]
    Idle,
    /// The document was taken (a coherent copy) and the file is being written in the background.
    Writing {
        id: u64,
        source: Source,
    },
    Done {
        id: u64,
        bytes: usize,
    },
    Failed {
        id: u64,
        code: &'static str,
        message: String,
        details: Option<Value>,
    },
}

impl ExportStatus {
    fn to_json(&self) -> Value {
        match self {
            Self::Idle => json!({ "state": "idle" }),
            Self::Writing { id, .. } => json!({ "state": "writing", "id": id }),
            Self::Done { id, bytes } => json!({ "state": "done", "id": id, "bytes": bytes }),
            Self::Failed {
                id,
                code,
                message,
                details,
            } => json!({
                "state": "failed", "id": id,
                "error": { "code": code, "message": message, "details": details },
            }),
        }
    }

    fn writing(&self) -> bool {
        matches!(self, Self::Writing { .. })
    }
}

impl Workstation {
    /// Whether the panel is open (the monitor's WebView is hidden meanwhile).
    pub fn history_visible(&self) -> bool {
        self.workflow.history_open
    }

    /// The playhead was moved on purpose (a gesture or `ui set timeline.playhead`).
    pub(super) fn perf_playhead_moved(&self) {
        self.perf_playhead_moved_to(self.state.timeline.playhead);
    }

    pub(super) fn perf_playhead_moved_to(&self, frame: i64) {
        self.perf
            .meter()
            .borrow_mut()
            .playhead_moved(self.perf.elapsed(), frame);
    }

    /// Every path of the history (the panel's buttons, the header, `ui set|invoke perf.history`)
    /// ends here.
    pub fn execute_perf_history(
        &mut self,
        command: HistoryCommand,
        source: Source,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        let now = self.perf.elapsed();
        let result = match &command {
            HistoryCommand::Record(on) => Ok(self.history_record(*on, now)),
            HistoryCommand::Show => self.history_show(window, cx),
            HistoryCommand::Hide => Ok(self.history_hide(window, cx)),
            HistoryCommand::Destination(text) => {
                Ok(self.history_destination(text, !matches!(&source, Source::Gesture), window, cx))
            }
            // Both exports take the document now and write it in the background: the same path
            // for `export_to <path>`, the button and `invoke export`; only where the path comes from
            // differs.
            HistoryCommand::Export(path) => self.history_export(path.clone(), now, source, cx),
            HistoryCommand::ExportToDestination => {
                let path = std::path::PathBuf::from(self.perf.history.destination.trim());
                self.history_export(path, now, source, cx)
            }
            other => self
                .perf
                .meter()
                .borrow_mut()
                .recorder_mut()
                .apply(other, now),
        };
        self.activity = Some(match &result {
            Ok(outcome) => Activity {
                source,
                text: if outcome["state"] == "writing" {
                    // Not "exported": the write has not ended. The end is reported when it does.
                    "Histórico de desempenho: exportando…".to_string()
                } else {
                    format!(
                        "Histórico de desempenho: {}",
                        outcome["action"].as_str().unwrap_or("comando")
                    )
                },
                error: false,
            },
            Err(error) => Activity {
                source,
                text: error.to_string(),
                error: true,
            },
        });
        // The panel redraws from its own snapshot; asked later because the command may come from
        // one of its own buttons (it is being updated right now).
        if let Some(view) = self.perf.history.view.clone() {
            cx.defer(move |cx| {
                view.update(cx, |_, cx| cx.notify());
            });
        }
        cx.notify();
        result
    }

    /// Takes the document **now** (a coherent copy of the view, frozen or moving) and writes it on
    /// the background executor: the UI thread does no disk work, so a slow volume cannot freeze
    /// the window. What needs no disk is refused at once (nothing recorded, no or a relative
    /// path, an export already writing); everything about the file (exists, folder missing,
    /// temporary busy, a refusing volume) is the typed outcome of the write, observable in
    /// `get perf.history` -> `export`. One export at a time.
    fn history_export(
        &mut self,
        path: std::path::PathBuf,
        now: Duration,
        source: Source,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        if self.perf.history.export.writing() {
            return Err(CommandError::new(
                "perf_history_export_in_progress",
                "An export is still being written; wait for it to end (see `get perf.history`)",
            )
            .target(perf_history::TARGET));
        }
        let prepared = self
            .perf
            .meter()
            .borrow()
            .recorder()
            .prepare_export(now, path)?;
        self.perf.history.exports_started += 1;
        let id = self.perf.history.exports_started;
        self.perf.history.export = ExportStatus::Writing { id, source };
        let (frozen, range) = (prepared.frozen(), prepared.range());
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { prepared.write() })
                .await;
            let _ = this.update(cx, |this, cx| this.history_export_ended(id, result, cx));
        })
        .detach();
        Ok(json!({
            "target": perf_history::TARGET, "action": "export", "changed": false,
            "state": "writing", "export_id": id, "frozen": frozen, "range": range,
        }))
    }

    /// The background write ended: the outcome becomes observable and the status line says it.
    fn history_export_ended(
        &mut self,
        id: u64,
        result: Result<usize, perf_history::HistoryError>,
        cx: &mut Context<Self>,
    ) {
        let ExportStatus::Writing {
            id: waiting,
            source,
        } = self.perf.history.export
        else {
            return;
        };
        if waiting != id {
            return;
        }
        let (status, activity) = match result {
            Ok(bytes) => (
                ExportStatus::Done { id, bytes },
                Activity {
                    source,
                    text: "Histórico de desempenho: export concluído".to_string(),
                    error: false,
                },
            ),
            Err(error) => {
                let error = CommandError::from(error);
                let text = error.to_string();
                (
                    ExportStatus::Failed {
                        id,
                        code: error.code,
                        message: error.message,
                        details: error.details.map(|details| *details),
                    },
                    Activity {
                        source,
                        text,
                        error: true,
                    },
                )
            }
        };
        self.perf.history.export = status;
        self.activity = Some(activity);
        if let Some(view) = self.perf.history.view.clone() {
            cx.defer(move |cx| {
                view.update(cx, |_, cx| cx.notify());
            });
        }
        cx.notify();
    }

    fn history_record(&mut self, on: bool, now: Duration) -> Value {
        let changed = if on {
            let playing = self.playback.running_frame();
            let on_screen = self.viewport.presented_frame();
            self.perf
                .meter()
                .borrow_mut()
                .start_recording(now, playing, on_screen)
        } else {
            let stopped = self.perf.meter().borrow_mut().stop_recording(now);
            if stopped {
                // A refresh tick may have marked the next paint as the diagnostic's own and then
                // lost its probe to this stop before that paint: the mark must not swallow a real
                // frame of the next recording.
                self.perf.own_redraw().set(false);
            }
            stopped
        };
        json!({
            "target": perf_history::TARGET,
            "action": if on { "record_start" } else { "record_stop" },
            "changed": changed, "recording": on,
        })
    }

    fn history_show(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Value, CommandError> {
        if self.workflow.history_open {
            return Ok(json!({
                "target": perf_history::TARGET, "action": "show", "changed": false,
                "visible": true,
            }));
        }
        if self.workflow.has_dialog() {
            return Err(CommandError::new(
                "dialog_open",
                "Another dialog is open; close it before opening the performance history",
            )
            .target(perf_history::TARGET));
        }
        let this = cx.entity();
        let destination = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Caminho absoluto de um arquivo .json novo")
        });
        let remembered = self.perf.history.destination.clone();
        destination.update(cx, |input, cx| input.set_value(remembered, window, cx));
        let view = cx.new(|cx| HistoryView::new(this.clone(), destination, cx));
        self.history_mark_open(view.clone());
        let closing = this.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let closing = closing.clone();
            dialog
                .title("Histórico de desempenho")
                .w(px(DIALOG_WIDTH))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Fechar")
                        .show_cancel(false),
                )
                .child(view.clone())
                .on_close(move |_, window, cx| {
                    // Closing is not stopping: the recording goes on. The monitor's WebView comes
                    // back with the next render.
                    closing.update(cx, |workstation, cx| {
                        workstation.history_mark_closed();
                        cx.notify();
                    });
                    window.refresh();
                })
        });
        Ok(json!({
            "target": perf_history::TARGET, "action": "show", "changed": true, "visible": true,
        }))
    }

    /// Fills the export destination (typed in the field, picked, or `set ... export_destination`).
    /// It only remembers a path: nothing is written until the export button (or `invoke export`).
    fn history_destination(
        &mut self,
        text: &str,
        sync_field: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Value {
        let changed = self.perf.history.destination != text;
        self.perf.history.destination = text.to_string();
        // A panel-origin command already owns the view's update lease; its field is synced
        // by HistoryView::run. Only an external command may read the view here.
        if sync_field && let Some(view) = &self.perf.history.view {
            let input = view.read(cx).destination.clone();
            let shown = text.to_string();
            input.update(cx, |input, cx| input.set_value(shown, window, cx));
        }
        json!({
            "target": perf_history::TARGET, "action": "destination", "changed": changed,
            "export_destination": text,
        })
    }

    fn history_hide(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Value {
        let changed = self.workflow.history_open;
        if changed {
            self.history_mark_closed();
            window.close_dialog(cx);
            // Escape and the shortcuts only reach the workstation while it has focus.
            self.focus_root(window, cx);
        }
        json!({
            "target": perf_history::TARGET, "action": "hide", "changed": changed,
            "visible": false,
        })
    }

    /// The panel's state without a window (what `show` does around the dialog).
    pub(super) fn history_mark_open(&mut self, view: Entity<HistoryView>) {
        self.workflow.history_open = true;
        self.perf.history.view = Some(view);
    }

    pub(super) fn history_mark_closed(&mut self) {
        self.workflow.history_open = false;
        self.perf.history.view = None;
    }

    /// `get`/`snapshot` of the history: flags, window, peaks per metric and the newest events.
    pub(super) fn adopt_perf_history(&self, node: &mut Value) {
        if node["id"] != perf_history::TARGET {
            return;
        }
        let meter = self.perf.meter();
        let meter = meter.borrow();
        let state = meter.recorder().state_json(self.perf.elapsed());
        if let Some(fields) = state.as_object() {
            for (key, value) in fields {
                node["state"][key] = value.clone();
            }
        }
        node["state"]["visible"] = json!(self.workflow.history_open);
        node["state"]["card_visible"] = json!(meter.card_visible());
        node["state"]["export_destination"] = json!(self.perf.history.destination);
        node["state"]["export"] = self.perf.history.export.to_json();
        node["state"]["refresh_hz"] = json!(1000 / REFRESH_EVERY.as_millis().max(1));
        node["label"] = json!("Histórico de desempenho (60 s)");
    }
}

// ---------------------------------------------------------------------------- the panel

pub(super) struct HistoryView {
    workstation: Entity<Workstation>,
    /// Where the charts are, written in prepaint and read by the pointer handler.
    chart: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// The refresh timer: exists only while the recording moves. Dropping it cancels it.
    timer: Option<Task<()>>,
    /// The answer to the last button: an export path or an error, in words.
    note: Option<(String, bool)>,
    /// Where the JSON goes: a path entry, mirrored in `HistoryState::destination`.
    destination: Entity<InputState>,
    /// How many refreshes the timer asked for (the 4 Hz rule is about this number).
    #[cfg(test)]
    ticks: u64,
}

impl HistoryView {
    pub(super) fn new(
        workstation: Entity<Workstation>,
        destination: Entity<InputState>,
        cx: &mut Context<Self>,
    ) -> Self {
        // Typing is the only way the field changes by itself (`set_value` emits nothing), so the
        // workstation hears every edit and `get` can say what the field holds.
        cx.subscribe(&destination, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let text = input.read(cx).value().to_string();
                this.workstation.update(cx, |workstation, _| {
                    workstation.perf.history.destination = text;
                });
            }
        })
        .detach();
        Self {
            workstation,
            chart: Rc::new(Cell::new(None)),
            timer: None,
            note: None,
            destination,
            #[cfg(test)]
            ticks: 0,
        }
    }

    /// What the destination field shows (tests).
    #[cfg(test)]
    pub(super) fn destination_text(&self, cx: &App) -> String {
        self.destination.read(cx).value().to_string()
    }

    /// Types into the destination field as a person would (tests): the field emits the change
    /// the workstation listens to, so the subscription is what is being exercised.
    #[cfg(test)]
    pub(super) fn type_destination(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = text.to_string();
        self.destination.update(cx, |input, cx| {
            input.set_value(text, window, cx);
            cx.emit(InputEvent::Change);
        });
    }

    /// Whether a timer is armed (tests).
    #[cfg(test)]
    pub(super) fn timer_armed(&self) -> bool {
        self.timer.is_some()
    }

    #[cfg(test)]
    pub(super) fn ticks(&self) -> u64 {
        self.ticks
    }

    fn ensure_timer(&mut self, moving: bool, cx: &mut Context<Self>) {
        if !moving {
            self.timer = None;
            return;
        }
        if self.timer.is_some() {
            return;
        }
        self.timer = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(REFRESH_EVERY).await;
                if this.update(cx, |this, cx| this.tick(cx)).is_err() {
                    break;
                }
            }
        }));
    }

    /// One 4 Hz refresh: the next paint is the diagnostic's own and is not a UI frame.
    fn tick(&mut self, cx: &mut Context<Self>) {
        #[cfg(test)]
        {
            self.ticks += 1;
        }
        let workstation = self.workstation.read(cx);
        if workstation.perf.meter().borrow().enabled() {
            workstation.perf.own_redraw().set(true);
        }
        cx.notify();
    }

    pub(super) fn run(
        &mut self,
        command: HistoryCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let HistoryCommand::Destination(text) = &command {
            self.destination
                .update(cx, |input, cx| input.set_value(text.clone(), window, cx));
        }
        let result = self.workstation.update(cx, |workstation, cx| {
            workstation.execute_perf_history(command, Source::Gesture, window, cx)
        });
        if let Err(error) = result {
            self.note = Some((error.to_string(), true));
        } else {
            self.note = None;
        }
        cx.notify();
    }

    /// The export button: writes to the destination the user gave, through the same command as
    /// `invoke perf.history export`. A refusal that needs no disk (no destination, a relative
    /// path) is shown in words at once; the write itself runs in the background and its end
    /// (new file, or the typed reason it was refused) shows in the panel when it arrives.
    fn export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let result = self.workstation.update(cx, |workstation, cx| {
            workstation.execute_perf_history(
                HistoryCommand::ExportToDestination,
                Source::Gesture,
                window,
                cx,
            )
        });
        self.note = match result {
            Ok(_) => None,
            Err(error) => Some((error.to_string(), true)),
        };
        cx.notify();
    }

    /// The native save dialog as a path entry: it only fills the field; writing is the export
    /// button's job. Cancelling changes nothing.
    fn pick_destination(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let base = self
            .workstation
            .read(cx)
            .state
            .project
            .workspace
            .clone()
            .unwrap_or_else(std::env::temp_dir);
        let receiver = cx.prompt_for_new_path(&base, Some("tessel-perf-history.json"));
        cx.spawn_in(window, async move |this, cx| {
            let Some(path) = receiver.await.ok().and_then(Result::ok).flatten() else {
                return;
            };
            let text = path.to_string_lossy().into_owned();
            let _ = this.update_in(cx, |this, window, cx| {
                this.run(HistoryCommand::Destination(text), window, cx);
            });
        })
        .detach();
    }

    /// A click on a chart selects the instant under it (never one before the recording began).
    fn select_at(&mut self, x: f32, window: &mut Window, cx: &mut Context<Self>) {
        let now = self.workstation.read(cx).perf.elapsed();
        let snapshot = self
            .workstation
            .read(cx)
            .perf
            .meter()
            .borrow()
            .recorder()
            .view(now);
        let Some(snapshot) = snapshot else {
            return;
        };
        let bucket = ((x / BAR).floor().max(0.0) as usize).min(BUCKETS - 1);
        let start_ms = snapshot.bucket_start_ms(bucket);
        if start_ms < 0.0 || !snapshot.observed(bucket) {
            self.note = Some((
                "Antes do início da gravação: nada foi observado.".into(),
                false,
            ));
            cx.notify();
            return;
        }
        let at = Duration::from_secs_f64((start_ms + 125.0) / 1000.0);
        self.run(HistoryCommand::Select(Some(at)), window, cx);
    }
}

/// What the panel draws, read once per render from the recorder.
struct Reading {
    recording: bool,
    has_data: bool,
    frozen: bool,
    snapshot: Option<Rc<Snapshot>>,
    inspection: Option<Inspection>,
    selected_bucket: Option<usize>,
    monitor_on_screen: Option<bool>,
    destination: String,
    /// The last export, in words: `(text, is_error)`; `writing` disables the button.
    export: Option<(String, bool)>,
    export_writing: bool,
}

/// The last export in words for the panel, independent of the editable destination field.
/// The status JSON and the completion note never infer a path from that field.
fn export_note(status: &ExportStatus) -> Option<(String, bool)> {
    match status {
        ExportStatus::Idle => None,
        ExportStatus::Writing { .. } => {
            Some(("Exportando… (gravando em segundo plano)".into(), false))
        }
        ExportStatus::Done { bytes, .. } => Some((
            format!("Exportação concluída: {bytes} bytes. Arquivo novo: nada foi substituído."),
            false,
        )),
        ExportStatus::Failed { code, message, .. } => {
            Some((format!("Exportação recusada ({code}): {message}"), true))
        }
    }
}

impl Render for HistoryView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Tokens::new(cx);
        let mono = cx.theme().mono_font_family.clone();
        let reading = {
            let workstation = self.workstation.read(cx);
            let now = workstation.perf.elapsed();
            let meter = workstation.perf.meter();
            let meter = meter.borrow();
            let recorder = meter.recorder();
            let snapshot = recorder.view(now).map(Rc::new);
            let selected_bucket = recorder
                .selected()
                .and_then(|at| snapshot.as_ref().and_then(|s| s.index_at(at)));
            Reading {
                recording: recorder.recording(),
                has_data: recorder.has_data(),
                frozen: recorder.frozen().is_some(),
                inspection: recorder.inspection(now),
                selected_bucket,
                monitor_on_screen: recorder.context().and_then(|c| c.monitor_on_screen),
                destination: workstation.perf.history.destination.clone(),
                export: export_note(&workstation.perf.history.export),
                export_writing: workstation.perf.history.export.writing(),
                snapshot,
            }
        };
        self.ensure_timer(reading.recording && !reading.frozen, cx);

        let status = match (reading.recording, reading.frozen, reading.has_data) {
            (true, false, _) => "Gravando · janela móvel de 60 s",
            (true, true, _) => "Congelado · instantâneo imutável (a gravação continua por baixo)",
            (false, true, _) => "Congelado · gravação parada",
            (false, false, true) => "Parado · os dados continuam legíveis até limpar",
            (false, false, false) => "Desligado · nada é gravado",
        };

        let toolbar = h_flex()
            .gap_2()
            .items_center()
            .flex_wrap()
            .child(
                Button::new("history-record")
                    .small()
                    .label(if reading.recording { "Parar" } else { "Gravar" })
                    .accessibility_label(if reading.recording {
                        "Parar a gravação"
                    } else {
                        "Iniciar a gravação"
                    })
                    .on_click(cx.listener({
                        let recording = reading.recording;
                        move |this, _, window, cx| {
                            this.run(HistoryCommand::Record(!recording), window, cx);
                        }
                    })),
            )
            .child(
                Button::new("history-freeze")
                    .small()
                    .label(if reading.frozen {
                        "Retomar"
                    } else {
                        "Congelar"
                    })
                    .accessibility_label(if reading.frozen {
                        "Retomar o histórico em movimento"
                    } else {
                        "Congelar um instantâneo para inspeção"
                    })
                    .disabled(!reading.has_data)
                    .tooltip(if reading.has_data {
                        "Fixa os últimos 60 s para inspecionar; a gravação continua"
                    } else {
                        "Indisponível: não há gravação para congelar"
                    })
                    .on_click(cx.listener({
                        let frozen = reading.frozen;
                        move |this, _, window, cx| {
                            let command = if frozen {
                                HistoryCommand::Resume
                            } else {
                                HistoryCommand::Freeze
                            };
                            this.run(command, window, cx);
                        }
                    })),
            )
            .child(
                Button::new("history-clear-selection")
                    .small()
                    .label("Limpar seleção")
                    .accessibility_label("Limpar o instante selecionado")
                    .disabled(reading.selected_bucket.is_none())
                    .tooltip(if reading.selected_bucket.is_some() {
                        "Remove a seleção"
                    } else {
                        "Indisponível: nenhum instante selecionado"
                    })
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.run(HistoryCommand::Select(None), window, cx);
                    })),
            )
            .child(
                Button::new("history-clear")
                    .small()
                    .label("Limpar histórico")
                    .accessibility_label("Esquecer o histórico gravado")
                    .disabled(!reading.has_data)
                    .tooltip(if reading.has_data {
                        "Esquece o que foi gravado; o projeto não é tocado"
                    } else {
                        "Indisponível: não há histórico"
                    })
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.run(HistoryCommand::Clear, window, cx);
                    })),
            )
            .child(
                Button::new("history-export")
                    .small()
                    .label("Exportar JSON")
                    .accessibility_label("Exportar o histórico como JSON local")
                    .disabled(
                        !reading.has_data
                            || reading.destination.trim().is_empty()
                            || reading.export_writing,
                    )
                    .tooltip(if reading.export_writing {
                        "Indisponível: uma exportação ainda está sendo gravada"
                    } else if !reading.has_data {
                        "Indisponível: não há histórico para exportar"
                    } else if reading.destination.trim().is_empty() {
                        "Indisponível: informe o destino do arquivo ao lado"
                    } else {
                        "Grava um JSON novo no destino informado (sem caminhos nem textos do projeto)"
                    })
                    .on_click(cx.listener(|this, _, window, cx| this.export(window, cx))),
            );
        let destination = h_flex()
            .gap_2()
            .items_center()
            .child(
                div()
                    .w(px(LABEL_WIDTH))
                    .flex_shrink_0()
                    .text_size(px(11.))
                    .text_color(t.dim)
                    .child("Destino do JSON (caminho absoluto)"),
            )
            .child(div().flex_1().child(Input::new(&self.destination)))
            .child(
                Button::new("history-export-pick")
                    .small()
                    .label("Escolher…")
                    .accessibility_label("Escolher o arquivo de destino da exportação")
                    .tooltip("Abre o seletor do sistema; ele só preenche o caminho")
                    .on_click(cx.listener(|this, _, window, cx| this.pick_destination(window, cx))),
            );

        let rows = match &reading.snapshot {
            Some(snapshot) => Metric::ALL
                .into_iter()
                .map(|metric| self.metric_row(metric, snapshot, &reading, &t, mono.clone(), cx))
                .collect::<Vec<_>>(),
            None => Vec::new(),
        };
        let empty = reading.snapshot.is_none().then(|| {
            div().py_4().text_size(px(12.)).text_color(t.dim).child(
                "Sem dados. Use Gravar: o histórico começa vazio e nada é inventado antes disso.",
            )
        });
        let events = reading
            .snapshot
            .as_ref()
            .map(|snapshot| self.events_row(snapshot, &reading, &t, cx));
        let inspection = reading
            .inspection
            .as_ref()
            .zip(reading.snapshot.as_ref())
            .map(|(inspection, snapshot)| inspection_block(inspection, snapshot, &t, mono.clone()));
        let footer = reading
            .snapshot
            .as_ref()
            .map(|snapshot| footer(snapshot, reading.monitor_on_screen, &t, mono.clone()));
        // Selection/button feedback and the asynchronous export are independent outcomes:
        // neither may hide the other, including a write that finished after a later selection.
        let note = v_flex().children(self.note.iter().chain(reading.export.iter()).map(
            |(text, error)| {
                div()
                    .text_size(px(11.))
                    .text_color(if *error { t.bad } else { t.dim })
                    .child(text.clone())
            },
        ));

        v_flex()
            .gap_2()
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(t.text)
                    .child(status),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(t.dim)
                    .child("Memória local deste processo. Nada vai ao projeto, ao desfazer, ao autosave nem à rede. Fechar o painel não para a gravação."),
            )
            .child(toolbar)
            .child(destination)
            .child(note)
            .children(empty)
            .children(rows)
            .children(events)
            .children(inspection)
            .children(footer)
    }
}

impl HistoryView {
    fn chart(
        &self,
        snapshot: &Rc<Snapshot>,
        reading: &Reading,
        t: &Tokens,
        paint: impl Fn(Bounds<Pixels>, &mut Window, &Snapshot, Option<usize>, &Tokens) + 'static,
        cx: &mut Context<Self>,
    ) -> Div {
        let shared = self.chart.clone();
        let snapshot = snapshot.clone();
        let selected = reading.selected_bucket;
        let tokens = *t;
        div()
            .w(px(CHART_WIDTH))
            .h(px(ROW_HEIGHT))
            .flex_shrink_0()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    let Some(bounds) = this.chart.get() else {
                        return;
                    };
                    let x = f32::from(event.position.x - bounds.origin.x);
                    this.select_at(x, window, cx);
                }),
            )
            .child(
                canvas(
                    move |bounds, _, _| shared.set(Some(bounds)),
                    move |bounds, _, window, _| paint(bounds, window, &snapshot, selected, &tokens),
                )
                .size_full(),
            )
    }

    fn metric_row(
        &self,
        metric: Metric,
        snapshot: &Rc<Snapshot>,
        reading: &Reading,
        t: &Tokens,
        mono: SharedString,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let scale = scale_of(snapshot, metric);
        let color = color_of(metric, t);
        let chart = self.chart(
            snapshot,
            reading,
            t,
            move |bounds, window, snapshot, selected, t| {
                paint_series(bounds, window, snapshot, metric, scale, color, selected, t);
            },
            cx,
        );
        h_flex()
            .gap_2()
            .items_center()
            .child(
                div()
                    .w(px(LABEL_WIDTH))
                    .flex_shrink_0()
                    .truncate()
                    .text_size(px(11.))
                    .text_color(t.dim)
                    .child(metric.label()),
            )
            .child(chart)
            .child(
                div()
                    .w(px(SUMMARY_WIDTH))
                    .flex_shrink_0()
                    .truncate()
                    .font_family(mono)
                    .text_size(px(10.))
                    .text_color(t.text)
                    .child(summary_text(metric, snapshot, scale)),
            )
    }

    fn events_row(
        &self,
        snapshot: &Rc<Snapshot>,
        reading: &Reading,
        t: &Tokens,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let chart = self.chart(
            snapshot,
            reading,
            t,
            |bounds, window, snapshot, selected, t| {
                paint_events(bounds, window, snapshot, selected, t);
            },
            cx,
        );
        h_flex()
            .gap_2()
            .items_center()
            .child(
                div()
                    .w(px(LABEL_WIDTH))
                    .flex_shrink_0()
                    .text_size(px(11.))
                    .text_color(t.dim)
                    .child("Eventos e sessões"),
            )
            .child(chart)
            .child(
                div()
                    .w(px(SUMMARY_WIDTH))
                    .flex_shrink_0()
                    .truncate()
                    .text_size(px(10.))
                    .text_color(t.dim)
                    .child(format!(
                        "{} eventos · {} sessão(ões)",
                        snapshot.events.len(),
                        snapshot.segments.len().max(1)
                    )),
            )
    }
}

// ---------------------------------------------------------------------------- drawing

/// The height a bucket stands for: a rate for frame counters, the sum for losses, the mean of a
/// gauge and the worst sample of a wait.
fn level(metric: Metric, agg: &Agg) -> f64 {
    match metric {
        Metric::UiFrames | Metric::MonitorFrames => rate_fps(agg.count),
        Metric::LostFrames => agg.sum,
        Metric::RssBytes | Metric::TextureBytes => agg.mean().unwrap_or(0.0),
        _ => agg.max,
    }
}

/// Every row has its own scale (a scale is never shared between units): zero to the largest bar.
fn scale_of(snapshot: &Snapshot, metric: Metric) -> f64 {
    snapshot
        .series(metric)
        .flatten()
        .map(|agg| level(metric, agg))
        .fold(0.0, f64::max)
        .max(f64::EPSILON)
}

fn color_of(metric: Metric, t: &Tokens) -> Hsla {
    match metric {
        Metric::UiFrames | Metric::UiIntervalMs => t.accent,
        Metric::MonitorFrames | Metric::MonitorIntervalMs => t.video_border,
        Metric::RenderMs | Metric::LatencyMs => t.title_border,
        Metric::LostFrames => t.bad,
        Metric::RssBytes | Metric::TextureBytes => t.audio_border,
    }
}

fn quad(window: &mut Window, x: f32, y: f32, width: f32, height: f32, color: Hsla) {
    window.paint_quad(fill(
        Bounds::new(point(px(x), px(y)), size(px(width), px(height))),
        color,
    ));
}

/// The recorded range as a lane, then one bar per observed bucket. A bucket that was not
/// observed has no bar (a gap, never a zero); a zero is a one-pixel bar.
#[allow(clippy::too_many_arguments)]
fn paint_series(
    bounds: Bounds<Pixels>,
    window: &mut Window,
    snapshot: &Snapshot,
    metric: Metric,
    scale: f64,
    color: Hsla,
    selected: Option<usize>,
    t: &Tokens,
) {
    let (x0, y0) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    let height = f32::from(bounds.size.height);
    paint_lane(window, x0, y0, height, snapshot, t);
    for (index, agg) in snapshot.series(metric).enumerate() {
        let Some(agg) = agg else {
            continue;
        };
        let bar = ((level(metric, agg) / scale) as f32 * height).clamp(1.0, height);
        quad(
            window,
            x0 + index as f32 * BAR,
            y0 + height - bar,
            BAR - 0.5,
            bar,
            color,
        );
    }
    paint_selection(window, x0, y0, height, selected, t);
}

fn paint_lane(window: &mut Window, x0: f32, y0: f32, height: f32, snapshot: &Snapshot, t: &Tokens) {
    let first = (0..BUCKETS).find(|index| snapshot.observed(*index));
    let last = (0..BUCKETS).rev().find(|index| snapshot.observed(*index));
    if let (Some(first), Some(last)) = (first, last) {
        quad(
            window,
            x0 + first as f32 * BAR,
            y0,
            (last - first + 1) as f32 * BAR,
            height,
            t.lane_alt,
        );
    }
}

fn paint_selection(
    window: &mut Window,
    x0: f32,
    y0: f32,
    height: f32,
    selected: Option<usize>,
    t: &Tokens,
) {
    if let Some(index) = selected {
        quad(window, x0 + index as f32 * BAR, y0, BAR, height, t.playhead);
    }
}

fn event_color(kind: EventKind, t: &Tokens) -> Hsla {
    match kind {
        EventKind::RequestFailed
        | EventKind::ContextLost
        | EventKind::RuntimeUnavailable
        | EventKind::ExportFailed => t.bad,
        EventKind::PlaybackStarted | EventKind::PlaybackStopped => t.ok,
        EventKind::Seek => t.accent,
        EventKind::ProjectReplaced | EventKind::SceneChanged | EventKind::ViewportReloaded => {
            t.selected
        }
        _ => t.dim,
    }
}

/// Session edges as full-height rules (nothing is measured across them) and one marker per event.
fn paint_events(
    bounds: Bounds<Pixels>,
    window: &mut Window,
    snapshot: &Snapshot,
    selected: Option<usize>,
    t: &Tokens,
) {
    let (x0, y0) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
    let height = f32::from(bounds.size.height);
    paint_lane(window, x0, y0, height, snapshot, t);
    for segment in snapshot.segments.iter().filter(|segment| segment.id > 0) {
        if let Some(index) = snapshot.index_at(segment.start) {
            quad(
                window,
                x0 + index as f32 * BAR,
                y0,
                1.0,
                height,
                t.border_strong,
            );
        }
    }
    for event in &snapshot.events {
        if let Some(index) = snapshot.index_at(event.at) {
            quad(
                window,
                x0 + index as f32 * BAR,
                y0 + height * 0.2,
                BAR + 1.0,
                height * 0.6,
                event_color(event.kind, t),
            );
        }
    }
    paint_selection(window, x0, y0, height, selected, t);
}

// ---------------------------------------------------------------------------- text

fn comma(text: String) -> String {
    text.replace('.', ",")
}

fn summary_text(metric: Metric, snapshot: &Snapshot, scale: f64) -> String {
    let Some(summary) = snapshot.summary(metric) else {
        return "sem amostra".to_string();
    };
    match metric {
        Metric::UiFrames | Metric::MonitorFrames => {
            let peak = snapshot
                .series(metric)
                .flatten()
                .map(|agg| rate_fps(agg.count))
                .fold(0.0, f64::max);
            format!(
                "pico {} fps · {} quadros",
                comma(format!("{peak:.0}")),
                summary.count
            )
        }
        Metric::LostFrames => {
            let total: f64 = snapshot.series(metric).flatten().map(|agg| agg.sum).sum();
            format!("{total:.0} perdidos · pico/250 ms {scale:.0}")
        }
        _ => format!(
            "mín {} · méd {} · máx {}",
            metric.format(summary.min),
            metric.format(summary.mean),
            metric.format(summary.max)
        ),
    }
}

fn relative(snapshot: &Snapshot, at_ms: f64) -> String {
    let end = snapshot.captured_at.as_secs_f64() * 1000.0;
    comma(format!("{:.1} s", (at_ms - end) / 1000.0))
}

fn inspection_block(
    inspection: &Inspection,
    snapshot: &Snapshot,
    t: &Tokens,
    mono: SharedString,
) -> impl IntoElement + use<> {
    let head = format!(
        "Instante selecionado: {} a {} (balde de 250 ms){}",
        relative(snapshot, inspection.start_ms),
        relative(snapshot, inspection.end_ms),
        if inspection.observed {
            ""
        } else {
            " · fora do intervalo gravado"
        },
    );
    let lines = Metric::ALL.into_iter().map(|metric| {
        let agg = &inspection.metrics[metric.index()];
        div()
            .whitespace_nowrap()
            .child(format!("{}: {}", metric.label(), describe(metric, agg)))
    });
    let events = inspection.events.iter().map(|event| {
        let mut text = format!(
            "{} · {}",
            relative(snapshot, event.at.as_secs_f64() * 1000.0),
            event.kind.label()
        );
        if let Some(frame) = event.frame {
            text += &format!(" · quadro {frame}");
        }
        if let Some(detail) = &event.detail {
            text += &format!(" · {detail}");
        }
        if event.repeat > 1 {
            text += &format!(" · ×{}", event.repeat);
        }
        div()
            .whitespace_nowrap()
            .text_color(t.title_border)
            .child(text)
    });
    v_flex()
        .gap_1()
        .p_2()
        .rounded(px(4.))
        .border_1()
        .border_color(t.border)
        .font_family(mono)
        .text_size(px(10.))
        .text_color(t.text)
        .child(div().text_color(t.dim).child(head))
        .children(lines)
        .child(
            div()
                .text_color(t.dim)
                .child("Eventos correlacionados (do início da espera até o balde):"),
        )
        .children(events)
        .children(inspection.events.is_empty().then(|| {
            div()
                .text_color(t.faint)
                .child("nenhum evento nessa janela")
        }))
}

fn footer(
    snapshot: &Snapshot,
    monitor_on_screen: Option<bool>,
    t: &Tokens,
    mono: SharedString,
) -> impl IntoElement + use<> {
    let counters = snapshot.counters;
    let mut parts = vec![format!(
        "respostas obsoletas {} · imagens sem pedido {} · pedidos esquecidos {} · amostras tardias {} · memória indisponível {} · eventos descartados {}",
        counters.stale_responses,
        counters.unmatched_images,
        counters.forgotten_requests,
        counters.late_samples,
        counters.memory_unavailable,
        snapshot.dropped_events
    )];
    if let Some(context) = &snapshot.context {
        parts.push(format!(
            "quadro {}×{}{} · camadas proxy {}",
            context.width,
            context.height,
            context
                .quality
                .as_ref()
                .map(|quality| format!(" ({quality})"))
                .unwrap_or_default(),
            context.proxy_layers
        ));
    }
    if monitor_on_screen == Some(false) {
        parts.push("o monitor estava escondido (diálogo aberto): a gravação continua".to_string());
    }
    v_flex()
        .gap_1()
        .font_family(mono)
        .text_size(px(10.))
        .text_color(t.dim)
        .children(parts.into_iter().map(|text| div().child(text)))
}

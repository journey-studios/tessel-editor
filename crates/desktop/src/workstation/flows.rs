//! What each file/import/package/export dialog asks for and the operation it sends.
//!
//! Nothing here knows GPUI: a dialog is a [`FlowKind`], a list of fields and a function that turns
//! the typed values into the same JSON operation a script would send (`project.open`,
//! `media.import`, `project.package`, `export.start`, …). That is what keeps the dialog, the
//! automation tree and the CLI on one set of rules; the dialog only collects strings.

use serde_json::{Value, json};
use tessel_editor_core::CommandError;
use tessel_editor_core::preset::BUILTIN_IDS;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FlowKind {
    Open,
    SaveAs,
    Recover,
    Import,
    ImportProject,
    Package,
    Export,
}

impl FlowKind {
    pub const ALL: [FlowKind; 7] = [
        Self::Open,
        Self::SaveAs,
        Self::Recover,
        Self::Import,
        Self::ImportProject,
        Self::Package,
        Self::Export,
    ];

    /// Stable suffix of the automation IDs (`flow.<id>`).
    pub fn id(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::SaveAs => "save_as",
            Self::Recover => "recover",
            Self::Import => "import",
            Self::ImportProject => "import_project",
            Self::Package => "package",
            Self::Export => "export",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.id() == id)
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Open => "Abrir projeto",
            Self::SaveAs => "Salvar projeto como",
            Self::Recover => "Recuperar projeto",
            Self::Import => "Importar mídia",
            Self::ImportProject => "Importar de outro projeto",
            Self::Package => "Empacotar projeto",
            Self::Export => "Exportar vídeo",
        }
    }

    pub fn submit_label(self) -> &'static str {
        match self {
            Self::Open => "Abrir",
            Self::SaveAs => "Salvar",
            Self::Recover => "Recuperar",
            Self::Import => "Importar",
            Self::ImportProject => "Importar",
            Self::Package => "Empacotar",
            Self::Export => "Exportar",
        }
    }

    pub fn fields(self) -> &'static [FieldSpec] {
        match self {
            Self::Open => &[PATH_OPEN],
            Self::SaveAs => &[PATH_SAVE],
            Self::Recover => &[PATH_OPEN],
            Self::Import => &[PATHS, MODE],
            Self::ImportProject => &[SOURCE],
            Self::Package => &[PACKAGE_PATH, PACKAGE_FORMAT],
            Self::Export => &[PRESET, DESTINATION],
        }
    }

    /// Whether the dialog shows a report (preflight or recovery listing) it can refresh.
    pub fn has_report(self) -> bool {
        matches!(self, Self::Export | Self::Package | Self::Recover)
    }
}

/// How the native picker next to a field behaves.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Picker {
    None,
    Files,
    File,
    NewFile(&'static str),
}

#[derive(Clone, Copy, Debug)]
pub struct FieldSpec {
    pub name: &'static str,
    pub label: &'static str,
    pub placeholder: &'static str,
    pub picker: Picker,
    /// Options of a choice field; an empty list makes the field free text.
    pub options: &'static [&'static str],
}

const PATH_OPEN: FieldSpec = FieldSpec {
    name: "path",
    label: "Documento (.tessel.json)",
    placeholder: "/caminho/projeto.tessel.json",
    picker: Picker::File,
    options: &[],
};
const PATH_SAVE: FieldSpec = FieldSpec {
    name: "path",
    label: "Salvar em",
    placeholder: "/caminho/projeto.tessel.json",
    picker: Picker::NewFile("projeto.tessel.json"),
    options: &[],
};
const PATHS: FieldSpec = FieldSpec {
    name: "paths",
    label: "Arquivos (separados por ;)",
    placeholder: "/caminho/a.mp4;/caminho/b.wav",
    picker: Picker::Files,
    options: &[],
};
const MODE: FieldSpec = FieldSpec {
    name: "mode",
    label: "Modo",
    placeholder: "copy",
    picker: Picker::None,
    options: &["copy", "reference"],
};
const SOURCE: FieldSpec = FieldSpec {
    name: "source",
    label: "Projeto de origem (.tessel.json)",
    placeholder: "/caminho/outro.tessel.json",
    picker: Picker::File,
    options: &[],
};
const PACKAGE_PATH: FieldSpec = FieldSpec {
    name: "path",
    label: "Destino do pacote",
    placeholder: "/caminho/projeto.tesselpack",
    picker: Picker::NewFile("projeto.tesselpack"),
    options: &[],
};
const PACKAGE_FORMAT: FieldSpec = FieldSpec {
    name: "format",
    label: "Formato",
    placeholder: "archive",
    picker: Picker::None,
    options: &["archive", "directory"],
};
const PRESET: FieldSpec = FieldSpec {
    name: "preset",
    label: "Preset",
    placeholder: "fullhd",
    picker: Picker::None,
    // The built-ins; the dialog appends the project's own presets when it opens.
    options: &BUILTIN_IDS,
};
const DESTINATION: FieldSpec = FieldSpec {
    name: "destination",
    label: "Arquivo de saída (.mp4)",
    placeholder: "/caminho/saida.mp4",
    picker: Picker::NewFile("saida.mp4"),
    options: &[],
};

fn missing(label: &str) -> CommandError {
    CommandError::new("missing_argument", format!("Preencha o campo «{label}»"))
}

/// The operation a dialog's values stand for. Empty required fields are refused here, with the
/// field's label, before anything reaches the core.
pub fn build_request(kind: FlowKind, values: &[(&str, String)]) -> Result<Value, CommandError> {
    let value = |name: &str| {
        values
            .iter()
            .find(|(field, _)| *field == name)
            .map(|(_, text)| text.trim().to_string())
            .unwrap_or_default()
    };
    let required = |name: &str| {
        let text = value(name);
        let label = kind
            .fields()
            .iter()
            .find(|field| field.name == name)
            .map_or(name, |field| field.label);
        if text.is_empty() {
            Err(missing(label))
        } else {
            Ok(text)
        }
    };
    Ok(match kind {
        FlowKind::Open => json!({ "op": "project.open", "path": required("path")? }),
        FlowKind::SaveAs => json!({ "op": "project.save", "path": required("path")? }),
        FlowKind::Recover => {
            json!({ "op": "project.recover", "path": required("path")?, "apply": true })
        }
        FlowKind::Import => {
            let paths: Vec<String> = required("paths")?
                .split(';')
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(str::to_string)
                .collect();
            // A lone separator passes `required` but names no file.
            if paths.is_empty() {
                required("paths").and_then(|_| Err(missing("Arquivos")))?;
            }
            let mode = value("mode");
            json!({ "op": "media.import", "paths": paths, "mode": if mode.is_empty() { "copy".to_string() } else { mode } })
        }
        FlowKind::ImportProject => json!({ "op": "project.import", "source": required("source")? }),
        FlowKind::Package => {
            let format = value("format");
            json!({ "op": "project.package", "path": required("path")?, "format": if format.is_empty() { "archive".to_string() } else { format } })
        }
        FlowKind::Export => {
            json!({ "op": "export.start", "preset": required("preset")?, "destination": required("destination")? })
        }
    })
}

/// The read-only operation that fills a dialog's report (preflight, listing), if it has one.
pub fn report_request(kind: FlowKind, values: &[(&str, String)]) -> Option<Value> {
    let request = match kind {
        FlowKind::Export => build_request(kind, values).ok().map(|mut request| {
            request["op"] = json!("export.preflight");
            request
        }),
        FlowKind::Package => build_request(kind, values).ok().map(|mut request| {
            request["op"] = json!("project.package_preflight");
            request
        }),
        FlowKind::Recover => build_request(kind, values).ok().map(|mut request| {
            request["apply"] = json!(false);
            request
        }),
        _ => None,
    }?;
    Some(request)
}

/// Shown in a dialog's report while its preflight job runs.
pub const VERIFYING: &str = "Verificando…";

/// One-line account of a preflight/listing report, for the dialog and the status bar.
pub fn describe_report(report: &Value) -> String {
    let blockers = report["blockers"].as_array().map_or(0, Vec::len);
    let warnings = report["warnings"].as_array().map_or(0, Vec::len);
    if report.get("blockers").is_some() {
        let first = report["blockers"][0]["message"].as_str().unwrap_or("");
        let summary = &report["summary"];
        let mut text = if blockers == 0 {
            "Pronto para exportar".to_string()
        } else {
            format!("{blockers} impedimento(s): {first}")
        };
        if let (Some(width), Some(height), Some(frames)) = (
            summary["width"].as_i64(),
            summary["height"].as_i64(),
            summary["frames"].as_i64(),
        ) {
            text.push_str(&format!(" · {width}×{height} · {frames} quadros"));
        }
        if warnings > 0 {
            text.push_str(&format!(" · {warnings} aviso(s)"));
        }
        return text;
    }
    if let Some(revisions) = report["revisions"].as_array() {
        return format!("{} revisão(ões) de recuperação", revisions.len());
    }
    report.to_string().chars().take(160).collect()
}

/// What the status bar says when a background job finishes.
pub fn describe_job_event(event: &Value) -> (String, bool) {
    let kind = event["kind"].as_str().unwrap_or("job");
    let label = match kind {
        "video-export" => "Exportação",
        "package" => "Pacote",
        "import" => "Importação",
        "project-import" => "Importação de projeto",
        "restore" => "Restauração",
        "proxy" => "Preparo de mídia",
        "preflight" => "Verificação",
        _ => "Tarefa",
    };
    match event["state"].as_str() {
        Some("succeeded") => (format!("{label} concluída"), false),
        Some("cancelled") => (format!("{label} cancelada"), false),
        _ => {
            let message = event["error"]["message"].as_str().unwrap_or("falhou");
            (format!("{label} falhou: {message}"), true)
        }
    }
}

/// The automation nodes of the flows: always present, `open` is their only action.
pub fn nodes() -> Vec<Value> {
    FlowKind::ALL
        .into_iter()
        .map(|kind| {
            json!({
                "id": format!("flow.{}", kind.id()), "kind": "button", "label": kind.title(),
                "parent": "region:header", "children": [], "enabled": true, "visible": true,
                "fields": [], "actions": [{ "action": "open", "label": kind.title() }],
                "source": "flows.rs",
            })
        })
        .chain([json!({
            "id": "flow.save", "kind": "button", "label": "Salvar projeto",
            "parent": "region:header", "children": [], "enabled": true, "visible": true,
            "fields": [], "actions": [{ "action": "save", "label": "Salvar projeto" }],
            "source": "flows.rs",
        })])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values<'a>(pairs: &[(&'a str, &str)]) -> Vec<(&'a str, String)> {
        pairs
            .iter()
            .map(|(name, value)| (*name, value.to_string()))
            .collect()
    }

    #[test]
    fn every_kind_has_fields_and_a_distinct_id() {
        let mut ids: Vec<_> = FlowKind::ALL.iter().map(|kind| kind.id()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), FlowKind::ALL.len());
        for kind in FlowKind::ALL {
            assert!(!kind.fields().is_empty(), "{kind:?}");
            assert_eq!(FlowKind::parse(kind.id()), Some(kind));
        }
        assert_eq!(FlowKind::parse("nope"), None);
    }

    #[test]
    fn dialogs_send_the_operations_scripts_send() {
        assert_eq!(
            build_request(FlowKind::Open, &values(&[("path", " /p/a.tessel.json ")])).unwrap(),
            json!({ "op": "project.open", "path": "/p/a.tessel.json" })
        );
        assert_eq!(
            build_request(
                FlowKind::Import,
                &values(&[("paths", "/a.mp4; /b.wav;"), ("mode", "reference")])
            )
            .unwrap(),
            json!({ "op": "media.import", "paths": ["/a.mp4", "/b.wav"], "mode": "reference" })
        );
        assert_eq!(
            build_request(
                FlowKind::Export,
                &values(&[("preset", "vertical"), ("destination", "/o.mp4")])
            )
            .unwrap(),
            json!({ "op": "export.start", "preset": "vertical", "destination": "/o.mp4" })
        );
        assert_eq!(
            build_request(FlowKind::Package, &values(&[("path", "/p.tesselpack")])).unwrap()["format"],
            "archive"
        );
    }

    #[test]
    fn an_empty_required_field_is_refused_with_its_label() {
        let error = build_request(
            FlowKind::Export,
            &values(&[("preset", "fullhd"), ("destination", "  ")]),
        )
        .unwrap_err();
        assert_eq!(error.code, "missing_argument");
        assert!(
            error.message.contains("Arquivo de saída"),
            "{}",
            error.message
        );
        assert!(build_request(FlowKind::Import, &values(&[("paths", " ; ")])).is_err());
    }

    #[test]
    fn reports_are_read_only_variants_of_the_same_request() {
        let export = report_request(
            FlowKind::Export,
            &values(&[("preset", "fullhd"), ("destination", "/o.mp4")]),
        )
        .unwrap();
        assert_eq!(export["op"], "export.preflight");
        let recover = report_request(FlowKind::Recover, &values(&[("path", "/p")])).unwrap();
        assert_eq!(recover["apply"], false);
        assert!(report_request(FlowKind::Open, &values(&[("path", "/p")])).is_none());
        assert!(report_request(FlowKind::Export, &values(&[])).is_none());
    }

    #[test]
    fn job_events_read_in_the_status_bar() {
        assert_eq!(
            describe_job_event(&json!({ "kind": "video-export", "state": "succeeded" })),
            ("Exportação concluída".to_string(), false)
        );
        let (text, error) = describe_job_event(
            &json!({ "kind": "package", "state": "failed", "error": { "message": "disco cheio" } }),
        );
        assert!(error && text.contains("disco cheio"));
        assert!(!describe_job_event(&json!({ "kind": "import", "state": "cancelled" })).1);
    }

    #[test]
    fn preflight_summaries_distinguish_blockers_from_warnings() {
        let ready = json!({ "blockers": [], "warnings": [{}], "summary": { "width": 1920, "height": 1080, "frames": 48 } });
        assert!(describe_report(&ready).starts_with("Pronto"));
        assert!(describe_report(&ready).contains("1920×1080"));
        let blocked = json!({ "blockers": [{ "message": "sem ffmpeg" }], "warnings": [] });
        assert!(describe_report(&blocked).contains("sem ffmpeg"));
    }
}

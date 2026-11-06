//! What each file/import/package/export dialog asks for and the operation it sends.
//!
//! Nothing here knows GPUI: a dialog is a [`FlowKind`], a list of fields and a function that turns
//! the typed values into the same JSON operation a script would send (`project.open`,
//! `media.import`, `project.package`, `export.start`, …). That is what keeps the dialog, the
//! automation tree and the CLI on one set of rules; the dialog only collects strings.

use serde_json::{Value, json};
use tessel_editor_core::CommandError;
use tessel_editor_core::preset::BUILTIN_IDS;

use super::scene_import::{self, ImportPlan};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FlowKind {
    Open,
    SaveAs,
    Recover,
    Import,
    ImportProject,
    Package,
    Export,
    ExportScene,
    /// F043: `scene.attach.start` of a render-bundle (a job), then adopt (with a resource folder),
    /// load, select and show it. See `scene_import.rs`.
    AttachScene,
    /// F043: load or re-adopt the resources of a scene that is already attached.
    SceneResources,
}

impl FlowKind {
    pub const ALL: [FlowKind; 10] = [
        Self::Open,
        Self::SaveAs,
        Self::Recover,
        Self::Import,
        Self::ImportProject,
        Self::Package,
        Self::Export,
        Self::ExportScene,
        Self::AttachScene,
        Self::SceneResources,
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
            Self::ExportScene => "export_scene",
            Self::AttachScene => "attach_scene",
            Self::SceneResources => "scene_resources",
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
            Self::ExportScene => "Exportar cena",
            Self::AttachScene => "Anexar cena",
            Self::SceneResources => "Recursos da cena",
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
            Self::ExportScene => "Exportar cena",
            Self::AttachScene => "Anexar",
            Self::SceneResources => "Carregar",
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
            Self::ExportScene => &[
                SCENE,
                PRESET,
                FROM_FRAME,
                FRAME_COUNT,
                SCENE_AUDIO,
                DESTINATION,
            ],
            Self::AttachScene => &[BUNDLE, SCENE_ID, SCENE_NAME, RESOURCE_FOLDER, SHOW],
            Self::SceneResources => &[SCENE, RESOURCE_FOLDER, SHOW],
        }
    }

    /// The scene flows hand over to `scene_import.rs` instead of sending one operation: loading
    /// resources is a chain of background jobs the window follows on its tick.
    pub fn is_scene_import(self) -> bool {
        matches!(self, Self::AttachScene | Self::SceneResources)
    }

    /// Whether the dialog shows a report (preflight or recovery listing) it can refresh.
    pub fn has_report(self) -> bool {
        matches!(
            self,
            Self::Export | Self::ExportScene | Self::Package | Self::Recover
        )
    }
}

/// How the native picker next to a field behaves.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Picker {
    None,
    Files,
    File,
    NewFile(&'static str),
    /// A folder (F043: where a scene's resources were authored).
    Directory,
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

const BUNDLE: FieldSpec = FieldSpec {
    name: "bundle",
    label: "Render-bundle (.json)",
    placeholder: "/caminho/render-bundle.json",
    picker: Picker::File,
    options: &[],
};
const SCENE_ID: FieldSpec = FieldSpec {
    name: "id",
    label: "ID da cena (opcional)",
    placeholder: "street",
    picker: Picker::None,
    options: &[],
};
const SCENE_NAME: FieldSpec = FieldSpec {
    name: "name",
    label: "Nome (opcional)",
    placeholder: "Rua à noite",
    picker: Picker::None,
    options: &[],
};
const SCENE: FieldSpec = FieldSpec {
    name: "scene",
    label: "Cena",
    placeholder: "street",
    picker: Picker::None,
    options: &[],
};
const FROM_FRAME: FieldSpec = FieldSpec {
    name: "from",
    label: "Primeiro frame (opcional)",
    placeholder: "0",
    picker: Picker::None,
    options: &[],
};
const FRAME_COUNT: FieldSpec = FieldSpec {
    name: "frames",
    label: "Quantidade de frames (opcional)",
    placeholder: "Até o fim",
    picker: Picker::None,
    options: &[],
};
const SCENE_AUDIO: FieldSpec = FieldSpec {
    name: "audio",
    label: "Incluir áudio (ainda indisponível para cenas)",
    placeholder: "yes",
    picker: Picker::None,
    options: &["yes", "no"],
};
const RESOURCE_FOLDER: FieldSpec = FieldSpec {
    name: "source",
    label: "Pasta dos recursos (opcional)",
    placeholder: "/caminho/da/pasta",
    picker: Picker::Directory,
    options: &[],
};
const SHOW: FieldSpec = FieldSpec {
    name: "show",
    label: "Exibir no monitor",
    placeholder: "yes",
    picker: Picker::None,
    options: &["yes", "no"],
};

pub(super) fn field_value(values: &[(&str, String)], name: &str) -> String {
    values
        .iter()
        .find(|(field, _)| *field == name)
        .map(|(_, text)| text.trim().to_string())
        .unwrap_or_default()
}

/// What follows a scene dialog's own operation: whether resources are loaded, from which folder,
/// and whether the scene is then shown. `scene` is the ID the core gave (or the user typed).
/// The folder is only a path: whether it exists, is absolute or holds the files is decided by
/// the media host's `scene.resources.*`, never here.
pub fn import_plan(kind: FlowKind, values: &[(&str, String)], scene: &str) -> ImportPlan {
    let source = Some(field_value(values, "source")).filter(|folder| !folder.is_empty());
    ImportPlan {
        scene: scene.to_string(),
        // Attaching alone is a valid outcome; loading from the project's own folder is what the
        // second dialog is for.
        load: kind == FlowKind::SceneResources || source.is_some(),
        source,
        show: field_value(values, "show") != "no",
    }
}

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
        FlowKind::ExportScene => {
            let mut request = json!({"op": "scene.export", "scene": required("scene")?,
                "preset": required("preset")?, "destination": required("destination")?});
            for name in ["from", "frames"] {
                let text = value(name);
                if !text.is_empty() {
                    let number = text.parse::<i64>().map_err(|_| {
                        CommandError::new(
                            "invalid_value",
                            format!("«{name}» deve ser um número inteiro"),
                        )
                    })?;
                    request[name] = json!(number);
                }
            }
            match value("audio").as_str() {
                "" | "yes" => {}
                "no" => request["audio"] = json!(false),
                _ => {
                    return Err(CommandError::new(
                        "invalid_value",
                        "Áudio deve ser yes ou no",
                    ));
                }
            }
            request
        }
        FlowKind::AttachScene => {
            // Read and validated by a job (BL100); the core's `scene.attach` runs when it is applied.
            let mut request = json!({ "op": "scene.attach.start", "path": required("bundle")? });
            for name in ["id", "name"] {
                let text = value(name);
                if !text.is_empty() {
                    request[name] = json!(text);
                }
            }
            request
        }
        // The first job of the chain; the rest follows from its answer (`scene_import.rs`).
        FlowKind::SceneResources => {
            let scene = required("scene")?;
            scene_import::first_request(&import_plan(kind, values, &scene))
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
        FlowKind::ExportScene => build_request(kind, values).ok().map(|mut request| {
            request["op"] = json!("scene.export.preflight");
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
    fn directed_export_dialog_uses_the_shared_operation_and_typed_range() {
        let fields = values(&[
            ("scene", "demo"),
            ("preset", "1080p"),
            ("destination", "/tmp/out.mp4"),
            ("from", "2"),
            ("frames", "4"),
            ("audio", "no"),
        ]);
        let request = build_request(FlowKind::ExportScene, &fields).unwrap();
        assert_eq!(
            request,
            json!({"op":"scene.export", "scene":"demo", "preset":"1080p", "destination":"/tmp/out.mp4", "from":2, "frames":4, "audio":false})
        );
        assert_eq!(
            report_request(FlowKind::ExportScene, &fields).unwrap()["op"],
            "scene.export.preflight"
        );
        assert_eq!(FlowKind::parse("export_scene"), Some(FlowKind::ExportScene));
    }

    #[test]
    fn directed_export_dialog_refuses_bad_numbers_and_does_not_silently_omit_audio() {
        let mut fields = values(&[
            ("scene", "demo"),
            ("preset", "1080p"),
            ("destination", "/tmp/out.mp4"),
            ("from", "abc"),
        ]);
        assert_eq!(
            build_request(FlowKind::ExportScene, &fields)
                .unwrap_err()
                .code,
            "invalid_value"
        );
        fields.pop();
        assert!(
            build_request(FlowKind::ExportScene, &fields)
                .unwrap()
                .get("audio")
                .is_none()
        );
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

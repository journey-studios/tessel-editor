//! Transition problems in the three languages (F064, P4b S3a, plan §9.4): one pure function of the
//! core that projects what the importer or an edit op found into
//! `{ code, message, hint, transitionId?, shotId?, details }`, for the library, a dry run and the
//! future inspector alike. No adapter builds a sentence.
//!
//! The text lives in one table, [`TEXTS`], keyed by the issue code and, when the sentence changes
//! shape inside one code, by `code:kind` (`kind` is a field of the problem's `details.args`). A
//! template holds `{name}` markers that [`fill`] replaces from `args`; a marker with no value makes
//! the whole problem *untranslated*, never a half-filled sentence. The English templates are the
//! importer's own messages, word for word (a test compares them with real issues), so the English
//! an executor reports and what this function returns in `Locale::En` are the same.
//!
//! There is no silent default arm: a code outside the table comes back with the importer's original
//! text and `details.untranslated: true`, which a test proves is never reached by a transition rule.
//! The structural codes (a wrong type, a missing field, a free-form value) have a localized
//! sentence that points at `path`; the importer's original reason travels in `details.reason`.

use serde_json::{Map, Value, json};

use super::{Labels, Locale};
use crate::directed_scene::Issue;

/// What a rule or an op found, before it is put into words.
#[derive(Debug, Clone, Copy)]
pub struct RawProblem<'a> {
    /// The rule's code (`TRANSITION_OUT_OF_RANGE`, `UNKNOWN_TRANSITION`, `transition_window`, ...).
    pub code: &'a str,
    /// Where it was found: a JSON Pointer for an importer issue, an op field for an op error.
    pub path: Option<&'a str>,
    /// The original English message (kept for a code the table does not know).
    pub message: &'a str,
    /// `transitionId`, `hint` (original English), `args` and any extra facts.
    pub details: Option<&'a Value>,
}

impl<'a> RawProblem<'a> {
    pub fn from_issue(issue: &'a Issue) -> Self {
        Self {
            code: issue.code,
            path: Some(issue.path.as_str()),
            message: issue.message.as_str(),
            details: issue.details.as_ref(),
        }
    }
}

/// One problem, in one language.
#[derive(Debug, Clone, PartialEq)]
pub struct TransitionProblem {
    pub code: String,
    pub message: String,
    pub hint: Option<String>,
    pub transition_id: Option<String>,
    pub shot_id: Option<String>,
    /// `path`, `args`, `lacks`, `reason`, `untranslated`: what an adapter may act on.
    pub details: Value,
}

impl TransitionProblem {
    pub fn to_json(&self) -> Value {
        let mut value = json!({ "code": self.code, "message": self.message });
        if let Some(hint) = &self.hint {
            value["hint"] = json!(hint);
        }
        if let Some(id) = &self.transition_id {
            value["transitionId"] = json!(id);
        }
        if let Some(id) = &self.shot_id {
            value["shotId"] = json!(id);
        }
        value["details"] = self.details.clone();
        value
    }
}

struct Entry {
    key: &'static str,
    message: Labels,
    hint: Option<Labels>,
}

const fn entry(key: &'static str, message: Labels, hint: Option<Labels>) -> Entry {
    Entry { key, message, hint }
}

const fn l(pt_br: &'static str, en: &'static str, es: &'static str) -> Labels {
    Labels::new(pt_br, en, es)
}

const fn some(pt_br: &'static str, en: &'static str, es: &'static str) -> Option<Labels> {
    Some(Labels::new(pt_br, en, es))
}

const WINDOW_HINT: Labels = l(
    "Encurte-a ou mude o alinhamento para a janela ficar dentro da edição.",
    "Shorten it or change its alignment so the window stays inside the edit.",
    "Acórtela o cambie el alineamiento para que la ventana quede dentro de la edición.",
);

const PARAM_HINT: Labels = l(
    "{param}: {accepts}.",
    "{param}: {accepts}.",
    "{param}: {accepts}.",
);

/// Every sentence of a transition problem. The English column is the importer's text.
static TEXTS: &[Entry] = &[
    entry(
        "FX_TRANSITION_NOT_YET",
        l(
            "transições entre planos ainda não foram implementadas; deixe `transitions` vazio",
            "transitions between shots are not implemented yet; leave `transitions` empty",
            "las transiciones entre planos aún no están implementadas; deje `transitions` vacío",
        ),
        None,
    ),
    entry(
        "FX_TRANSITION_KEYS_NOT_YET",
        l(
            "keyframes em uma transição ainda não foram implementados; deixe `keys` de fora",
            "keys on a transition are not implemented yet; leave `keys` out",
            "las claves en una transición aún no están implementadas; deje `keys` fuera",
        ),
        None,
    ),
    entry(
        "WRONG_FAMILY",
        l(
            "transição {transitionId}: {effect} é um efeito de {family}, não uma transição",
            "transition {transitionId}: {effect} is a {family} effect, not a transition",
            "transición {transitionId}: {effect} es un efecto de {family}, no una transición",
        ),
        some(
            "Uma transição usa um efeito da família de transição; um look vai numa trilha de fx como clipe.",
            "A transition takes an effect of the transition family; a look goes on a fx track as a clip.",
            "Una transición usa un efecto de la familia de transición; un look va en una pista de fx como clip.",
        ),
    ),
    entry(
        "UNKNOWN_EFFECT",
        l(
            "transição {transitionId}: não existe o efeito {effect}",
            "transition {transitionId}: there is no effect {effect}",
            "transición {transitionId}: no existe el efecto {effect}",
        ),
        None,
    ),
    entry(
        "UNKNOWN_PARAM",
        l(
            "Transição {transitionId}: {effect} não tem o parâmetro {param}",
            "Transition {transitionId}: {effect} has no parameter {param}",
            "Transición {transitionId}: {effect} no tiene el parámetro {param}",
        ),
        some(
            "Parâmetros de {effect}: {known}.",
            "Parameters of {effect}: {known}.",
            "Parámetros de {effect}: {known}.",
        ),
    ),
    entry(
        "PARAM_TYPE",
        l(
            "Transição {transitionId}: o parâmetro {param} tem o tipo errado",
            "Transition {transitionId}: Parameter {param} has the wrong type",
            "Transición {transitionId}: el parámetro {param} tiene el tipo equivocado",
        ),
        Some(PARAM_HINT),
    ),
    entry(
        "PARAM_OUT_OF_RANGE",
        l(
            "Transição {transitionId}: o parâmetro {param} vale {shown}, fora do que aceita",
            "Transition {transitionId}: Parameter {param} is {shown}, outside what it accepts",
            "Transición {transitionId}: el parámetro {param} vale {shown}, fuera de lo que acepta",
        ),
        Some(PARAM_HINT),
    ),
    entry(
        "UNKNOWN_KEYFRAME_EASING",
        l(
            "A suavização {easing} da transição {transitionId} não é suportada",
            "Transition {transitionId} easing {easing} is not supported",
            "La curva {easing} de la transición {transitionId} no es compatible",
        ),
        some("Use {known}.", "Use {known}.", "Use {known}."),
    ),
    entry(
        "INVALID_VALUE:enum",
        l(
            "`{value}` não é um de: {options}",
            "`{value}` is not one of: {options}",
            "`{value}` no es uno de: {options}",
        ),
        None,
    ),
    entry(
        "UNKNOWN_SHOT",
        l(
            "A transição {transitionId} cita o plano {shot}, que não existe",
            "Transition {transitionId} names shot {shot}, which does not exist",
            "La transición {transitionId} nombra el plano {shot}, que no existe",
        ),
        some("Planos: {shots}.", "Shots: {shots}.", "Planos: {shots}."),
    ),
    entry(
        "TRANSITION_NOT_AT_CUT:last",
        l(
            "Transição {transitionId}: {to} não vem depois de {from} na edição",
            "Transition {transitionId}: {to} does not follow {from} in the edit",
            "Transición {transitionId}: {to} no sigue a {from} en la edición",
        ),
        some(
            "Uma transição fica num corte, entre um plano e o seguinte (este é o último plano).",
            "A transition sits on a cut, between a shot and the one after it (this is the last shot).",
            "Una transición va en un corte, entre un plano y el siguiente (este es el último plano).",
        ),
    ),
    entry(
        "TRANSITION_NOT_AT_CUT:next",
        l(
            "Transição {transitionId}: {to} não vem depois de {from} na edição",
            "Transition {transitionId}: {to} does not follow {from} in the edit",
            "Transición {transitionId}: {to} no sigue a {from} en la edición",
        ),
        some(
            "Uma transição fica num corte, entre um plano e o seguinte (depois dele vem {next}).",
            "A transition sits on a cut, between a shot and the one after it (after it comes {next}).",
            "Una transición va en un corte, entre un plano y el siguiente (después viene {next}).",
        ),
    ),
    entry(
        "TRANSITION_DUPLICATE",
        l(
            "As transições {first} e {transitionId} estão no mesmo corte ({from} para {to})",
            "Transitions {first} and {transitionId} are on the same cut ({from} to {to})",
            "Las transiciones {first} y {transitionId} están en el mismo corte ({from} a {to})",
        ),
        some(
            "Uma transição por corte: set_transition substitui a que já existe.",
            "One transition per cut: set_transition replaces the one that is there.",
            "Una transición por corte: set_transition reemplaza la que ya existe.",
        ),
    ),
    entry(
        "TRANSITION_OUT_OF_RANGE:duration_below_one",
        l(
            "A transição {transitionId} dura {duration} quadros",
            "Transition {transitionId} lasts {duration} frames",
            "La transición {transitionId} dura {duration} fotogramas",
        ),
        some(
            "Uma transição dura pelo menos 1 quadro.",
            "A transition lasts at least 1 frame.",
            "Una transición dura al menos 1 fotograma.",
        ),
    ),
    entry(
        "TRANSITION_OUT_OF_RANGE:duration_above_edit",
        l(
            "A transição {transitionId} dura {duration} quadros, mais que a edição inteira ({edit})",
            "Transition {transitionId} lasts {duration} frames, more than the whole edit ({edit})",
            "La transición {transitionId} dura {duration} fotogramas, más que toda la edición ({edit})",
        ),
        some(
            "Uma transição não pode ser mais longa que a edição: encurte-a.",
            "A transition cannot be longer than the edit: shorten it.",
            "Una transición no puede ser más larga que la edición: acórtela.",
        ),
    ),
    entry(
        "TRANSITION_OUT_OF_RANGE:window_outside_edit",
        l(
            "A transição {transitionId} (quadros {start} a {end}) sai da edição (0 a {edit})",
            "Transition {transitionId} (frames {start} to {end}) leaves the edit (0 to {edit})",
            "La transición {transitionId} (fotogramas {start} a {end}) sale de la edición (0 a {edit})",
        ),
        Some(WINDOW_HINT),
    ),
    entry(
        "TRANSITION_OUT_OF_RANGE:overflow",
        l(
            "Transição {transitionId}: uma janela de {duration} quadros no corte {cut} não cabe na aritmética de quadros",
            "Transition {transitionId}: a window of {duration} frames at the cut {cut} does not fit the frame arithmetic",
            "Transición {transitionId}: una ventana de {duration} fotogramas en el corte {cut} no cabe en la aritmética de fotogramas",
        ),
        Some(WINDOW_HINT),
    ),
    entry(
        "TRANSITION_TOO_LONG",
        l(
            "A transição {transitionId} precisa de {before} quadros dentro de {from} ({fromLength}) e {after} dentro de {to} ({toLength})",
            "Transition {transitionId} needs {before} frames inside {from} ({fromLength}) and {after} inside {to} ({toLength})",
            "La transición {transitionId} necesita {before} fotogramas dentro de {from} ({fromLength}) y {after} dentro de {to} ({toLength})",
        ),
        some(
            "Com o alinhamento {alignment}, o máximo que estes dois planos comportam é {most} quadros.",
            "With alignment {alignment} the longest these two shots hold is {most} frames.",
            "Con el alineamiento {alignment}, lo máximo que caben estos dos planos es {most} fotogramas.",
        ),
    ),
    entry(
        "TRANSITION_TOO_LONG:no_room",
        l(
            "Nenhuma transição cabe entre {from} ({fromLength} quadros) e {to} ({toLength} quadros) com o alinhamento {alignment}",
            "No transition fits between {from} ({fromLength} frames) and {to} ({toLength} frames) with alignment {alignment}",
            "Ninguna transición cabe entre {from} ({fromLength} fotogramas) y {to} ({toLength} fotogramas) con el alineamiento {alignment}",
        ),
        some(
            "Alongue um dos planos ou mude o alinhamento.",
            "Lengthen one of the shots or change the alignment.",
            "Alargue uno de los planos o cambie el alineamiento.",
        ),
    ),
    entry(
        "TRANSITION_NO_HANDLES",
        l(
            "Transição {transitionId} (handles: {handles}): {lacks}",
            "Transition {transitionId} (handles: {handles}): {lacks}",
            "Transición {transitionId} (handles: {handles}): {lacks}",
        ),
        some(
            "Use handles \"hold\" para congelar o primeiro/último quadro, encurte a transição ou {fixes}.{note}",
            "Set handles to \"hold\" to freeze the first/last frame, shorten the transition, or {fixes}.{note}",
            "Use handles \"hold\" para congelar el primer/último fotograma, acorte la transición o {fixes}.{note}",
        ),
    ),
    entry(
        "TRANSITION_OVERLAP",
        l(
            "As transições {beforeId} (quadros {beforeStart} a {beforeEnd}) e {afterId} (a partir do quadro {afterStart}) se sobrepõem",
            "Transitions {beforeId} (frames {beforeStart} to {beforeEnd}) and {afterId} (from frame {afterStart}) overlap",
            "Las transiciones {beforeId} (fotogramas {beforeStart} a {beforeEnd}) y {afterId} (desde el fotograma {afterStart}) se solapan",
        ),
        some(
            "Encurte uma delas ou mude o alinhamento: as janelas de cortes vizinhos não podem dividir um quadro.",
            "Shorten one of them or change its alignment: the windows of neighbouring cuts cannot share a frame.",
            "Acorte una o cambie el alineamiento: las ventanas de cortes vecinos no pueden compartir un fotograma.",
        ),
    ),
    entry(
        "UNKNOWN_TRANSITION:some",
        l(
            "não existe a transição `{transitionId}`",
            "no transition `{transitionId}`",
            "no existe la transición `{transitionId}`",
        ),
        some(
            "Transições: {known}.",
            "Transitions: {known}.",
            "Transiciones: {known}.",
        ),
    ),
    entry(
        "UNKNOWN_TRANSITION:none",
        l(
            "não existe a transição `{transitionId}`",
            "no transition `{transitionId}`",
            "no existe la transición `{transitionId}`",
        ),
        some(
            "A cena não tem transições.",
            "The scene has no transitions.",
            "La escena no tiene transiciones.",
        ),
    ),
    entry(
        "TRANSITION_ID_MISMATCH",
        l(
            "O corte de {from} para {to} já tem a transição {existing}; o `id` {transitionId} a renomearia",
            "The cut {from} to {to} already has transition {existing}; `id` {transitionId} would rename it",
            "El corte de {from} a {to} ya tiene la transición {existing}; el `id` {transitionId} la renombraría",
        ),
        some(
            "Deixe o `id` de fora ou use {existing}: set_transition mantém o id da transição do corte.",
            "Leave `id` out or use {existing}: set_transition keeps the id of the transition on a cut.",
            "Deje el `id` fuera o use {existing}: set_transition mantiene el id de la transición del corte.",
        ),
    ),
    entry(
        "transition_window",
        l(
            "O quadro {frame} está dentro da janela da transição {transitionId} (quadros {start} a {end})",
            "Frame {frame} is inside the window of transition {transitionId} (frames {start} to {end})",
            "El fotograma {frame} está dentro de la ventana de la transición {transitionId} (fotogramas {start} a {end})",
        ),
        some(
            "Mova a agulha para fora dessa janela para editar este overlay.",
            "Move the playhead outside that window to edit this overlay.",
            "Mueva el cabezal fuera de esa ventana para editar este overlay.",
        ),
    ),
    // Structural findings: a sentence that points at the place; the original reason is kept in
    // `details.reason`.
    entry(
        "INVALID_TYPE",
        l(
            "Tipo inválido em `{path}`",
            "Wrong type at `{path}`",
            "Tipo no válido en `{path}`",
        ),
        None,
    ),
    entry(
        "MISSING_FIELD",
        l(
            "Campo obrigatório ausente em `{path}`",
            "Missing field at `{path}`",
            "Falta un campo obligatorio en `{path}`",
        ),
        None,
    ),
    entry(
        "INVALID_VALUE",
        l(
            "Valor inválido em `{path}`",
            "Invalid value at `{path}`",
            "Valor no válido en `{path}`",
        ),
        None,
    ),
    entry(
        "DUPLICATE_ID",
        l(
            "Id duplicado em `{path}`",
            "Duplicate id at `{path}`",
            "Id duplicado en `{path}`",
        ),
        None,
    ),
    entry(
        "RESERVED_ID",
        l(
            "Id reservado em `{path}`",
            "Reserved id at `{path}`",
            "Id reservado en `{path}`",
        ),
        None,
    ),
    entry(
        "FX_LIMIT",
        l(
            "Limite do fx excedido em `{path}`",
            "fx limit exceeded at `{path}`",
            "Límite de fx excedido en `{path}`",
        ),
        None,
    ),
];

/// The smaller sentences the longer ones are built from (a lack of handles lists its pieces).
static PHRASES: &[(&str, Labels)] = &[
    (
        "side.before",
        l("antes do início", "before its start", "antes de su inicio"),
    ),
    (
        "side.after",
        l("depois do fim", "after its end", "después de su final"),
    ),
    (
        "what.both",
        l(
            "instância de cena {instance}, clipe de câmera {camera}",
            "scene instance {instance}, camera clip {camera}",
            "instancia de escena {instance}, clip de cámara {camera}",
        ),
    ),
    (
        "what.instance",
        l("instância de cena", "scene instance", "instancia de escena"),
    ),
    (
        "what.camera",
        l("clipe de câmera", "camera clip", "clip de cámara"),
    ),
    (
        "lack.start",
        l(
            "{shot} não tem {most} quadros {side} ({what})",
            "{shot} lacks {most} frames {side} ({what})",
            "{shot} no tiene {most} fotogramas {side} ({what})",
        ),
    ),
    (
        "lack.end",
        l(
            "o cameraInFrame {cameraIn} de {shot} passa {cameraEnd} quadros do fim do seu clipe de câmera {cameraClip} ({cameraFrames} quadros)",
            "{shot}'s cameraInFrame {cameraIn} is {cameraEnd} frames past the end of its camera clip {cameraClip} ({cameraFrames} frames)",
            "el cameraInFrame {cameraIn} de {shot} pasa {cameraEnd} fotogramas del final de su clip de cámara {cameraClip} ({cameraFrames} fotogramas)",
        ),
    ),
    ("and", l(" e ", " and ", " y ")),
    (
        "fix.instance",
        l(
            "dê a {shot} mais {n} quadros da sua instância de cena {sceneInstance} {side}",
            "give {shot} {n} more frames of its scene instance {sceneInstance} {side}",
            "dé a {shot} {n} fotogramas más de su instancia de escena {sceneInstance} {side}",
        ),
    ),
    (
        "fix.camera",
        l(
            "estenda o clipe de câmera {cameraClip} em {n} quadros {side}",
            "extend its camera clip {cameraClip} by {n} frames {side}",
            "extienda el clip de cámara {cameraClip} en {n} fotogramas {side}",
        ),
    ),
    (
        "fix.cameraEnd",
        l(
            "estenda o clipe de câmera {cameraClip} em {cameraEnd} quadros no fim, para que o cameraInFrame {cameraIn} fique dentro dele",
            "extend its camera clip {cameraClip} by {cameraEnd} frames at its end, so that cameraInFrame {cameraIn} lies inside it",
            "extienda el clip de cámara {cameraClip} en {cameraEnd} fotogramas al final, para que el cameraInFrame {cameraIn} quede dentro",
        ),
    ),
    (
        "refuse_note",
        l(
            " \"refuse\" pede os mesmos quadros reais que \"use\" (ainda não diferem em nenhum cálculo, scene-fx-decisions.md §18 #6); só \"hold\" aceita quadros congelados.",
            " \"refuse\" asks for the same real frames as \"use\" (they differ in no calculation yet, scene-fx-decisions.md §18 #6); only \"hold\" accepts frozen ones.",
            " \"refuse\" pide los mismos fotogramas reales que \"use\" (aún no difieren en ningún cálculo, scene-fx-decisions.md §18 #6); solo \"hold\" acepta fotogramas congelados.",
        ),
    ),
    (
        "accepts.number",
        l(
            "número {min}..{max}, padrão {default} {unit}",
            "number {min}..{max}, default {default} {unit}",
            "número {min}..{max}, predeterminado {default} {unit}",
        ),
    ),
    (
        "accepts.choice",
        l(
            "um de {options}, padrão {default}",
            "one of {options}, default {default}",
            "uno de {options}, predeterminado {default}",
        ),
    ),
];

/// Every key of the table and of the phrases, for the tests that enumerate them.
pub fn translated_keys() -> Vec<&'static str> {
    TEXTS
        .iter()
        .map(|entry| entry.key)
        .chain(PHRASES.iter().map(|(key, _)| *key))
        .collect()
}

/// The three-language templates of a key (message, hint). Exposed for the tests that compare the
/// markers across languages.
pub fn templates(key: &str) -> Option<(Labels, Option<Labels>)> {
    TEXTS
        .iter()
        .find(|entry| entry.key == key)
        .map(|entry| (entry.message, entry.hint))
        .or_else(|| {
            PHRASES
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, labels)| (*labels, None))
        })
}

fn phrase(key: &str) -> Option<&'static Labels> {
    PHRASES
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, labels)| labels)
}

/// The text of a value as a sentence shows it. `None` for a value a sentence cannot carry.
fn shown(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        Value::Array(items) => items
            .iter()
            .map(shown)
            .collect::<Option<Vec<_>>>()
            .map(|v| v.join(", ")),
        _ => None,
    }
}

/// Replaces every `{name}` of `template` with its value in `args`. `None` when a marker has no
/// value: the caller reports the problem untranslated rather than print a hole.
fn fill(template: &str, args: &Map<String, Value>) -> Option<String> {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let close = after.find('}')?;
        out.push_str(&shown(args.get(&after[..close])?)?);
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    Some(out)
}

fn number_of(object: &Map<String, Value>, key: &str) -> Option<i64> {
    object.get(key).and_then(Value::as_i64).filter(|n| *n >= 0)
}

/// The pieces of a lack of handles that depend on the language: the list of what is missing, the
/// list of repairs and the note of `refuse`. They are built from structured numbers, never parsed
/// out of the English sentence.
fn derive_no_handles(args: &mut Map<String, Value>, locale: Locale) -> Option<()> {
    let lacks = args.get("lacks")?.as_array()?.clone();
    let and = phrase("and")?.get(locale);
    let side = |entry: &Map<String, Value>| -> Option<Value> {
        let key = match entry.get("side")?.as_str()? {
            "before" => "side.before",
            _ => "side.after",
        };
        Some(json!(phrase(key)?.get(locale)))
    };
    let mut found: Vec<String> = Vec::new();
    let mut fixes: Vec<String> = Vec::new();
    for lack in &lacks {
        let lack = lack.as_object()?;
        let mut piece = lack.clone();
        piece.insert("side".to_string(), side(lack)?);
        let (instance, camera) = (number_of(lack, "instance")?, number_of(lack, "camera")?);
        let camera_end = match lack.get("cameraEnd") {
            None => 0,
            Some(_) => number_of(lack, "cameraEnd")?,
        };
        let camera_extend = match lack.get("cameraExtend") {
            None => camera,
            Some(_) => number_of(lack, "cameraExtend")?,
        };
        if instance > 0 || camera > 0 {
            piece.insert("most".to_string(), json!(instance.max(camera)));
            let what_key = match (instance > 0, camera > 0) {
                (true, true) => "what.both",
                (true, false) => "what.instance",
                _ => "what.camera",
            };
            let what = fill(phrase(what_key)?.get(locale), &piece)?;
            piece.insert("what".to_string(), json!(what));
            found.push(fill(phrase("lack.start")?.get(locale), &piece)?);
        }
        if camera_end > 0 {
            found.push(fill(phrase("lack.end")?.get(locale), &piece)?);
        }
        if instance > 0 {
            piece.insert("n".to_string(), json!(instance));
            fixes.push(fill(phrase("fix.instance")?.get(locale), &piece)?);
        }
        if camera > 0 {
            // The repair is the exact extension (BL082.4.25); a problem without it asks for `camera`.
            piece.insert("n".to_string(), json!(camera_extend));
            fixes.push(fill(phrase("fix.camera")?.get(locale), &piece)?);
        }
        if camera_end > 0 {
            fixes.push(fill(phrase("fix.cameraEnd")?.get(locale), &piece)?);
        }
    }
    let note = if args.get("handles").and_then(Value::as_str) == Some("refuse") {
        phrase("refuse_note")?.get(locale)
    } else {
        ""
    };
    args.insert("lacks".to_string(), json!(found.join(and)));
    args.insert("fixes".to_string(), json!(fixes.join("; ")));
    args.insert("note".to_string(), json!(note));
    Some(())
}

/// What a parameter accepts, in the language asked.
fn derive_accepts(args: &mut Map<String, Value>, locale: Locale) -> Option<()> {
    let key = match args.get("acceptsKind")?.as_str()? {
        "number" => "accepts.number",
        _ => "accepts.choice",
    };
    let text = fill(phrase(key)?.get(locale), args)?;
    args.insert("accepts".to_string(), json!(text));
    Some(())
}

fn key_of(code: &str, args: Option<&Map<String, Value>>) -> String {
    match args
        .and_then(|args| args.get("kind"))
        .and_then(Value::as_str)
    {
        Some(kind) => format!("{code}:{kind}"),
        None => code.to_string(),
    }
}

/// The message and the hint of a problem in `locale`, or `None` when the key is not in the table
/// or an argument the sentence needs is missing.
fn render(
    raw: &RawProblem<'_>,
    transition_id: Option<&str>,
    args: Option<&Map<String, Value>>,
    locale: Locale,
) -> Option<(String, Option<String>)> {
    let (code, path, reason) = (raw.code, raw.path, raw.message);
    let key = key_of(code, args);
    let entry = TEXTS.iter().find(|entry| entry.key == key)?;
    let mut values = args.cloned().unwrap_or_default();
    if let Some(id) = transition_id {
        values.entry("transitionId").or_insert_with(|| json!(id));
    }
    if let Some(path) = path {
        values.entry("path").or_insert_with(|| json!(path));
    }
    values.entry("reason").or_insert_with(|| json!(reason));
    match code {
        "TRANSITION_NO_HANDLES" => derive_no_handles(&mut values, locale)?,
        "PARAM_TYPE" | "PARAM_OUT_OF_RANGE" => derive_accepts(&mut values, locale)?,
        _ => {}
    }
    let message = fill(entry.message.get(locale), &values)?;
    let hint = match &entry.hint {
        Some(hint) => Some(fill(hint.get(locale), &values)?),
        None => None,
    };
    Some((message, hint))
}

/// The shot a problem is about, when it is about exactly one.
fn shot_of(args: Option<&Map<String, Value>>) -> Option<String> {
    let args = args?;
    if let Some(shot) = args.get("shot").and_then(Value::as_str) {
        return Some(shot.to_string());
    }
    let lacks = args.get("lacks")?.as_array()?;
    let mut shots = lacks
        .iter()
        .filter_map(|lack| lack.get("shot").and_then(Value::as_str));
    let first = shots.next()?;
    shots.all(|shot| shot == first).then(|| first.to_string())
}

/// Projects one problem into `locale`. Pure: the same input gives the same output.
pub fn project_problem(raw: &RawProblem<'_>, locale: Locale) -> TransitionProblem {
    let facts = raw.details.and_then(Value::as_object);
    let args = facts
        .and_then(|facts| facts.get("args"))
        .and_then(Value::as_object);
    let transition_id = facts
        .and_then(|facts| facts.get("transitionId"))
        .or_else(|| args.and_then(|args| args.get("transitionId")))
        .and_then(Value::as_str)
        .map(str::to_string);
    let original_hint = facts
        .and_then(|facts| facts.get("hint"))
        .and_then(Value::as_str)
        .map(str::to_string);

    let mut details = Map::new();
    if let Some(path) = raw.path {
        details.insert("path".to_string(), json!(path));
    }
    if let Some(facts) = facts {
        for (key, value) in facts {
            if key != "hint" && key != "transitionId" {
                details.insert(key.clone(), value.clone());
            }
        }
    }
    let generic = matches!(
        raw.code,
        "INVALID_TYPE"
            | "MISSING_FIELD"
            | "INVALID_VALUE"
            | "DUPLICATE_ID"
            | "RESERVED_ID"
            | "FX_LIMIT"
    ) && args.is_none();
    let (message, hint) = match render(raw, transition_id.as_deref(), args, locale) {
        Some(found) => {
            if generic {
                details.insert("reason".to_string(), json!(raw.message));
            }
            found
        }
        None => {
            details.insert("untranslated".to_string(), json!(true));
            (raw.message.to_string(), original_hint)
        }
    };
    TransitionProblem {
        code: raw.code.to_string(),
        message,
        hint,
        transition_id,
        shot_id: shot_of(args),
        details: Value::Object(details),
    }
}

/// [`project_problem`] over the issues of an importer refusal, in order.
pub fn project_issues(issues: &[Issue], locale: Locale) -> Vec<TransitionProblem> {
    issues
        .iter()
        .map(|issue| project_problem(&RawProblem::from_issue(issue), locale))
        .collect()
}

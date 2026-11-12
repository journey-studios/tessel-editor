//! How a diagnostic mode is read from the environment: one reader for every experiment that is a named choice (BL157.1 asset
//! I/O, BL157.6 playback poll), so each of them declares its variable and its names and nothing else.
//!
//! The mode is read once, when what it configures is created. An unset or blank variable is the default; a name nobody knows
//! is a typo, warned about on stderr with the names that exist, and the default is used: the report of the experiment names
//! the mode in effect, so a typo never passes for the other arm.

use std::fmt::Write as _;

/// A mode chosen by a named environment variable.
pub(super) trait EnvMode: Copy + Default + PartialEq + 'static {
    /// The variable that selects it.
    const VAR: &'static str;
    /// Every name it accepts and the mode that name selects. They are also what the status and the trace say.
    const NAMES: &'static [(&'static str, Self)];

    /// The name a mode goes by in the status and the trace.
    fn name(self) -> &'static str {
        Self::NAMES
            .iter()
            .find(|(_, mode)| *mode == self)
            .map(|(name, _)| *name)
            .expect("every mode has a name")
    }
}

/// The mode the content of the variable asks for (`None`: unset): the default when it is unset or blank, the warning for a name
/// nobody knows. Case and surrounding spaces do not matter.
pub(super) fn parse_env<M: EnvMode>(value: Option<&str>) -> Result<M, String> {
    let Some(text) = value.map(str::trim).filter(|text| !text.is_empty()) else {
        return Ok(M::default());
    };
    let wanted = text.to_ascii_lowercase();
    if let Some((_, mode)) = M::NAMES.iter().find(|(name, _)| *name == wanted) {
        return Ok(*mode);
    }
    let mut names = String::new();
    for (index, (name, _)) in M::NAMES.iter().enumerate() {
        let separator = match (index, M::NAMES.len()) {
            (0, _) => "",
            (last, len) if last + 1 == len => " or ",
            _ => ", ",
        };
        let _ = write!(names, "{separator}`{name}`");
    }
    Err(format!(
        "{}={text:?} is not {names}: using `{}`",
        M::VAR,
        M::default().name()
    ))
}

/// The mode the environment asks for, read now. A typo is warned about on stderr and the default is used.
pub(super) fn from_env<M: EnvMode>() -> M {
    match parse_env(std::env::var(M::VAR).ok().as_deref()) {
        Ok(mode) => mode,
        Err(warning) => {
            eprintln!("tessel-editor: {warning}");
            M::default()
        }
    }
}

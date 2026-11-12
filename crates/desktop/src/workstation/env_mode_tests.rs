//! The reader of the named-choice environment variables (BL157.1, BL157.6): the one helper, and what each mode declares to it.

use std::fmt::Debug;

use super::env_mode::{EnvMode, parse_env};
use super::playback_poll::PollMode;
use super::viewport_protocol::AssetIoMode;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
enum Shade {
    #[default]
    Light,
    Medium,
    Dark,
}

impl EnvMode for Shade {
    const VAR: &'static str = "TEST_SHADE";
    const NAMES: &'static [(&'static str, Self)] = &[
        ("light", Self::Light),
        ("medium", Self::Medium),
        ("dark", Self::Dark),
    ];
}

#[test]
fn unset_or_blank_is_the_default() {
    assert_eq!(parse_env::<Shade>(None), Ok(Shade::Light));
    assert_eq!(parse_env::<Shade>(Some("")), Ok(Shade::Light));
    assert_eq!(parse_env::<Shade>(Some("  \n")), Ok(Shade::Light));
}

#[test]
fn a_name_is_found_whatever_the_case_and_the_spacing() {
    assert_eq!(parse_env::<Shade>(Some("dark")), Ok(Shade::Dark));
    assert_eq!(parse_env::<Shade>(Some("  MEDIUM\n")), Ok(Shade::Medium));
    assert_eq!(parse_env::<Shade>(Some("Light")), Ok(Shade::Light));
}

#[test]
fn a_typo_is_a_warning_with_the_variable_the_value_every_name_and_the_mode_in_use() {
    let warning = parse_env::<Shade>(Some("darkk")).unwrap_err();
    assert!(warning.starts_with("TEST_SHADE="), "{warning}");
    assert!(warning.contains("\"darkk\""), "{warning}");
    assert!(
        warning.contains("`light`, `medium` or `dark`"),
        "every name, listed: {warning}"
    );
    assert!(warning.ends_with("using `light`"), "{warning}");
}

/// What a mode declares: the documented variable, the names the interface of the experiment documents, its default, and that
/// each name selects the mode that reports itself under it.
fn declares<M: EnvMode + Debug>(var: &str, names: &[&str], default: M) {
    assert_eq!(M::VAR, var);
    assert_eq!(
        M::NAMES.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
        names
    );
    assert_eq!(M::default(), default);
    for (name, mode) in M::NAMES {
        assert_eq!(parse_env::<M>(Some(name)), Ok(*mode));
        assert_eq!(mode.name(), *name);
    }
}

#[test]
fn the_poll_modes_declare_the_documented_variable_names_and_default() {
    declares(
        "TESSEL_PLAYBACK_POLL",
        &["fixed", "deadline"],
        PollMode::Fixed,
    );
}

#[test]
fn the_asset_io_modes_declare_the_documented_variable_names_and_default() {
    declares(
        "TESSEL_VIEWPORT_ASSET_IO",
        &["sync", "async"],
        AssetIoMode::Sync,
    );
    // The reader names itself in the status and the trace: it must be the name the variable gave it.
    for (name, mode) in AssetIoMode::NAMES {
        assert_eq!(mode.reader().name(), *name);
    }
}

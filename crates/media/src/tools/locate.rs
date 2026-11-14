//! Where FFmpeg, FFprobe and Node live on this machine.
//!
//! One small resolver shared by `Tools::discover` (FFmpeg/FFprobe) and the render crate (Node), so
//! the rules are written once. It is an adapter helper, not domain logic: it only maps a tool
//! name to an executable file. The environment, the platform and the file system come through
//! [`Host`], so the tests drive every rule without touching the process environment.
//!
//! Order, for every tool: the explicit override (`TESSEL_FFMPEG`, `TESSEL_FFPROBE`, `TESSEL_NODE`),
//! then each `PATH` directory in order, then the usual install places of the platform. A
//! candidate counts only if it is a **file** (a directory called `ffmpeg.exe` is not a tool).
//!
//! Windows adds what `Command::new("ffmpeg")` would do through `PATHEXT` (`ffmpeg.exe`, in the
//! order of `PATHEXT`) and looks in the places installers use: `Program Files` and `LOCALAPPDATA`
//! (Node), the WinGet `Links` folders and the `Gyan.FFmpeg*` WinGet packages (FFmpeg). That last
//! one reads two directory levels (`Packages/Gyan.FFmpeg*/ffmpeg-*/bin`) and nothing deeper: no
//! recursive disk scan, no shell, no version in the code, no change to `PATH` or the registry.
//!
//! Everything that is not Windows keeps what it always did: the name as given, then, for FFmpeg
//! only, Homebrew's `/opt/homebrew/bin` and `/usr/local/bin` (a GUI-launched macOS process does
//! not have them on its `PATH`).

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};

/// Entries read from one directory level (a WinGet `Packages` folder has one per installed package).
const MAX_ENTRIES: usize = 4096;

/// What `PATHEXT` is when the variable is missing (the Windows default, minus script hosts).
const DEFAULT_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Windows,
    /// macOS and Linux: the behaviour before Windows had its own rules.
    Other,
}

/// The machine, as far as discovery needs it.
pub trait Host {
    fn os(&self) -> Os;
    fn var(&self, name: &str) -> Option<OsString>;
    /// `true` only for a file (after following links); `false` for a directory or nothing.
    fn is_file(&self, path: &Path) -> bool;
    /// Full paths of the entries directly inside `dir`; empty if it cannot be read.
    fn entries(&self, dir: &Path) -> Vec<PathBuf>;
}

/// The real machine.
pub struct System;

impl Host for System {
    fn os(&self) -> Os {
        if cfg!(windows) {
            Os::Windows
        } else {
            Os::Other
        }
    }

    fn var(&self, name: &str) -> Option<OsString> {
        std::env::var_os(name)
    }

    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn entries(&self, dir: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .flatten()
                    .take(MAX_ENTRIES)
                    .map(|entry| entry.path())
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Ffmpeg,
    Ffprobe,
    Node,
}

impl Tool {
    /// The name looked up (`.exe` and the other `PATHEXT` extensions are added on Windows).
    pub fn binary(self) -> &'static str {
        match self {
            Self::Ffmpeg => "ffmpeg",
            Self::Ffprobe => "ffprobe",
            Self::Node => "node",
        }
    }

    pub fn override_var(self) -> &'static str {
        match self {
            Self::Ffmpeg => "TESSEL_FFMPEG",
            Self::Ffprobe => "TESSEL_FFPROBE",
            Self::Node => "TESSEL_NODE",
        }
    }

    /// An explicit FFmpeg/FFprobe path that is not a file is an error (the user asked for that
    /// exact binary); a Node path that is not a file has always fallen back to the `PATH`.
    fn bad_override_falls_back(self) -> bool {
        self == Self::Node
    }
}

/// Where a resolved executable came from, for diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// The named environment variable.
    Override(&'static str),
    Path,
    /// A usual install place, by name.
    Install(&'static str),
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Override(var) => write!(f, "override {var}"),
            Self::Path => f.write_str("PATH"),
            Self::Install(place) => write!(f, "install location: {place}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub path: PathBuf,
    pub origin: Origin,
}

/// The executable for `tool`, or `None`.
pub fn locate(host: &dyn Host, tool: Tool) -> Option<Found> {
    let var = tool.override_var();
    if let Some(explicit) = host.var(var) {
        let explicit = PathBuf::from(explicit);
        if host.is_file(&explicit) {
            return Some(Found {
                path: explicit,
                origin: Origin::Override(var),
            });
        }
        if !tool.bad_override_falls_back() {
            return None;
        }
    }
    let file_names = file_names(host, tool.binary());
    let first_file = |dir: &Path| {
        file_names
            .iter()
            .map(|name| dir.join(name))
            .find(|candidate| host.is_file(candidate))
    };
    let path = host.var("PATH");
    // Preserve Unix lookup: empty entries name the current directory. With PATH absent,
    // FFmpeg historically searched it too, while Node did not search PATH at all.
    if path.is_some() || tool != Tool::Node {
        let path = path.unwrap_or_default();
        if let Some(found) = std::env::split_paths(&path)
            .filter(|dir| host.os() != Os::Windows || !dir.as_os_str().is_empty())
            .find_map(|dir| first_file(&dir))
        {
            return Some(Found {
                path: found,
                origin: Origin::Path,
            });
        }
    }
    install_dirs(host, tool)
        .into_iter()
        .find_map(|(place, dir)| {
            first_file(&dir).map(|path| Found {
                path,
                origin: Origin::Install(place),
            })
        })
}

/// The file names a directory is tried with: the bare name, or on Windows the name with each
/// `PATHEXT` extension a process can be started from, in `PATHEXT` order (`.exe` is always there).
fn file_names(host: &dyn Host, binary: &str) -> Vec<String> {
    if host.os() != Os::Windows {
        return vec![binary.to_string()];
    }
    let pathext = host
        .var("PATHEXT")
        .map(|value| value.to_string_lossy().into_owned())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_PATHEXT.to_string());
    let mut names: Vec<String> = Vec::new();
    for extension in pathext.split(';').map(str::trim) {
        let launchable = [".com", ".exe", ".bat", ".cmd"]
            .iter()
            .any(|known| extension.eq_ignore_ascii_case(known));
        // Lower case: the file system ignores the case, and the path reads the same everywhere.
        let name = format!("{binary}{}", extension.to_ascii_lowercase());
        if launchable && !names.iter().any(|seen| seen.eq_ignore_ascii_case(&name)) {
            names.push(name);
        }
    }
    let exe = format!("{binary}.exe");
    if !names.iter().any(|seen| seen.eq_ignore_ascii_case(&exe)) {
        names.push(exe);
    }
    names
}

fn install_dirs(host: &dyn Host, tool: Tool) -> Vec<(&'static str, PathBuf)> {
    match host.os() {
        Os::Other => match tool {
            // Homebrew is not on the PATH of GUI-launched processes on macOS.
            Tool::Ffmpeg | Tool::Ffprobe => vec![
                ("/opt/homebrew/bin", PathBuf::from("/opt/homebrew/bin")),
                ("/usr/local/bin", PathBuf::from("/usr/local/bin")),
            ],
            Tool::Node => Vec::new(),
        },
        Os::Windows => windows_install_dirs(host, tool),
    }
}

fn windows_install_dirs(host: &dyn Host, tool: Tool) -> Vec<(&'static str, PathBuf)> {
    let var = |name: &str| {
        host.var(name)
            .map(PathBuf::from)
            .filter(|path| !path.as_os_str().is_empty())
    };
    let local = var("LOCALAPPDATA");
    let program_files = var("ProgramFiles");
    let program_files_x86 = var("ProgramFiles(x86)");
    let mut dirs: Vec<(&'static str, PathBuf)> = Vec::new();
    let winget_links = |dirs: &mut Vec<_>| {
        if let Some(local) = &local {
            dirs.push((
                "WinGet Links",
                join(local, &["Microsoft", "WinGet", "Links"]),
            ));
        }
        if let Some(root) = &program_files {
            dirs.push(("WinGet Links (machine)", join(root, &["WinGet", "Links"])));
        }
    };
    match tool {
        Tool::Node => {
            if let Some(root) = &program_files {
                dirs.push(("Program Files\\nodejs", root.join("nodejs")));
            }
            if let Some(root) = &program_files_x86 {
                dirs.push(("Program Files (x86)\\nodejs", root.join("nodejs")));
            }
            if let Some(local) = &local {
                dirs.push((
                    "LOCALAPPDATA\\Programs\\nodejs",
                    join(local, &["Programs", "nodejs"]),
                ));
            }
            winget_links(&mut dirs);
        }
        Tool::Ffmpeg | Tool::Ffprobe => {
            winget_links(&mut dirs);
            let mut packages = Vec::new();
            if let Some(local) = &local {
                packages.push(join(local, &["Microsoft", "WinGet", "Packages"]));
            }
            if let Some(root) = &program_files {
                packages.push(join(root, &["WinGet", "Packages"]));
            }
            for root in packages {
                dirs.extend(
                    gyan_bin_dirs(host, &root)
                        .into_iter()
                        .map(|dir| ("WinGet package Gyan.FFmpeg", dir)),
                );
            }
        }
    }
    dirs
}

/// `Packages/Gyan.FFmpeg*/ffmpeg-*/bin`, newest build first. Two levels, no deeper.
fn gyan_bin_dirs(host: &dyn Host, packages: &Path) -> Vec<PathBuf> {
    let mut builds: Vec<(Vec<u64>, PathBuf)> = Vec::new();
    for package in host.entries(packages) {
        if !lower_name(&package).starts_with("gyan.ffmpeg") {
            continue;
        }
        for build in host.entries(&package) {
            if let Some(version) = lower_name(&build).strip_prefix("ffmpeg-") {
                builds.push((version_key(version), build.join("bin")));
            }
        }
    }
    builds.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    builds.into_iter().map(|(_, dir)| dir).collect()
}

/// `base` with each part appended as its own component (one separator style on Windows).
fn join(base: &Path, parts: &[&str]) -> PathBuf {
    parts
        .iter()
        .fold(base.to_path_buf(), |path, part| path.join(part))
}

fn lower_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// `9.0.2-full_build` → `[9, 0, 2]`: the leading dotted number, compared numerically.
fn version_key(name: &str) -> Vec<u64> {
    name.chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect::<String>()
        .split('.')
        .filter_map(|part| part.parse().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;

    /// An in-memory machine: variables, files and directories. Nothing here reads the process
    /// environment or the disk, so these tests run in parallel with the others.
    struct Fake {
        os: Os,
        vars: BTreeMap<String, OsString>,
        files: BTreeSet<PathBuf>,
        dirs: BTreeSet<PathBuf>,
    }

    impl Fake {
        fn new(os: Os) -> Self {
            Self {
                os,
                vars: BTreeMap::new(),
                files: BTreeSet::new(),
                dirs: BTreeSet::new(),
            }
        }

        fn var(mut self, name: &str, value: impl Into<OsString>) -> Self {
            self.vars.insert(name.to_ascii_uppercase(), value.into());
            self
        }

        fn path<P: AsRef<Path>>(self, dirs: &[P]) -> Self {
            let joined = std::env::join_paths(dirs.iter().map(AsRef::as_ref)).unwrap();
            self.var("PATH", joined)
        }

        fn file(mut self, path: impl Into<PathBuf>) -> Self {
            let path = path.into();
            self.dirs
                .extend(path.ancestors().skip(1).map(Path::to_path_buf));
            self.files.insert(path);
            self
        }

        fn dir(mut self, path: impl Into<PathBuf>) -> Self {
            let path = path.into();
            self.dirs.extend(path.ancestors().map(Path::to_path_buf));
            self
        }
    }

    impl Host for Fake {
        fn os(&self) -> Os {
            self.os
        }

        // Windows variables are case-insensitive; the fake keeps them upper-cased.
        fn var(&self, name: &str) -> Option<OsString> {
            self.vars.get(&name.to_ascii_uppercase()).cloned()
        }

        fn is_file(&self, path: &Path) -> bool {
            self.files.contains(path)
        }

        fn entries(&self, dir: &Path) -> Vec<PathBuf> {
            self.files
                .iter()
                .chain(&self.dirs)
                .filter(|entry| entry.parent() == Some(dir))
                .cloned()
                .collect()
        }
    }

    fn at(base: &str, parts: &[&str]) -> PathBuf {
        parts
            .iter()
            .fold(PathBuf::from(base), |path, part| path.join(part))
    }

    fn windows() -> Fake {
        Fake::new(Os::Windows)
    }

    fn found(host: &Fake, tool: Tool) -> Option<PathBuf> {
        locate(host, tool).map(|found| found.path)
    }

    const LOCAL: &str = "/c/Users/Ana Maria/AppData/Local";
    const FILES: &str = "/c/Program Files";
    const FILES_X86: &str = "/c/Program Files (x86)";

    // ---- Windows: PATH ----

    #[test]
    fn windows_finds_exe_on_path_in_a_folder_with_spaces() {
        let dir = at(FILES, &["nodejs"]);
        let host = windows().path(&[&dir]).file(dir.join("node.exe"));
        let found = locate(&host, Tool::Node).unwrap();
        assert_eq!(found.path, dir.join("node.exe"));
        assert_eq!(found.origin, Origin::Path);
    }

    #[test]
    fn windows_never_matches_the_bare_name() {
        // `node` without an extension is not what Windows runs; the old code would have
        // returned this file and then failed to start it.
        let dir = at(FILES, &["nodejs"]);
        let host = windows().path(&[&dir]).file(dir.join("node"));
        assert_eq!(found(&host, Tool::Node), None);
    }

    #[test]
    fn windows_path_order_wins_over_a_later_folder() {
        let (first, second) = (at("/c/a", &["bin"]), at("/c/b", &["bin"]));
        let host = windows()
            .path(&[&first, &second])
            .file(first.join("ffmpeg.exe"))
            .file(second.join("ffmpeg.exe"));
        assert_eq!(found(&host, Tool::Ffmpeg), Some(first.join("ffmpeg.exe")));
    }

    #[test]
    fn windows_follows_the_pathext_order_inside_one_folder() {
        let dir = at("/c/tools", &[]);
        let both = |pathext: Option<&str>| {
            let mut host = windows()
                .path(&[&dir])
                .file(dir.join("ffmpeg.cmd"))
                .file(dir.join("ffmpeg.exe"));
            if let Some(pathext) = pathext {
                host = host.var("PATHEXT", pathext);
            }
            found(&host, Tool::Ffmpeg)
        };
        assert_eq!(
            both(None),
            Some(dir.join("ffmpeg.exe")),
            "default: .EXE before .CMD"
        );
        assert_eq!(
            both(Some(".CMD;.EXE")),
            Some(dir.join("ffmpeg.cmd")),
            "PATHEXT lists .CMD first"
        );
        assert_eq!(
            both(Some(".VBS;.JS")),
            Some(dir.join("ffmpeg.exe")),
            "script hosts are not launchable and .exe is always tried"
        );
    }

    #[test]
    fn windows_pathext_is_case_insensitive_and_variable_names_too() {
        let dir = at("/c/tools", &[]);
        let host = windows()
            .var("Path", std::env::join_paths([&dir]).unwrap())
            .var("pathext", ".exe")
            .file(dir.join("ffprobe.exe"));
        assert_eq!(found(&host, Tool::Ffprobe), Some(dir.join("ffprobe.exe")));
    }

    #[test]
    fn a_directory_named_like_the_tool_is_rejected_and_the_search_goes_on() {
        let (first, second) = (at("/c/a", &[]), at("/c/b", &[]));
        let host = windows()
            .path(&[&first, &second])
            .dir(first.join("ffmpeg.exe"))
            .file(second.join("ffmpeg.exe"));
        assert_eq!(found(&host, Tool::Ffmpeg), Some(second.join("ffmpeg.exe")));
        // Only the directory exists: nothing is found, in the PATH or as an override.
        let only_dir = windows().path(&[&first]).dir(first.join("ffmpeg.exe"));
        assert_eq!(found(&only_dir, Tool::Ffmpeg), None);
    }

    #[test]
    fn empty_path_entries_are_not_the_current_directory() {
        let host = windows()
            .var("PATH", std::env::join_paths(["", ""]).unwrap())
            .file(PathBuf::from("ffmpeg.exe"));
        assert_eq!(found(&host, Tool::Ffmpeg), None);
    }

    // ---- overrides ----

    #[test]
    fn a_valid_override_wins_over_the_path() {
        let dir = at("/c/tools", &[]);
        let custom = at("/d/custom build", &["ffmpeg.exe"]);
        let host = windows()
            .path(&[&dir])
            .var("TESSEL_FFMPEG", custom.clone())
            .file(dir.join("ffmpeg.exe"))
            .file(custom.clone());
        let found = locate(&host, Tool::Ffmpeg).unwrap();
        assert_eq!(found.path, custom);
        assert_eq!(found.origin, Origin::Override("TESSEL_FFMPEG"));
    }

    #[test]
    fn an_invalid_ffmpeg_override_fails_instead_of_falling_back() {
        let dir = at("/c/tools", &[]);
        let missing = at("/d/gone", &["ffmpeg.exe"]);
        let host = windows()
            .path(&[&dir])
            .var("TESSEL_FFMPEG", missing)
            .var("TESSEL_FFPROBE", dir.clone())
            .file(dir.join("ffmpeg.exe"))
            .file(dir.join("ffprobe.exe"));
        assert_eq!(found(&host, Tool::Ffmpeg), None, "missing file");
        assert_eq!(
            found(&host, Tool::Ffprobe),
            None,
            "a directory is not a file"
        );
        // The same on a machine that is not Windows.
        let unix = Fake::new(Os::Other)
            .path(&[Path::new("/usr/bin")])
            .var("TESSEL_FFMPEG", "/nope/ffmpeg")
            .file("/usr/bin/ffmpeg");
        assert_eq!(found(&unix, Tool::Ffmpeg), None);
    }

    #[test]
    fn an_empty_ffmpeg_override_is_still_an_invalid_override() {
        let host = Fake::new(Os::Other)
            .path(&[Path::new("/usr/bin")])
            .var("TESSEL_FFMPEG", "")
            .file("/usr/bin/ffmpeg");
        assert_eq!(found(&host, Tool::Ffmpeg), None);
    }

    #[test]
    fn a_valid_node_override_wins_and_an_invalid_one_falls_back() {
        let dir = at(FILES, &["nodejs"]);
        let custom = at("/d/node-22", &["node.exe"]);
        let base = windows().path(&[&dir]).file(dir.join("node.exe"));
        let valid = base.var("TESSEL_NODE", custom.clone()).file(custom.clone());
        assert_eq!(found(&valid, Tool::Node), Some(custom));

        let invalid = windows()
            .path(&[&dir])
            .var("TESSEL_NODE", at("/d/gone", &["node.exe"]))
            .file(dir.join("node.exe"));
        let found_node = locate(&invalid, Tool::Node).unwrap();
        assert_eq!(found_node.path, dir.join("node.exe"));
        assert_eq!(found_node.origin, Origin::Path);

        let directory = windows()
            .path(&[&dir])
            .var("TESSEL_NODE", dir.clone())
            .file(dir.join("node.exe"));
        assert_eq!(found(&directory, Tool::Node), Some(dir.join("node.exe")));
    }

    // ---- Windows: install places ----

    #[test]
    fn windows_node_is_found_in_program_files_without_a_path() {
        let node = at(FILES, &["nodejs", "node.exe"]);
        let host = windows().var("ProgramFiles", FILES).file(node.clone());
        let hit = locate(&host, Tool::Node).unwrap();
        assert_eq!(hit.path, node);
        assert_eq!(hit.origin, Origin::Install("Program Files\\nodejs"));

        let x86 = at(FILES_X86, &["nodejs", "node.exe"]);
        let host = windows()
            .var("ProgramFiles", FILES)
            .var("ProgramFiles(x86)", FILES_X86)
            .file(x86.clone());
        assert_eq!(found(&host, Tool::Node), Some(x86));

        let per_user = at(LOCAL, &["Programs", "nodejs", "node.exe"]);
        let host = windows().var("LOCALAPPDATA", LOCAL).file(per_user.clone());
        assert_eq!(found(&host, Tool::Node), Some(per_user));
    }

    #[test]
    fn windows_path_wins_over_the_install_places() {
        let on_path = at("/c/tools", &["node.exe"]);
        let installed = at(FILES, &["nodejs", "node.exe"]);
        let host = windows()
            .var("ProgramFiles", FILES)
            .path(&[on_path.parent().unwrap()])
            .file(on_path.clone())
            .file(installed);
        assert_eq!(found(&host, Tool::Node), Some(on_path));
    }

    #[test]
    fn windows_ffmpeg_is_found_in_the_winget_links_folder() {
        for tool in [Tool::Ffmpeg, Tool::Ffprobe] {
            let link = at(
                LOCAL,
                &[
                    "Microsoft",
                    "WinGet",
                    "Links",
                    &format!("{}.exe", tool.binary()),
                ],
            );
            let host = windows().var("LOCALAPPDATA", LOCAL).file(link.clone());
            let found = locate(&host, tool).unwrap();
            assert_eq!(found.path, link);
            assert_eq!(found.origin, Origin::Install("WinGet Links"));
        }
    }

    fn gyan(build: &str, binary: &str) -> PathBuf {
        at(
            LOCAL,
            &[
                "Microsoft",
                "WinGet",
                "Packages",
                "Gyan.FFmpeg_Microsoft.Winget.Source_8wekyb3d8bbwe",
                build,
                "bin",
                binary,
            ],
        )
    }

    #[test]
    fn windows_ffmpeg_is_found_in_the_gyan_winget_layout_and_the_newest_build_wins() {
        let newest = gyan("ffmpeg-10.1-full_build", "ffmpeg.exe");
        let host = windows()
            .var("LOCALAPPDATA", LOCAL)
            .file(gyan("ffmpeg-9.0.2-full_build", "ffmpeg.exe"))
            .file(newest.clone())
            .file(gyan("ffmpeg-8.0-essentials_build", "ffmpeg.exe"));
        let found = locate(&host, Tool::Ffmpeg).unwrap();
        assert_eq!(
            found.path, newest,
            "10.1 is newer than 9.0.2 numerically, not as text"
        );
        assert_eq!(found.origin, Origin::Install("WinGet package Gyan.FFmpeg"));
    }

    #[test]
    fn the_gyan_layout_is_limited_to_two_levels_and_to_that_package() {
        let too_deep = at(
            LOCAL,
            &[
                "Microsoft",
                "WinGet",
                "Packages",
                "Gyan.FFmpeg_x",
                "ffmpeg-9.0-full_build",
                "bin",
                "nested",
                "ffmpeg.exe",
            ],
        );
        let other_package = at(
            LOCAL,
            &[
                "Microsoft",
                "WinGet",
                "Packages",
                "Someone.Else_x",
                "ffmpeg-9.0-full_build",
                "bin",
                "ffmpeg.exe",
            ],
        );
        let not_a_build = at(
            LOCAL,
            &[
                "Microsoft",
                "WinGet",
                "Packages",
                "Gyan.FFmpeg_x",
                "tools",
                "bin",
                "ffmpeg.exe",
            ],
        );
        let host = windows()
            .var("LOCALAPPDATA", LOCAL)
            .file(too_deep)
            .file(other_package)
            .file(not_a_build);
        assert_eq!(found(&host, Tool::Ffmpeg), None);
    }

    #[test]
    fn windows_ffmpeg_on_the_path_wins_over_winget() {
        let on_path = at("/c/ffmpeg/bin", &["ffmpeg.exe"]);
        let host = windows()
            .var("LOCALAPPDATA", LOCAL)
            .path(&[on_path.parent().unwrap()])
            .file(on_path.clone())
            .file(gyan("ffmpeg-9.0.2-full_build", "ffmpeg.exe"));
        assert_eq!(found(&host, Tool::Ffmpeg), Some(on_path));
    }

    #[test]
    fn missing_windows_variables_give_no_candidates_and_no_panic() {
        let host = windows();
        for tool in [Tool::Ffmpeg, Tool::Ffprobe, Tool::Node] {
            assert_eq!(found(&host, tool), None);
        }
    }

    // ---- not Windows ----

    #[test]
    fn other_systems_keep_the_bare_name_and_the_homebrew_places() {
        let dir = Path::new("/usr/bin");
        let host = Fake::new(Os::Other).path(&[dir]).file("/usr/bin/node");
        assert_eq!(found(&host, Tool::Node), Some(dir.join("node")));

        let brew = Fake::new(Os::Other).file("/opt/homebrew/bin/ffmpeg");
        let found_ffmpeg = locate(&brew, Tool::Ffmpeg).unwrap();
        assert_eq!(found_ffmpeg.path, Path::new("/opt/homebrew/bin/ffmpeg"));
        assert_eq!(found_ffmpeg.origin, Origin::Install("/opt/homebrew/bin"));
        let local = Fake::new(Os::Other).file("/usr/local/bin/ffprobe");
        assert_eq!(
            found(&local, Tool::Ffprobe),
            Some(PathBuf::from("/usr/local/bin/ffprobe"))
        );
        // The earlier PATH entry still beats the Homebrew places.
        let both = Fake::new(Os::Other)
            .path(&[dir])
            .file("/usr/bin/ffmpeg")
            .file("/opt/homebrew/bin/ffmpeg");
        assert_eq!(found(&both, Tool::Ffmpeg), Some(dir.join("ffmpeg")));
    }

    #[test]
    fn unix_empty_path_entries_keep_their_previous_current_directory_semantics() {
        for tool in [Tool::Ffmpeg, Tool::Ffprobe, Tool::Node] {
            let host = Fake::new(Os::Other).var("PATH", "").file(tool.binary());
            assert_eq!(found(&host, tool), Some(PathBuf::from(tool.binary())));
        }
        let host = Fake::new(Os::Other).file("ffmpeg").file("node");
        assert_eq!(found(&host, Tool::Ffmpeg), Some(PathBuf::from("ffmpeg")));
        assert_eq!(
            found(&host, Tool::Node),
            None,
            "Node did not search an absent PATH"
        );
    }

    #[test]
    fn other_systems_do_not_use_extensions_or_the_windows_places() {
        let host = Fake::new(Os::Other)
            .var("LOCALAPPDATA", LOCAL)
            .var("ProgramFiles", FILES)
            .var("PATHEXT", ".EXE")
            .path(&[Path::new("/usr/bin")])
            .file("/usr/bin/node.exe")
            .file(at(FILES, &["nodejs", "node.exe"]))
            .file(gyan("ffmpeg-9.0.2-full_build", "ffmpeg.exe"))
            .file(at(LOCAL, &["Microsoft", "WinGet", "Links", "ffmpeg.exe"]));
        assert_eq!(found(&host, Tool::Node), None);
        assert_eq!(found(&host, Tool::Ffmpeg), None);
    }

    #[test]
    fn node_never_looked_in_homebrew_outside_windows() {
        // Preserved on purpose: only FFmpeg had the Homebrew places.
        let host = Fake::new(Os::Other).file("/opt/homebrew/bin/node");
        assert_eq!(found(&host, Tool::Node), None);
    }

    #[test]
    fn windows_does_not_use_the_homebrew_places() {
        let host = windows().file("/opt/homebrew/bin/ffmpeg.exe");
        assert_eq!(found(&host, Tool::Ffmpeg), None);
    }

    /// Not a check: prints where each tool resolves on *this* machine, with its origin
    /// (`cargo test -p tessel-editor-media --lib -- --ignored --nocapture real_machine`).
    #[test]
    #[ignore = "reads this machine's environment and disk; run it by name to see the resolution"]
    fn real_machine_resolution_is_reported() {
        for tool in [Tool::Ffmpeg, Tool::Ffprobe, Tool::Node] {
            match locate(&System, tool) {
                Some(found) => println!(
                    "{}: {} (origin: {})",
                    tool.binary(),
                    found.path.display(),
                    found.origin
                ),
                None => println!("{}: not found", tool.binary()),
            }
        }
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(version_key("10.0-full_build") > version_key("9.9.9-full_build"));
        assert_eq!(version_key("9.0.2-full_build"), [9, 0, 2]);
        assert_eq!(version_key("2025-01-01-git-abc-full_build"), [2025]);
        assert!(version_key("garbage").is_empty());
    }
}

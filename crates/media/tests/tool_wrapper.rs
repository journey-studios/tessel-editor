#[cfg(unix)]
mod common;

#[cfg(unix)]
#[test]
fn unix_wrapper_quotes_both_paths_and_preserves_arguments() {
    use std::{fs, os::unix::fs::PermissionsExt, process::Command};
    let root = std::env::temp_dir().join(format!("tessel-wrapper-quote-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let real = root.join("tool's executable");
    let flag = root.join("can't continue");
    fs::write(&real, "#!/bin/sh\nprintf '%s\\n' \"$@\"\n").unwrap();
    fs::set_permissions(&real, fs::Permissions::from_mode(0o755)).unwrap();
    let wrapper = common::wrapper::forwarding_tool(&root, "wrapper", &real, &flag, 0.0);
    let args = ["one two", "three'four", "$HOME;$(false)"];
    let output = Command::new(&wrapper).args(args).output().unwrap();
    assert!(output.status.success(), "{:?}", output);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        args.join("\n") + "\n"
    );
    fs::write(&flag, "stop").unwrap();
    let refused = Command::new(wrapper).output().unwrap();
    assert_eq!(refused.status.code(), Some(1));
    assert!(
        String::from_utf8(refused.stderr)
            .unwrap()
            .contains("forced failure")
    );
    fs::remove_dir_all(root).unwrap();
}

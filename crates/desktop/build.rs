//! The editor's main thread runs GPUI. On Windows its stack is the size the executable's header reserves,
//! 1 MiB by default, where macOS and Linux give the main thread 8 MiB. A debug build overflowed 1 MiB after
//! a few effect edits in the real window (BL168.9, `scripts/scene-fx-ui-proof.py`: "thread 'main' has
//! overflowed its stack"), so the Windows executable reserves the same 8 MiB. Only the binaries of this
//! package are linked with it; tests run on threads of their own and are unaffected.
//! Checked by `tests/windows_stack.rs`.

const MAIN_STACK_BYTES: u64 = 8 * 1024 * 1024;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    match std::env::var("CARGO_CFG_TARGET_ENV").as_deref() {
        Ok("msvc") => println!("cargo:rustc-link-arg-bins=/STACK:{MAIN_STACK_BYTES}"),
        Ok("gnu") => println!("cargo:rustc-link-arg-bins=-Wl,--stack,{MAIN_STACK_BYTES}"),
        _ => {}
    }
}

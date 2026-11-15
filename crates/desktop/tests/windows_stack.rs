//! BL168.9: the editor's main thread (GPUI) gets the 8 MiB stack macOS and Linux give it. On Windows that
//! size is the `SizeOfStackReserve` of the executable's PE header, set by `build.rs`; with the 1 MiB default a
//! debug build overflowed in the real window. This reads the header of the binary Cargo built for this test.
#![cfg(windows)]

const WANTED: u64 = 8 * 1024 * 1024;

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap())
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

/// `SizeOfStackReserve` of a PE32+ image: the optional header follows the `PE\0\0` signature and the
/// 20-byte COFF header, and the field is at offset 72 of it.
fn stack_reserve(image: &[u8]) -> u64 {
    assert_eq!(&image[..2], b"MZ", "not a PE image");
    let pe = u32_at(image, 0x3c) as usize;
    assert_eq!(&image[pe..pe + 4], b"PE\0\0", "no PE signature");
    let optional = pe + 4 + 20;
    assert_eq!(u16_at(image, optional), 0x20b, "not PE32+ (64-bit)");
    u64::from_le_bytes(image[optional + 72..optional + 80].try_into().unwrap())
}

#[test]
fn the_editor_reserves_an_8_mib_main_thread_stack() {
    let path = env!("CARGO_BIN_EXE_tessel-editor");
    let image = std::fs::read(path).unwrap_or_else(|error| panic!("{path}: {error}"));
    assert_eq!(stack_reserve(&image), WANTED, "{path}");
}

#[test]
fn the_header_reader_sees_the_default_of_another_executable() {
    // A control: the test binary itself is linked without the flag, so the reader must not answer 8 MiB for it
    // (a reader that always said 8 MiB would pass the test above for nothing).
    let image = std::fs::read(std::env::current_exe().unwrap()).unwrap();
    assert_ne!(stack_reserve(&image), WANTED);
}

#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
fn main() {
    if let Err(error) = synapta_connect::portable::run() {
        synapta_connect::platform::win::message_box("Synapta Connect Portable", &format!("{error:#}"));
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Synapta Connect Portable requires Windows.");
    std::process::exit(1);
}

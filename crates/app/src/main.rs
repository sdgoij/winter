//! The Winter executable.
//!
//! On Windows this is a GUI-subsystem binary: a double-click opens the window,
//! not a console window. Diagnostics go to the trace file (ADR 081).

#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    {
        winter_winui::run()?;
    }
    #[cfg(target_os = "macos")]
    {
        winter_cocoa::run()?;
    }
    #[cfg(target_os = "linux")]
    {
        winter_gtk4::run()?;
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    compile_error!("Winter has a shell on Windows, macOS and Linux only");
    Ok(())
}

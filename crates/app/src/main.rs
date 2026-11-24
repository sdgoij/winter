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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    {
        winter::winui::run()?;
    }
    #[cfg(target_os = "macos")]
    {
        winter::macos::run()?;
    }
    #[cfg(target_os = "linux")]
    {
        winter::linux::run()?;
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    compile_error!("Winter has a shell on Windows, macOS and Linux only");
    Ok(())
}

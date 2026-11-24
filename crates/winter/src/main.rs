fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    {
        winter::winui::run()?;
    }
    #[cfg(target_os = "macos")]
    {
        winter::macos::run()?;
    }
    // The GTK shell is the Linux shell (ADR 074). `--no-default-features`
    // leaves it out and the egui shell below runs instead.
    #[cfg(all(target_os = "linux", feature = "gtk"))]
    {
        winter::linux::run()?;
    }
    #[cfg(all(
        not(windows),
        not(target_os = "macos"),
        not(all(target_os = "linux", feature = "gtk"))
    ))]
    {
        winter::app::run()?;
    }
    Ok(())
}

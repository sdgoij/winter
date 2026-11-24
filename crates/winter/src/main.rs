fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    {
        winter::winui::run()?;
    }
    #[cfg(target_os = "macos")]
    {
        winter::macos::run()?;
    }
    // The GTK shell is parked (ADR 062) and opt-in: Linux runs the egui shell
    // below unless `cargo run --features gtk` asks for this one.
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

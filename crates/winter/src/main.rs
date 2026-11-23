fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    {
        winter::winui::run()?;
    }
    // The GTK shell is opt-in until it reaches parity with the egui one
    // (ADR 061): `cargo run --features gtk` picks it on Linux.
    #[cfg(all(target_os = "linux", feature = "gtk"))]
    {
        winter::linux::run()?;
    }
    #[cfg(all(not(windows), not(all(target_os = "linux", feature = "gtk"))))]
    {
        winter::app::run()?;
    }
    Ok(())
}

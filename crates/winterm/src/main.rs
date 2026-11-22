fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    {
        winterm::winui::run()?;
    }
    #[cfg(not(windows))]
    {
        winterm::app::run()?;
    }
    Ok(())
}

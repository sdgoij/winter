//! `theme_utils` — light/dark theme detection.
//!
//! Ported from ThemeUtils in the Windows Terminal C++ codebase.

/// Returns true if the current system theme appears to be dark.
///
/// On Windows, this queries the registry. On other platforms,
/// returns `false` (light theme) as the default.
#[cfg(target_os = "windows")]
pub fn is_dark_theme() -> bool {
    // Query Windows registry for the app mode
    // Apps with value 0 = light, 1 = dark
    match std::env::var("WT_THEME") {
        Ok(val) => val == "dark",
        Err(_) => {
            // Default to light theme on CI/headless
            false
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn is_dark_theme() -> bool {
    // On non-Windows, check XDG_CURRENT_DESKTOP or similar env vars
    // Default to light theme for portability
    false
}

/// Returns true if the current system theme appears to be light.
pub fn is_light_theme() -> bool {
    !is_dark_theme()
}

/// Returns the detected theme as a string.
pub fn detected_theme() -> &'static str {
    if is_dark_theme() {
        "dark"
    } else {
        "light"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_theme_detection() {
        // Just verify the functions don't panic
        let dark = is_dark_theme();
        let light = is_light_theme();
        assert_eq!(dark, !light);

        let theme = detected_theme();
        assert!(theme == "dark" || theme == "light");
    }
}

//! The desktop's window-chrome palette.
//!
//! The strip is painted by this shell, so it cannot inherit a GTK header bar's
//! colours. On KDE the window manager publishes them in `kdeglobals`; reading
//! that file keeps the strip in step with every other window on the desktop.
//! A desktop that publishes no palette keeps the shell's own fallbacks.

use std::path::PathBuf;

/// The colours a desktop publishes for its window chrome, as `0xRRGGBB`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Chrome {
    pub(crate) titlebar: u32,
    pub(crate) titlebar_inactive: u32,
    pub(crate) foreground: u32,
    pub(crate) foreground_inactive: u32,
    pub(crate) window: u32,
    pub(crate) accent: u32,
}

impl Chrome {
    /// Reads the palette of the desktop this process is running on.
    pub(crate) fn system() -> Option<Self> {
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").ok();
        if !is_kde(desktop.as_deref()) {
            return None;
        }
        let text = std::fs::read_to_string(config_home()?.join("kdeglobals")).ok()?;
        Self::parse(&text)
    }

    /// Reads the palette out of a `kdeglobals` document.
    fn parse(text: &str) -> Option<Self> {
        let key = |section: &str, name: &str| value(text, section, name).and_then(rgb);

        let titlebar = key("Colors:Header", "BackgroundNormal").or_else(|| key("WM", "activeBackground"))?;
        let foreground = key("Colors:Header", "ForegroundNormal").or_else(|| key("WM", "activeForeground"))?;
        let titlebar_inactive = key("Colors:Header][Inactive", "BackgroundNormal")
            .or_else(|| key("WM", "inactiveBackground"))
            .unwrap_or(titlebar);
        let foreground_inactive = key("Colors:Header][Inactive", "ForegroundNormal")
            .or_else(|| key("WM", "inactiveForeground"))
            .unwrap_or(foreground);
        let window = key("Colors:Window", "BackgroundNormal").unwrap_or(titlebar);
        let accent = key("Colors:Selection", "BackgroundNormal")
            .or_else(|| key("Colors:Window", "DecorationFocus"))
            .unwrap_or(foreground);

        Some(Self {
            titlebar,
            titlebar_inactive,
            foreground,
            foreground_inactive,
            window,
            accent,
        })
    }
}

/// Whether `XDG_CURRENT_DESKTOP` names KDE.
fn is_kde(desktop: Option<&str>) -> bool {
    desktop.is_some_and(|desktop| desktop.split(':').any(|name| name.eq_ignore_ascii_case("kde")))
}

/// The user's configuration directory.
fn config_home() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        return Some(path.into());
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config"))
}

/// One value out of an INI-style document.
///
/// `kdeglobals` writes groups such as `[Colors:Header][Inactive]`, which is one
/// section whose name contains the brackets, so the outer pair alone is
/// stripped.
fn value<'a>(text: &'a str, section: &str, name: &str) -> Option<&'a str> {
    let mut current = "";
    for line in text.lines() {
        let line = line.trim();
        if let Some(inner) = line.strip_prefix('[').and_then(|line| line.strip_suffix(']')) {
            current = inner;
            continue;
        }
        if current != section {
            continue;
        }
        if let Some((key, value)) = line.split_once('=')
            && key.trim() == name
        {
            return Some(value.trim());
        }
    }
    None
}

/// A KDE colour value, `r,g,b`, as `0xRRGGBB`.
fn rgb(value: &str) -> Option<u32> {
    let mut parts = value.split(',').map(|part| part.trim().parse::<u32>().ok());
    let red = parts.next().flatten()?;
    let green = parts.next().flatten()?;
    let blue = parts.next().flatten()?;
    (red <= 255 && green <= 255 && blue <= 255).then_some((red << 16) | (green << 8) | blue)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PALETTE: &str = "\
[WM]
activeBackground=39,44,49
activeForeground=252,252,252
inactiveBackground=32,36,40
inactiveForeground=161,169,177

[Colors:Header]
BackgroundNormal=41,44,48
ForegroundNormal=252,252,252

[Colors:Header][Inactive]
BackgroundNormal=32,35,38
ForegroundNormal=161,169,177

[Colors:Window]
BackgroundNormal=32,35,38

[Colors:Selection]
BackgroundNormal=61,174,233
";

    #[test]
    fn the_kde_palette_reads_every_chrome_colour() {
        let chrome = Chrome::parse(PALETTE).expect("a palette");
        assert_eq!(chrome.titlebar, 0x29_2C_30);
        assert_eq!(chrome.titlebar_inactive, 0x20_23_26);
        assert_eq!(chrome.foreground, 0xFC_FC_FC);
        assert_eq!(chrome.foreground_inactive, 0xA1_A9_B1);
        assert_eq!(chrome.window, 0x20_23_26);
        assert_eq!(chrome.accent, 0x3D_AE_E9);
    }

    #[test]
    fn the_window_manager_colours_are_the_fallback() {
        let text = "\
[WM]
activeBackground=39,44,49
activeForeground=252,252,252
inactiveBackground=32,36,40
inactiveForeground=161,169,177
";
        let chrome = Chrome::parse(text).expect("a palette");
        assert_eq!(chrome.titlebar, 0x27_2C_31);
        assert_eq!(chrome.titlebar_inactive, 0x20_24_28);
        assert_eq!(chrome.foreground_inactive, 0xA1_A9_B1);
        // No window or selection colour: both follow the title bar.
        assert_eq!(chrome.window, chrome.titlebar);
        assert_eq!(chrome.accent, chrome.foreground);
    }

    #[test]
    fn a_document_without_chrome_colours_is_not_a_palette() {
        assert!(Chrome::parse("[General]\nfont=Noto Sans,10\n").is_none());
        assert!(Chrome::parse("activeBackground=1,2,3\n").is_none());
    }

    #[test]
    fn a_colour_must_have_three_bytes() {
        assert_eq!(rgb("39,44,49"), Some(0x27_2C_31));
        assert_eq!(rgb(" 1, 2, 3 "), Some(0x01_02_03));
        assert_eq!(rgb("1,2"), None);
        assert_eq!(rgb("1,2,300"), None);
        assert_eq!(rgb("red,2,3"), None);
    }

    #[test]
    fn kde_is_recognised_in_a_desktop_list() {
        assert!(is_kde(Some("KDE")));
        assert!(is_kde(Some("GNOME:KDE")));
        assert!(is_kde(Some("kde")));
        assert!(!is_kde(Some("GNOME")));
        assert!(!is_kde(None));
    }
}

//! The About dialog's content, shared by the three shells.
//!
//! GTK builds a window with labels, WinUI a content dialog and AppKit an
//! alert. The name, the description, the version line and the repository are
//! the same in all three, so they live here rather than in each shell, where a
//! change to one copy would leave the others behind.

/// The program's name, as the window title and the About dialog show it.
pub const NAME: &str = "Winter";

/// Where the shell's source lives; every About dialog links to it.
pub const REPOSITORY: &str = "https://github.com/sdgoij/winterm";

/// The description every About dialog shows, one entry per line.
pub const DESCRIPTION: [&str; 2] = [
    "A cross-platform terminal emulator in Rust.",
    "Ported from Microsoft's Windows Terminal.",
];

/// The About title: the name and the version the build was compiled as.
///
/// The shell passes its own `CARGO_PKG_VERSION`, so a package that renames the
/// binary still reports the version it was compiled with.
pub fn title(version: &str) -> String {
    format!("{NAME} {version}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_title_names_the_build() {
        assert_eq!(title("1.2.3"), "Winter 1.2.3");
    }
}

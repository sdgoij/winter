//! The settings draft and the status a settings editor shows.
//!
//! From `TerminalSettingsEditor/` and the way `CascadiaSettings` is loaded and
//! saved. The model holds the file's values apart from the ones in force, so a
//! dirty draft is computed by comparison rather than a flag (ADRs 028, 030);
//! each shell draws its own window over it.

use std::path::Path;

use settings::Settings;

/// The settings the window can edit, held apart from the settings in force.
///
/// Whether the draft is dirty is computed by comparing it with the settings in
/// force, so no widget has to remember to set a flag (ADRs 028, 030).
pub struct Draft {
    pub font_size: f32,
    pub default_profile: Option<String>,
    pub default_scheme: Option<String>,
    pub theme: Option<String>,
    pub profiles: Vec<ProfileDraft>,
    pub keybindings: Vec<BindingDraft>,
    pub schemes: Vec<SchemeDraft>,
    pub themes: Vec<ThemeDraft>,
}

/// One profile as the window edits it.
///
/// The arguments are text while they are being edited: splitting them on every
/// keystroke would make a space impossible to type. They are split on
/// whitespace when the draft is applied, and the file stays the place for an
/// argument that contains a space (ADR 030).
pub struct ProfileDraft {
    pub name: String,
    pub command: String,
    pub args: String,
    pub title: String,
}

impl ProfileDraft {
    pub fn from_profile(profile: &settings::Profile) -> Self {
        Self {
            name: profile.name.clone(),
            command: profile.command.clone(),
            args: profile.args.join(" "),
            title: profile.title.clone().unwrap_or_default(),
        }
    }

    pub fn to_profile(&self) -> settings::Profile {
        settings::Profile {
            name: self.name.clone(),
            command: self.command.clone(),
            args: self.args.split_whitespace().map(str::to_string).collect(),
            title: (!self.title.is_empty()).then(|| self.title.clone()),
        }
    }
}

/// One chord as the window edits it.
///
/// The chord stays text until the document is built, so a half-typed chord is
/// kept. A row whose text does not parse keeps the window from saving and is
/// named in the window (ADR 031).
pub struct BindingDraft {
    pub keys: String,
    pub action: settings::Action,
}

impl BindingDraft {
    pub fn from_binding(binding: &settings::Keybinding) -> Self {
        Self {
            keys: binding.keys.to_string(),
            action: binding.action,
        }
    }

    pub fn to_binding(&self) -> Option<settings::Keybinding> {
        Some(settings::Keybinding {
            keys: settings::Chord::parse(&self.keys).ok()?,
            action: self.action,
        })
    }
}

/// One scheme as the window edits it.
///
/// A colour is text until the document is built; an empty field means the
/// scheme leaves that colour to the built-in palette, which is different from
/// a field that cannot be parsed (ADR 032). The fields themselves are
/// `settings::SchemeColor`'s, so the form, the reader and the writer cannot
/// disagree about which colours a scheme has (ADR 034).
pub struct SchemeDraft {
    pub name: String,
    pub colours: Vec<String>,
}

impl SchemeDraft {
    pub fn from_scheme(scheme: &settings::Scheme) -> Self {
        Self {
            name: scheme.name.clone(),
            colours: settings::SchemeColor::ALL
                .iter()
                .map(|color| scheme.get(*color).map(|color| color.to_string()).unwrap_or_default())
                .collect(),
        }
    }

    pub fn new(name: String) -> Self {
        Self {
            name,
            colours: vec![String::new(); settings::SchemeColor::ALL.len()],
        }
    }

    pub fn to_scheme(&self) -> Result<settings::Scheme, String> {
        let mut scheme = settings::Scheme {
            name: self.name.clone(),
            ..Default::default()
        };

        for (color, text) in settings::SchemeColor::ALL.iter().zip(&self.colours) {
            if text.trim().is_empty() {
                continue;
            }
            let parsed =
                settings::Color::parse(text).map_err(|_| format!("{}: `{text}` is not a colour", color.name()))?;
            scheme.set(*color, Some(parsed));
        }

        Ok(scheme)
    }
}

/// One theme as the window edits it.
///
/// Four fills, each text until the document is built, like a scheme's colours
/// (ADR 032). An empty field leaves that fill to the shell, and a text that
/// does not parse keeps the window from saving.
pub struct ThemeDraft {
    pub name: String,
    pub tab_row_background: String,
    pub tab_row_unfocused: String,
    pub tab_background: String,
    pub tab_unfocused: String,
}

impl ThemeDraft {
    pub fn from_theme(theme: &settings::Theme) -> Self {
        Self {
            name: theme.name.clone(),
            tab_row_background: written(theme.tab_row.background),
            tab_row_unfocused: written(theme.tab_row.unfocused_background),
            tab_background: written(theme.tab.background),
            tab_unfocused: written(theme.tab.unfocused_background),
        }
    }

    pub fn new(name: String) -> Self {
        Self {
            name,
            tab_row_background: String::new(),
            tab_row_unfocused: String::new(),
            tab_background: String::new(),
            tab_unfocused: String::new(),
        }
    }

    pub fn to_theme(&self) -> Result<settings::Theme, String> {
        Ok(settings::Theme {
            name: self.name.clone(),
            tab_row: settings::TabRowTheme {
                background: read("tab row", &self.tab_row_background)?,
                unfocused_background: read("tab row, unfocused", &self.tab_row_unfocused)?,
            },
            tab: settings::TabTheme {
                background: read("tab", &self.tab_background)?,
                unfocused_background: read("tab, unfocused", &self.tab_unfocused)?,
            },
        })
    }
}

/// A colour as the window's text field writes it.
pub fn written(color: Option<settings::Color>) -> String {
    color.map(|color| color.to_string()).unwrap_or_default()
}

/// A text field's colour, or None when the field is empty.
pub fn read(label: &str, text: &str) -> Result<Option<settings::Color>, String> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    settings::Color::parse(text)
        .map(Some)
        .map_err(|_| format!("{label}: `{text}` is not a colour"))
}

impl Draft {
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            font_size: settings.font.size,
            default_profile: settings.default_profile.clone(),
            default_scheme: settings.default_scheme.clone(),
            theme: settings.theme.clone(),
            profiles: settings.profiles.iter().map(ProfileDraft::from_profile).collect(),
            keybindings: settings.keybindings.iter().map(BindingDraft::from_binding).collect(),
            schemes: settings.schemes.iter().map(SchemeDraft::from_scheme).collect(),
            themes: settings.themes.iter().map(ThemeDraft::from_theme).collect(),
        }
    }

    /// Whether the draft has edits that a save would write.
    ///
    /// An invalid chord or colour counts as an edit: the window must not look
    /// clean while a row cannot be saved.
    pub fn differs_from(&self, settings: &Settings) -> bool {
        self.document(settings).is_none_or(|document| document != *settings)
    }

    /// The document a save writes, or None when a value does not parse.
    pub fn document(&self, settings: &Settings) -> Option<Settings> {
        let mut document = settings.clone();
        document.font.size = self.font_size;
        document.default_profile = self.default_profile.clone();
        document.default_scheme = self.default_scheme.clone();
        document.theme = self.theme.clone();
        document.profiles = self.profiles.iter().map(ProfileDraft::to_profile).collect();
        document.keybindings = self
            .keybindings
            .iter()
            .map(BindingDraft::to_binding)
            .collect::<Option<Vec<_>>>()?;
        document.schemes = self
            .schemes
            .iter()
            .map(SchemeDraft::to_scheme)
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        document.themes = self
            .themes
            .iter()
            .map(ThemeDraft::to_theme)
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        Some(document)
    }

    /// Why the themes cannot be built, one message per bad colour.
    pub fn theme_errors(&self) -> Vec<String> {
        self.themes.iter().filter_map(|theme| theme.to_theme().err()).collect()
    }

    /// Why the schemes cannot be built, one message per bad colour.
    pub fn scheme_errors(&self) -> Vec<String> {
        self.schemes
            .iter()
            .filter_map(|scheme| scheme.to_scheme().err())
            .collect()
    }

    /// The file a save would write, as the window shows it.
    ///
    /// Nothing is shown while a value does not parse; the editor already says
    /// which row blocks the save.
    pub fn preview(&self, settings: &Settings) -> Option<String> {
        let document = self.document(settings)?;
        Some(settings::to_toml(&document))
    }
}
pub fn status_text(
    path: Option<&Path>,
    migrated_from: Option<&Path>,
    error: Option<&str>,
    warnings: &[String],
) -> String {
    let mut text = match path {
        Some(path) => format!("File: {}\n", path.display()),
        None => "File: the platform has no settings path; the defaults are in force.\n".to_string(),
    };
    if let Some(path) = migrated_from {
        text.push_str(&format!("Migrated from: {}\n", path.display()));
    }
    if let Some(error) = error {
        text.push_str(&format!("\nThe file was not used: {error}\n"));
    }
    for warning in warnings {
        text.push_str(&format!("Adjusted: {warning}\n"));
    }
    if error.is_none() && warnings.is_empty() && migrated_from.is_none() {
        text.push_str("Every value in the file was usable.\n");
    }
    text
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_names_the_file_and_every_adjustment() {
        let path = Path::new("/home/u/.config/winter/settings.toml");
        let old = Path::new("/home/u/.config/winter/settings.json");
        let warnings = vec!["default_profile: no such profile".to_string()];
        let text = status_text(Some(path), Some(old), None, &warnings);

        assert!(text.contains("File: /home/u/.config/winter/settings.toml"));
        assert!(text.contains("Migrated from: /home/u/.config/winter/settings.json"));
        assert!(text.contains("Adjusted: default_profile: no such profile"));
    }

    #[test]
    fn a_rejected_file_says_why_it_was_not_used() {
        let text = status_text(None, None, Some("bad TOML"), &[]);

        assert!(text.contains("the defaults are in force"));
        assert!(text.contains("The file was not used: bad TOML"));
        assert!(!text.contains("Every value"));
    }

    #[test]
    fn a_clean_file_says_so() {
        let text = status_text(Some(Path::new("settings.toml")), None, None, &[]);
        assert!(text.contains("Every value in the file was usable."));
    }
}

//! The settings window and its draft, from `TerminalSettingsEditor/` and the
//! way `CascadiaSettings` is loaded and saved.
//!
//! The window edits everything the file holds and writes it through
//! `settings::save`; the poll that reloads a saved file lives here too. The
//! state it edits is `App`'s, so the methods are `impl App` blocks (ADR 038).

use std::path::Path;

use eframe::egui;
use settings::Settings;

use crate::app::App;
use crate::terminal_widget::egui_color;

/// The colour of a conflict notice in the settings window.
pub(crate) const CONFLICT: egui::Color32 = egui::Color32::from_rgb(220, 170, 60);

/// The settings the window can edit, held apart from the settings in force.
///
/// Whether the draft is dirty is computed by comparing it with the settings in
/// force, so no widget has to remember to set a flag (ADRs 028, 030).
pub(crate) struct Draft {
    pub(crate) font_size: f32,
    pub(crate) default_profile: Option<String>,
    pub(crate) default_scheme: Option<String>,
    pub(crate) theme: Option<String>,
    pub(crate) profiles: Vec<ProfileDraft>,
    pub(crate) keybindings: Vec<BindingDraft>,
    pub(crate) schemes: Vec<SchemeDraft>,
    pub(crate) themes: Vec<ThemeDraft>,
}

/// One profile as the window edits it.
///
/// The arguments are text while they are being edited: splitting them on every
/// keystroke would make a space impossible to type. They are split on
/// whitespace when the draft is applied, and the file stays the place for an
/// argument that contains a space (ADR 030).
pub(crate) struct ProfileDraft {
    pub(crate) name: String,
    pub(crate) command: String,
    pub(crate) args: String,
    pub(crate) title: String,
}

impl ProfileDraft {
    pub(crate) fn from_profile(profile: &settings::Profile) -> Self {
        Self {
            name: profile.name.clone(),
            command: profile.command.clone(),
            args: profile.args.join(" "),
            title: profile.title.clone().unwrap_or_default(),
        }
    }

    pub(crate) fn to_profile(&self) -> settings::Profile {
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
pub(crate) struct BindingDraft {
    pub(crate) keys: String,
    pub(crate) action: settings::Action,
}

impl BindingDraft {
    pub(crate) fn from_binding(binding: &settings::Keybinding) -> Self {
        Self {
            keys: binding.keys.to_string(),
            action: binding.action,
        }
    }

    pub(crate) fn to_binding(&self) -> Option<settings::Keybinding> {
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
pub(crate) struct SchemeDraft {
    pub(crate) name: String,
    pub(crate) colours: Vec<String>,
}

impl SchemeDraft {
    pub(crate) fn from_scheme(scheme: &settings::Scheme) -> Self {
        Self {
            name: scheme.name.clone(),
            colours: settings::SchemeColor::ALL
                .iter()
                .map(|color| scheme.get(*color).map(|color| color.to_string()).unwrap_or_default())
                .collect(),
        }
    }

    pub(crate) fn new(name: String) -> Self {
        Self {
            name,
            colours: vec![String::new(); settings::SchemeColor::ALL.len()],
        }
    }

    pub(crate) fn to_scheme(&self) -> Result<settings::Scheme, String> {
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
pub(crate) struct ThemeDraft {
    pub(crate) name: String,
    pub(crate) tab_row_background: String,
    pub(crate) tab_row_unfocused: String,
    pub(crate) tab_background: String,
    pub(crate) tab_unfocused: String,
}

impl ThemeDraft {
    pub(crate) fn from_theme(theme: &settings::Theme) -> Self {
        Self {
            name: theme.name.clone(),
            tab_row_background: written(theme.tab_row.background),
            tab_row_unfocused: written(theme.tab_row.unfocused_background),
            tab_background: written(theme.tab.background),
            tab_unfocused: written(theme.tab.unfocused_background),
        }
    }

    pub(crate) fn new(name: String) -> Self {
        Self {
            name,
            tab_row_background: String::new(),
            tab_row_unfocused: String::new(),
            tab_background: String::new(),
            tab_unfocused: String::new(),
        }
    }

    pub(crate) fn to_theme(&self) -> Result<settings::Theme, String> {
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
pub(crate) fn written(color: Option<settings::Color>) -> String {
    color.map(|color| color.to_string()).unwrap_or_default()
}

/// A text field's colour, or None when the field is empty.
pub(crate) fn read(label: &str, text: &str) -> Result<Option<settings::Color>, String> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    settings::Color::parse(text)
        .map(Some)
        .map_err(|_| format!("{label}: `{text}` is not a colour"))
}

impl Draft {
    pub(crate) fn from_settings(settings: &Settings) -> Self {
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
    pub(crate) fn differs_from(&self, settings: &Settings) -> bool {
        self.document(settings).is_none_or(|document| document != *settings)
    }

    /// The document a save writes, or None when a value does not parse.
    pub(crate) fn document(&self, settings: &Settings) -> Option<Settings> {
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
    pub(crate) fn theme_errors(&self) -> Vec<String> {
        self.themes.iter().filter_map(|theme| theme.to_theme().err()).collect()
    }

    /// Why the schemes cannot be built, one message per bad colour.
    pub(crate) fn scheme_errors(&self) -> Vec<String> {
        self.schemes
            .iter()
            .filter_map(|scheme| scheme.to_scheme().err())
            .collect()
    }

    /// The file a save would write, as the window shows it.
    ///
    /// Nothing is shown while a value does not parse; the editor already says
    /// which row blocks the save.
    pub(crate) fn preview(&self, settings: &Settings) -> Option<String> {
        let document = self.document(settings)?;
        Some(settings::to_toml(&document))
    }
}

impl App {
    /// Shows or hides the settings window, rebuilding the draft when it opens.
    ///
    /// Closing discards unsaved edits, which is what the window says while the
    /// draft is dirty.
    pub(crate) fn toggle_settings(&mut self) {
        self.settings_open = !self.settings_open;
        self.settings_conflict = false;
        self.draft = if self.settings_open {
            Some(Draft::from_settings(&self.settings))
        } else {
            None
        };
    }
}

impl App {
    /// Draws what the settings file said, whether it was used, and the fields
    /// the window can change.
    pub(crate) fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.settings_open;
        let mut save = false;
        let mut revert = false;
        let rows = settings_rows(
            &self.settings,
            self.settings_path.as_deref(),
            self.settings_error.as_deref(),
        );

        egui::Window::new("Settings")
            .open(&mut open)
            .default_width(520.0)
            .default_height(560.0)
            .resizable(true)
            .show(ctx, |ui| {
                // The editor grew past a screen's height once profiles,
                // keybindings, schemes and the preview were all here, and a
                // window does not scroll itself: the Save button was below the
                // bottom edge.
                egui::ScrollArea::vertical().id_salt("winter-settings").show(ui, |ui| {
                    egui::Grid::new("winter-settings-rows")
                        .num_columns(2)
                        .spacing([16.0, 4.0])
                        .show(ui, |ui| {
                            for (label, value) in &rows {
                                ui.strong(*label);
                                ui.label(value);
                                ui.end_row();
                            }
                        });

                    ui.separator();
                    if let Some(draft) = self.draft.as_mut() {
                        draft_editor(
                            ui,
                            draft,
                            &self.settings,
                            self.settings_conflict,
                            &mut save,
                            &mut revert,
                        );
                    }

                    ui.separator();
                    ui.label("The file is also reloaded when another program saves it.");

                    if !self.settings_warnings.is_empty() {
                        ui.separator();
                        ui.strong("Adjusted on load");
                        for warning in &self.settings_warnings {
                            ui.label(warning);
                        }
                    }

                    if let Some(path) = &self.settings_migrated_from {
                        ui.separator();
                        ui.strong("Migrated from");
                        ui.label(format!("{} was read and written as this file.", path.display()));
                    }
                });
            });

        self.settings_open = open;
        if !self.settings_open {
            self.draft = None;
            self.settings_conflict = false;
        }

        if save {
            self.save_draft();
        }
        if revert {
            self.reload_settings();
        }
    }

    /// Writes the draft and applies what was written.
    ///
    /// The document is read back through the same path as startup, so what runs
    /// is always the file's own settings, normalized.
    pub(crate) fn save_draft(&mut self) {
        let Some(draft) = &self.draft else {
            return;
        };
        let Some(document) = draft.document(&self.settings) else {
            return;
        };

        match settings::save(&document) {
            Ok(()) => {
                self.settings_conflict = false;
                self.settings_error = None;
                self.reload_settings();
                if self.settings_open {
                    self.draft = Some(Draft::from_settings(&self.settings));
                }
            }
            Err(error) => self.settings_error = Some(error.to_string()),
        }
    }
}

impl App {
    /// Re-reads the settings file and applies what changed.
    ///
    /// Chords and the font size take effect at once, a scheme repaints every
    /// pane, and a changed profile affects the next tab: a running shell keeps
    /// the command it started with.
    pub(crate) fn reload_settings(&mut self) {
        let loaded = settings::load_with_report();
        self.settings_stamp = loaded.path.as_deref().and_then(settings::stamp);
        self.settings_migrated_from = loaded.migrated_from.clone();
        if let Some(error) = loaded.error {
            // A half-written save must not wipe a running session: the previous
            // settings stay in force and the window reports the error.
            self.settings_error = Some(error.to_string());
            return;
        }

        self.settings = loaded.settings;
        self.settings_path = loaded.path;
        self.settings_error = None;
        self.settings_warnings = loaded.warnings.iter().map(ToString::to_string).collect();
        self.font_size = self.settings.font.size;
        self.scheme = self.settings.scheme().cloned();

        let scheme = self.scheme.clone();
        for index in 0..self.tabs.len() {
            if let Some(tab) = self.tabs.get_mut(index) {
                tab.apply_scheme(scheme.as_ref());
            }
        }

        if self.settings_open {
            self.draft = Some(Draft::from_settings(&self.settings));
        }
    }
}

/// The rows the settings window shows.
///
/// Kept out of the window so the strings are testable without an egui context;
/// the window adds only layout.
pub(crate) fn settings_rows(
    settings: &Settings,
    path: Option<&Path>,
    error: Option<&str>,
) -> Vec<(&'static str, String)> {
    let mut rows = Vec::with_capacity(8);

    rows.push((
        "File",
        path.map_or_else(
            || "(none on this platform)".to_string(),
            |path| path.display().to_string(),
        ),
    ));
    rows.push(("Status", error.unwrap_or("loaded").to_string()));

    let profile = settings.profile();
    rows.push((
        "Profile",
        profile.map_or_else(
            || "(none)".to_string(),
            |profile| format!("{} ({})", profile.name, profile.command),
        ),
    ));

    rows.push(("Keybindings", settings.keybindings.len().to_string()));
    rows.push((
        "Scheme",
        settings
            .scheme()
            .map_or_else(|| "(built-in)".to_string(), |scheme| scheme.name.clone()),
    ));
    rows.push((
        "Theme",
        settings
            .theme()
            .map_or_else(|| "(shell)".to_string(), |theme| theme.name.clone()),
    ));
    rows.push(("Font", format!("{} pt", settings.font.size)));

    rows
}

/// Draws a colour field's swatch, or what an empty or broken field means.
pub(crate) fn color_swatch(ui: &mut egui::Ui, text: &str, empty: &str) {
    match settings::Color::parse(text)
        .ok()
        .and_then(|color| egui_color(color.text_color()))
    {
        Some(color) => {
            ui.colored_label(color, "██");
        }
        None if text.trim().is_empty() => {
            ui.label(empty);
        }
        None => {
            ui.label("");
        }
    }
}

/// Draws the editable fields and the Save/Revert buttons.
pub(crate) fn draft_editor(
    ui: &mut egui::Ui,
    draft: &mut Draft,
    settings: &Settings,
    conflict: bool,
    save: &mut bool,
    revert: &mut bool,
) {
    egui::Grid::new("winter-settings-editor")
        .num_columns(2)
        .spacing([16.0, 4.0])
        .show(ui, |ui| {
            ui.strong("Font size");
            ui.add(egui::DragValue::new(&mut draft.font_size).range(4.0..=100.0).speed(0.5));
            ui.end_row();

            ui.strong("Default profile");
            let names: Vec<String> = draft.profiles.iter().map(|profile| profile.name.clone()).collect();
            let selected = draft.default_profile.clone().unwrap_or_default();
            let selected = if selected.is_empty() {
                "(none)".to_string()
            } else {
                selected
            };
            egui::ComboBox::from_id_salt("winter-settings-profile")
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    for name in names {
                        ui.selectable_value(&mut draft.default_profile, Some(name.clone()), name);
                    }
                });
            ui.end_row();

            ui.strong("Scheme");
            let names: Vec<String> = draft.schemes.iter().map(|scheme| scheme.name.clone()).collect();
            let selected = draft.default_scheme.clone().unwrap_or_else(|| "built-in".to_string());
            egui::ComboBox::from_id_salt("winter-settings-scheme")
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut draft.default_scheme, None, "built-in");
                    for name in names {
                        ui.selectable_value(&mut draft.default_scheme, Some(name.clone()), name);
                    }
                });
            ui.end_row();

            ui.strong("Theme");
            let names: Vec<String> = draft.themes.iter().map(|theme| theme.name.clone()).collect();
            let selected = draft.theme.clone().unwrap_or_else(|| "shell".to_string());
            egui::ComboBox::from_id_salt("winter-settings-theme")
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut draft.theme, None, "shell");
                    for name in names {
                        ui.selectable_value(&mut draft.theme, Some(name.clone()), name);
                    }
                });
            ui.end_row();
        });

    ui.separator();
    ui.strong("Profiles");
    ui.label("Arguments are split on spaces; quote an argument in the file.");

    let mut remove = None;
    egui::ScrollArea::vertical()
        .id_salt("winter-profiles")
        .max_height(220.0)
        .show(ui, |ui| {
            for (index, profile) in draft.profiles.iter_mut().enumerate() {
                egui::Grid::new(("winter-profile", index))
                    .num_columns(2)
                    .spacing([16.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Name");
                        ui.text_edit_singleline(&mut profile.name);
                        ui.end_row();

                        ui.label("Command");
                        ui.text_edit_singleline(&mut profile.command);
                        ui.end_row();

                        ui.label("Arguments");
                        ui.text_edit_singleline(&mut profile.args);
                        ui.end_row();

                        ui.label("Title");
                        ui.text_edit_singleline(&mut profile.title);
                        ui.end_row();

                        ui.label("");
                        if ui.small_button("Remove").clicked() {
                            remove = Some(index);
                        }
                        ui.end_row();
                    });
            }
        });
    if let Some(index) = remove {
        draft.profiles.remove(index);
    }
    if ui.button("Add profile").clicked() {
        let mut profile = ProfileDraft::from_profile(&settings::Profile::default());
        profile.name = format!("profile {}", draft.profiles.len() + 1);
        draft.profiles.push(profile);
    }

    ui.separator();
    ui.strong("Keybindings");
    ui.label("A chord is [ctrl+][alt+][shift+]<key>. The file has the full chord syntax.");

    let mut remove = None;
    egui::ScrollArea::vertical()
        .id_salt("winter-bindings")
        .max_height(200.0)
        .show(ui, |ui| {
            for (index, binding) in draft.keybindings.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut binding.keys).desired_width(150.0));
                    egui::ComboBox::from_id_salt(("winter-binding-action", index))
                        .selected_text(binding.action.name())
                        .show_ui(ui, |ui| {
                            for action in settings::Action::ALL {
                                ui.selectable_value(&mut binding.action, *action, action.name());
                            }
                        });
                    if ui.small_button("Remove").clicked() {
                        remove = Some(index);
                    }
                });
            }
        });
    if let Some(index) = remove {
        draft.keybindings.remove(index);
    }
    if ui.button("Add binding").clicked() {
        draft.keybindings.push(BindingDraft {
            keys: String::new(),
            action: settings::Action::NewTab,
        });
    }

    let invalid: Vec<&str> = draft
        .keybindings
        .iter()
        .filter(|binding| binding.to_binding().is_none())
        .map(|binding| binding.keys.as_str())
        .collect();
    if !invalid.is_empty() {
        ui.colored_label(CONFLICT, format!("These chords do not parse: {}", invalid.join(", ")));
    }

    ui.separator();
    ui.strong("Schemes");
    ui.label("A colour is #RGB, #RRGGBB or rgb:r/g/b. An empty field keeps the built-in colour.");

    let mut remove = None;
    egui::ScrollArea::vertical()
        .id_salt("winter-schemes")
        .max_height(280.0)
        .show(ui, |ui| {
            for (index, scheme) in draft.schemes.iter_mut().enumerate() {
                egui::Grid::new(("winter-scheme", index))
                    .num_columns(3)
                    .spacing([12.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Name");
                        ui.text_edit_singleline(&mut scheme.name);
                        ui.label("");
                        ui.end_row();

                        for (color, text) in settings::SchemeColor::ALL.iter().zip(scheme.colours.iter_mut()) {
                            ui.label(color.name());
                            ui.text_edit_singleline(text);
                            color_swatch(ui, text, "built-in");
                            ui.end_row();
                        }

                        ui.label("");
                        if ui.small_button("Remove scheme").clicked() {
                            remove = Some(index);
                        }
                        ui.end_row();
                    });
            }
        });
    if let Some(index) = remove {
        draft.schemes.remove(index);
    }
    if ui.button("Add scheme").clicked() {
        let name = format!("scheme {}", draft.schemes.len() + 1);
        draft.schemes.push(SchemeDraft::new(name));
    }
    for error in draft.scheme_errors() {
        ui.colored_label(CONFLICT, error);
    }

    ui.separator();
    ui.strong("Themes");
    ui.label("A theme paints the tab row and its tabs. An empty field keeps the shell's colour.");

    let mut remove_theme = None;
    egui::ScrollArea::vertical()
        .id_salt("winter-themes")
        .max_height(220.0)
        .show(ui, |ui| {
            for (index, theme) in draft.themes.iter_mut().enumerate() {
                egui::Grid::new(("winter-theme", index))
                    .num_columns(3)
                    .spacing([12.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Name");
                        ui.text_edit_singleline(&mut theme.name);
                        ui.label("");
                        ui.end_row();

                        let fields = [
                            ("tab row", &mut theme.tab_row_background),
                            ("tab row, unfocused", &mut theme.tab_row_unfocused),
                            ("tab", &mut theme.tab_background),
                            ("tab, unfocused", &mut theme.tab_unfocused),
                        ];
                        for (label, text) in fields {
                            ui.label(label);
                            ui.text_edit_singleline(text);
                            color_swatch(ui, text, "shell");
                            ui.end_row();
                        }

                        ui.label("");
                        if ui.small_button("Remove theme").clicked() {
                            remove_theme = Some(index);
                        }
                        ui.end_row();
                    });
            }
        });
    if let Some(index) = remove_theme {
        draft.themes.remove(index);
    }
    if ui.button("Add theme").clicked() {
        let name = format!("theme {}", draft.themes.len() + 1);
        draft.themes.push(ThemeDraft::new(name));
    }
    for error in draft.theme_errors() {
        ui.colored_label(CONFLICT, error);
    }

    ui.separator();
    egui::CollapsingHeader::new("Preview").show(ui, |ui| match draft.preview(settings) {
        Some(text) => {
            egui::ScrollArea::vertical()
                .id_salt("winter-preview")
                .max_height(220.0)
                .show(ui, |ui| {
                    ui.monospace(text);
                });
        }
        None => {
            ui.label("The document cannot be written while a value does not parse.");
        }
    });

    ui.separator();
    let valid = draft.document(settings).is_some();
    let dirty = draft.differs_from(settings);
    ui.horizontal(|ui| {
        if ui.add_enabled(dirty && valid, egui::Button::new("Save")).clicked() {
            *save = true;
        }
        if ui.add_enabled(dirty, egui::Button::new("Revert")).clicked() {
            *revert = true;
        }
    });

    if conflict {
        ui.colored_label(
            CONFLICT,
            "The file changed on disk. Save overwrites it; Revert loads it.",
        );
    }
    if dirty {
        ui.label("Unsaved changes are discarded when this window closes.");
    }
}

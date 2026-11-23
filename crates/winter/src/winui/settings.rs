//! The settings dialog, from `TerminalSettingsEditor/`.
//!
//! The model is `config::Draft`, shared with the egui shell, so both windows
//! save the same document. This module draws it with WinUI 3 and turns each
//! edit into a [`SettingsMessage`]; the shell applies Save and Revert because
//! they touch the running session (ADR 028, ADR 040).

use std::path::{Path, PathBuf};
use std::rc::Rc;

use settings::Settings;
use windows_reactor::*;

use super::app::Message;
use crate::config::{Draft, ProfileDraft, SchemeDraft, ThemeDraft};

/// One edit in the settings dialog.
#[derive(Clone)]
pub(super) enum SettingsMessage {
    Close,
    Save,
    Revert,
    FontSize(f64),
    DefaultProfile(Option<String>),
    DefaultScheme(Option<String>),
    Theme(Option<String>),
    ProfileField {
        index: usize,
        field: ProfileField,
        text: String,
    },
    AddProfile,
    RemoveProfile(usize),
    BindingKeys {
        index: usize,
        text: String,
    },
    BindingAction {
        index: usize,
        action: settings::Action,
    },
    AddBinding,
    RemoveBinding(usize),
    SchemeName {
        index: usize,
        text: String,
    },
    SchemeColor {
        index: usize,
        slot: usize,
        text: String,
    },
    AddScheme,
    RemoveScheme(usize),
    ThemeName {
        index: usize,
        text: String,
    },
    ThemeField {
        index: usize,
        field: ThemeField,
        text: String,
    },
    AddTheme,
    RemoveTheme(usize),
}

/// One field of a profile row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProfileField {
    Name,
    Command,
    Args,
    Title,
}

/// One fill of a theme row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ThemeField {
    TabRow,
    TabRowUnfocused,
    Tab,
    TabUnfocused,
}

/// What the shell must do after an edit.
pub(super) enum SettingsCommand {
    None,
    Save,
    Revert,
    /// The dialog was dismissed; the shell returns focus to the pane.
    Close,
}

/// The dialog's state, its draft and the last report.
pub(super) struct SettingsWindow {
    open: bool,
    draft: Option<Draft>,
    error: Option<String>,
    conflict: bool,
    warnings: Vec<String>,
    migrated_from: Option<PathBuf>,
}

impl SettingsWindow {
    pub(super) fn new(loaded: &settings::Loaded) -> Self {
        Self {
            open: false,
            draft: None,
            error: loaded.error.as_ref().map(ToString::to_string),
            conflict: false,
            warnings: loaded.warnings.iter().map(ToString::to_string).collect(),
            migrated_from: loaded.migrated_from.clone(),
        }
    }

    pub(super) fn is_open(&self) -> bool {
        self.open
    }

    /// Shows the dialog on a fresh draft; closing discards unsaved edits.
    pub(super) fn open(&mut self, settings: &Settings) {
        self.open = true;
        self.conflict = false;
        self.draft = Some(Draft::from_settings(settings));
    }

    pub(super) fn close(&mut self) {
        self.open = false;
        self.conflict = false;
        self.draft = None;
    }

    pub(super) fn draft(&self) -> Option<&Draft> {
        self.draft.as_ref()
    }

    /// Replaces the draft with the settings in force after a save or reload.
    pub(super) fn sync_draft(&mut self, settings: &Settings) {
        if self.open {
            self.draft = Some(Draft::from_settings(settings));
        }
    }

    pub(super) fn set_report(&mut self, warnings: Vec<String>, migrated_from: Option<PathBuf>) {
        self.warnings = warnings;
        self.migrated_from = migrated_from;
    }

    pub(super) fn set_error(&mut self, error: String) {
        self.error = Some(error);
    }

    pub(super) fn clear_error(&mut self) {
        self.error = None;
    }

    pub(super) fn set_conflict(&mut self) {
        self.conflict = true;
    }

    pub(super) fn clear_conflict(&mut self) {
        self.conflict = false;
    }

    /// The document a save writes, or None while a row does not parse.
    pub(super) fn document(&self, settings: &Settings) -> Option<Settings> {
        self.draft.as_ref()?.document(settings)
    }

    /// Applies one edit to the draft.
    ///
    /// Save, Revert and Close are commands for the shell, not draft edits, so
    /// they are returned rather than applied here.
    pub(super) fn apply(&mut self, message: SettingsMessage) -> SettingsCommand {
        match message {
            SettingsMessage::Save => return SettingsCommand::Save,
            SettingsMessage::Revert => return SettingsCommand::Revert,
            SettingsMessage::Close => {
                self.close();
                return SettingsCommand::Close;
            }
            message => {
                let Some(draft) = self.draft.as_mut() else {
                    return SettingsCommand::None;
                };
                apply_to_draft(draft, message);
            }
        }
        SettingsCommand::None
    }

    /// Draws the dialog. The caller attaches it to the window's root.
    pub(super) fn view(
        &self,
        settings: &Settings,
        path: Option<&Path>,
        sender: &LocalSender<Message>,
    ) -> ContentDialog {
        let mut children: Vec<View> = Vec::new();
        children.push(heading("File").into());
        children.push(
            TextBlock::new()
                .text(status_rows(settings, path, self.error.as_deref()))
                .text_wrapping(TextWrapping::Wrap)
                .into(),
        );
        if let Some(error) = &self.error {
            children.push(
                TextBlock::new()
                    .text(error)
                    .foreground(Color::rgb(220, 170, 60))
                    .text_wrapping(TextWrapping::Wrap)
                    .into(),
            );
        }

        if !self.warnings.is_empty() {
            children.push(heading("Adjusted on load").into());
            for warning in &self.warnings {
                children.push(TextBlock::new().text(warning).text_wrapping(TextWrapping::Wrap).into());
            }
        }
        if let Some(path) = &self.migrated_from {
            children.push(heading("Migrated from").into());
            children.push(
                TextBlock::new()
                    .text(format!("{} was read and written as this file.", path.display()))
                    .text_wrapping(TextWrapping::Wrap)
                    .into(),
            );
        }

        if let Some(draft) = &self.draft {
            children.push(heading("Defaults").into());
            children.push(
                row((
                    label("Font size"),
                    NumberBox::new()
                        .value(Some(f64::from(draft.font_size)))
                        .minimum(4.0)
                        .maximum(100.0)
                        .small_change(0.5)
                        .width(120.0)
                        .on_value_changed({
                            let sender = sender.clone();
                            move |value: Option<f64>| {
                                if let Some(value) = value {
                                    let _ = sender.send(Message::Settings(SettingsMessage::FontSize(value)));
                                }
                            }
                        }),
                ))
                .into(),
            );

            let profiles: Vec<String> = draft.profiles.iter().map(|profile| profile.name.clone()).collect();
            children.push(
                row((
                    label("Default profile"),
                    ComboBox::new()
                        .items_source(profiles.clone())
                        .selected_index(profile_index(&draft.profiles, draft.default_profile.as_deref()))
                        .width(260.0)
                        .on_selection_changed({
                            let sender = sender.clone();
                            move |index: Option<usize>| {
                                let name = index.and_then(|index| profiles.get(index)).cloned();
                                let _ = sender.send(Message::Settings(SettingsMessage::DefaultProfile(name)));
                            }
                        }),
                ))
                .into(),
            );

            let schemes: Vec<String> = std::iter::once("built-in".to_string())
                .chain(draft.schemes.iter().map(|scheme| scheme.name.clone()))
                .collect();
            children.push(
                row((
                    label("Scheme"),
                    ComboBox::new()
                        .items_source(schemes.clone())
                        .selected_index(scheme_index(&draft.schemes, draft.default_scheme.as_deref()))
                        .width(260.0)
                        .on_selection_changed({
                            let sender = sender.clone();
                            move |index: Option<usize>| {
                                let name = scheme_name(index, &schemes);
                                let _ = sender.send(Message::Settings(SettingsMessage::DefaultScheme(name)));
                            }
                        }),
                ))
                .into(),
            );

            let themes: Vec<String> = std::iter::once("shell".to_string())
                .chain(draft.themes.iter().map(|theme| theme.name.clone()))
                .collect();
            children.push(
                row((
                    label("Theme"),
                    ComboBox::new()
                        .items_source(themes.clone())
                        .selected_index(theme_index(&draft.themes, draft.theme.as_deref()))
                        .width(260.0)
                        .on_selection_changed({
                            let sender = sender.clone();
                            move |index: Option<usize>| {
                                let name = theme_name(index, &themes);
                                let _ = sender.send(Message::Settings(SettingsMessage::Theme(name)));
                            }
                        }),
                ))
                .into(),
            );

            children.push(heading("Profiles").into());
            children.push(note("Arguments are split on spaces; quote an argument in the file.").into());
            for (index, profile) in draft.profiles.iter().enumerate() {
                let fields: Vec<View> = vec![
                    text_box(&profile.name, sender, move |text| SettingsMessage::ProfileField {
                        index,
                        field: ProfileField::Name,
                        text,
                    })
                    .header("Name")
                    .into(),
                    text_box(&profile.command, sender, move |text| SettingsMessage::ProfileField {
                        index,
                        field: ProfileField::Command,
                        text,
                    })
                    .header("Command")
                    .into(),
                    text_box(&profile.args, sender, move |text| SettingsMessage::ProfileField {
                        index,
                        field: ProfileField::Args,
                        text,
                    })
                    .header("Arguments")
                    .into(),
                    text_box(&profile.title, sender, move |text| SettingsMessage::ProfileField {
                        index,
                        field: ProfileField::Title,
                        text,
                    })
                    .header("Title")
                    .into(),
                    action_button("Remove", sender, SettingsMessage::RemoveProfile(index)).into(),
                ];
                children.push(StackPanel::new().spacing(6.0).children(fields).into());
            }
            children.push(action_button("Add profile", sender, SettingsMessage::AddProfile).into());

            children.push(heading("Keybindings").into());
            children.push(note("A chord is [ctrl+][alt+][shift+]<key>.").into());
            for (index, binding) in draft.keybindings.iter().enumerate() {
                let actions: Vec<&str> = settings::Action::ALL.iter().map(|action| action.name()).collect();
                let selected = settings::Action::ALL
                    .iter()
                    .position(|action| *action == binding.action);
                children.push(
                    StackPanel::new()
                        .orientation(Orientation::Horizontal)
                        .spacing(8.0)
                        .children((
                            text_box(&binding.keys, sender, move |text| SettingsMessage::BindingKeys {
                                index,
                                text,
                            })
                            .placeholder_text("ctrl+alt+t")
                            .width(180.0),
                            ComboBox::new()
                                .items_source(actions)
                                .selected_index(selected)
                                .width(180.0)
                                .on_selection_changed({
                                    let sender = sender.clone();
                                    move |selected: Option<usize>| {
                                        if let Some(action) =
                                            selected.and_then(|slot| settings::Action::ALL.get(slot)).copied()
                                        {
                                            let _ = sender.send(Message::Settings(SettingsMessage::BindingAction {
                                                index,
                                                action,
                                            }));
                                        }
                                    }
                                }),
                            action_button("Remove", sender, SettingsMessage::RemoveBinding(index)),
                        ))
                        .into(),
                );
            }
            children.push(action_button("Add binding", sender, SettingsMessage::AddBinding).into());
            let invalid: Vec<&str> = draft
                .keybindings
                .iter()
                .filter(|binding| binding.to_binding().is_none())
                .map(|binding| binding.keys.as_str())
                .collect();
            if !invalid.is_empty() {
                children.push(
                    TextBlock::new()
                        .text(format!("These chords do not parse: {}", invalid.join(", ")))
                        .foreground(Color::rgb(220, 170, 60))
                        .into(),
                );
            }

            children.push(heading("Schemes").into());
            children
                .push(note("A colour is #RGB, #RRGGBB or rgb:r/g/b. An empty field keeps the built-in colour.").into());
            for (index, scheme) in draft.schemes.iter().enumerate() {
                let mut fields: Vec<View> = Vec::new();
                fields.push(
                    text_box(&scheme.name, sender, move |text| SettingsMessage::SchemeName {
                        index,
                        text,
                    })
                    .header("Name")
                    .into(),
                );
                for (slot, &color) in settings::SchemeColor::ALL.iter().enumerate() {
                    let text = scheme.colours.get(slot).map_or("", String::as_str);
                    fields.push(
                        row((
                            label(color.name()),
                            text_box(text, sender, move |text| SettingsMessage::SchemeColor {
                                index,
                                slot,
                                text,
                            })
                            .width(220.0),
                            swatch(text, "built-in"),
                        ))
                        .into(),
                    );
                }
                fields.push(action_button("Remove scheme", sender, SettingsMessage::RemoveScheme(index)).into());
                children.push(StackPanel::new().spacing(4.0).children(fields).into());
            }
            children.push(action_button("Add scheme", sender, SettingsMessage::AddScheme).into());
            for error in draft.scheme_errors() {
                children.push(TextBlock::new().text(error).foreground(Color::rgb(220, 170, 60)).into());
            }

            children.push(heading("Themes").into());
            children
                .push(note("A theme paints the tab row and its tabs. An empty field keeps the shell's colour.").into());
            for (index, theme) in draft.themes.iter().enumerate() {
                let mut fields: Vec<View> = Vec::new();
                fields.push(
                    text_box(&theme.name, sender, move |text| SettingsMessage::ThemeName {
                        index,
                        text,
                    })
                    .header("Name")
                    .into(),
                );
                let fills = [
                    ("Tab row", ThemeField::TabRow, theme.tab_row_background.as_str()),
                    (
                        "Tab row, unfocused",
                        ThemeField::TabRowUnfocused,
                        theme.tab_row_unfocused.as_str(),
                    ),
                    ("Tab", ThemeField::Tab, theme.tab_background.as_str()),
                    ("Tab, unfocused", ThemeField::TabUnfocused, theme.tab_unfocused.as_str()),
                ];
                for (label_text, field, text) in fills {
                    fields.push(
                        row((
                            label(label_text),
                            text_box(text, sender, move |text| SettingsMessage::ThemeField {
                                index,
                                field,
                                text,
                            })
                            .width(220.0),
                            swatch(text, "shell"),
                        ))
                        .into(),
                    );
                }
                fields.push(action_button("Remove theme", sender, SettingsMessage::RemoveTheme(index)).into());
                children.push(StackPanel::new().spacing(4.0).children(fields).into());
            }
            children.push(action_button("Add theme", sender, SettingsMessage::AddTheme).into());
            for error in draft.theme_errors() {
                children.push(TextBlock::new().text(error).foreground(Color::rgb(220, 170, 60)).into());
            }

            children.push(heading("Preview").into());
            match draft.preview(settings) {
                Some(text) => children.push(
                    TextBlock::new()
                        .text(text)
                        .font_family("Consolas")
                        .is_text_selection_enabled(true)
                        .text_wrapping(TextWrapping::Wrap)
                        .into(),
                ),
                None => children.push(
                    TextBlock::new()
                        .text("The document cannot be written while a value does not parse.")
                        .into(),
                ),
            }

            children.push(heading("Apply").into());
            let valid = draft.document(settings).is_some();
            let dirty = draft.differs_from(settings);
            children.push(
                StackPanel::new()
                    .orientation(Orientation::Horizontal)
                    .spacing(8.0)
                    .children((
                        action_button("Save", sender, SettingsMessage::Save).is_enabled(dirty && valid),
                        action_button("Revert", sender, SettingsMessage::Revert).is_enabled(dirty),
                    ))
                    .into(),
            );
            if self.conflict {
                children.push(
                    TextBlock::new()
                        .text("The file changed on disk. Save overwrites it; Revert loads it.")
                        .foreground(Color::rgb(220, 170, 60))
                        .text_wrapping(TextWrapping::Wrap)
                        .into(),
                );
            }
            if dirty {
                children.push(
                    TextBlock::new()
                        .text("Unsaved changes are discarded when this window closes.")
                        .into(),
                );
            }
        }

        let content = ScrollViewer::new()
            .horizontal_scroll_bar_visibility(ScrollBarVisibility::Disabled)
            .vertical_scroll_bar_visibility(ScrollBarVisibility::Auto)
            .height(480.0)
            .content(StackPanel::new().spacing(10.0).children(children));

        ContentDialog::new()
            .title("Settings")
            .close_button_text("Close")
            .is_open(self.open)
            .content(content)
            .on_closed({
                let sender = sender.clone();
                move |_| {
                    let _ = sender.send(Message::Settings(SettingsMessage::Close));
                }
            })
    }
}

/// Applies a draft edit. Callers have already checked that a draft exists.
fn apply_to_draft(draft: &mut Draft, message: SettingsMessage) {
    match message {
        SettingsMessage::FontSize(size) => draft.font_size = size.clamp(4.0, 100.0) as f32,
        SettingsMessage::DefaultProfile(name) => draft.default_profile = name,
        SettingsMessage::DefaultScheme(name) => draft.default_scheme = name,
        SettingsMessage::Theme(name) => draft.theme = name,
        SettingsMessage::ProfileField { index, field, text } => {
            if let Some(profile) = draft.profiles.get_mut(index) {
                match field {
                    ProfileField::Name => profile.name = text,
                    ProfileField::Command => profile.command = text,
                    ProfileField::Args => profile.args = text,
                    ProfileField::Title => profile.title = text,
                }
            }
        }
        SettingsMessage::AddProfile => {
            let mut profile = ProfileDraft::from_profile(&settings::Profile::default());
            profile.name = format!("profile {}", draft.profiles.len() + 1);
            draft.profiles.push(profile);
        }
        SettingsMessage::RemoveProfile(index) => {
            if index < draft.profiles.len() {
                draft.profiles.remove(index);
            }
        }
        SettingsMessage::BindingKeys { index, text } => {
            if let Some(binding) = draft.keybindings.get_mut(index) {
                binding.keys = text;
            }
        }
        SettingsMessage::BindingAction { index, action } => {
            if let Some(binding) = draft.keybindings.get_mut(index) {
                binding.action = action;
            }
        }
        SettingsMessage::AddBinding => draft.keybindings.push(crate::config::BindingDraft {
            keys: String::new(),
            action: settings::Action::NewTab,
        }),
        SettingsMessage::RemoveBinding(index) => {
            if index < draft.keybindings.len() {
                draft.keybindings.remove(index);
            }
        }
        SettingsMessage::SchemeName { index, text } => {
            if let Some(scheme) = draft.schemes.get_mut(index) {
                scheme.name = text;
            }
        }
        SettingsMessage::SchemeColor { index, slot, text } => {
            if let Some(colour) = draft
                .schemes
                .get_mut(index)
                .and_then(|scheme| scheme.colours.get_mut(slot))
            {
                *colour = text;
            }
        }
        SettingsMessage::AddScheme => {
            let name = format!("scheme {}", draft.schemes.len() + 1);
            draft.schemes.push(SchemeDraft::new(name));
        }
        SettingsMessage::RemoveScheme(index) => {
            if index < draft.schemes.len() {
                draft.schemes.remove(index);
            }
        }
        SettingsMessage::ThemeName { index, text } => {
            if let Some(theme) = draft.themes.get_mut(index) {
                theme.name = text;
            }
        }
        SettingsMessage::ThemeField { index, field, text } => {
            if let Some(theme) = draft.themes.get_mut(index) {
                match field {
                    ThemeField::TabRow => theme.tab_row_background = text,
                    ThemeField::TabRowUnfocused => theme.tab_row_unfocused = text,
                    ThemeField::Tab => theme.tab_background = text,
                    ThemeField::TabUnfocused => theme.tab_unfocused = text,
                }
            }
        }
        SettingsMessage::AddTheme => {
            let name = format!("theme {}", draft.themes.len() + 1);
            draft.themes.push(ThemeDraft::new(name));
        }
        SettingsMessage::RemoveTheme(index) => {
            if index < draft.themes.len() {
                draft.themes.remove(index);
            }
        }
        SettingsMessage::Save | SettingsMessage::Revert | SettingsMessage::Close => {}
    }
}

fn heading(text: &str) -> TextBlock {
    TextBlock::new()
        .text(text)
        .font_weight(FontWeight::BOLD)
        .font_size(16.0)
        .margin(Thickness::new(0.0, 8.0, 0.0, 2.0))
}

fn note(text: &str) -> TextBlock {
    TextBlock::new()
        .text(text)
        .text_wrapping(TextWrapping::Wrap)
        .opacity(0.8)
}

fn label(text: &str) -> TextBlock {
    TextBlock::new().text(text).width(150.0)
}

fn row(children: impl IntoViews) -> StackPanel {
    StackPanel::new()
        .orientation(Orientation::Horizontal)
        .spacing(8.0)
        .children(children)
}

fn text_box(
    value: &str,
    sender: &LocalSender<Message>,
    message: impl Fn(String) -> SettingsMessage + 'static,
) -> TextBox {
    let sender = sender.clone();
    TextBox::new().text(value).on_text_changed(move |text: Rc<str>| {
        let _ = sender.send(Message::Settings(message(text.to_string())));
    })
}

fn action_button(label: &str, sender: &LocalSender<Message>, message: SettingsMessage) -> Button {
    let sender = sender.clone();
    Button::new().content(label).on_click(move || {
        let _ = sender.send(Message::Settings(message.clone()));
    })
}

fn swatch(text: &str, empty: &str) -> TextBlock {
    match settings::Color::parse(text) {
        Ok(color) => {
            let color = color.text_color();
            TextBlock::new()
                .text("██")
                .foreground(Color::rgb(color.get_r(), color.get_g(), color.get_b()))
        }
        Err(_) if text.trim().is_empty() => TextBlock::new().text(empty),
        Err(_) => TextBlock::new().text(""),
    }
}

fn status_rows(settings: &Settings, path: Option<&Path>, error: Option<&str>) -> String {
    let file = path.map_or_else(
        || "(none on this platform)".to_string(),
        |path| path.display().to_string(),
    );
    let profile = settings.profile().map_or_else(
        || "(none)".to_string(),
        |profile| format!("{} ({})", profile.name, profile.command),
    );
    let scheme = settings
        .scheme()
        .map_or_else(|| "(built-in)".to_string(), |scheme| scheme.name.clone());
    let theme = settings
        .theme()
        .map_or_else(|| "(shell)".to_string(), |theme| theme.name.clone());
    format!(
        "File: {file}\nStatus: {}\nProfile: {profile}\nKeybindings: {}\nScheme: {scheme}\nTheme: {theme}\nFont: {} pt",
        if error.is_some() {
            "defaults (see below)"
        } else {
            "loaded"
        },
        settings.keybindings.len(),
        settings.font.size
    )
}

/// The combo index for a profile name, or None for no selection.
fn profile_index(profiles: &[ProfileDraft], name: Option<&str>) -> Option<usize> {
    name.and_then(|name| profiles.iter().position(|profile| profile.name == name))
}

/// The combo index for a scheme: 0 is the built-in palette.
fn scheme_index(schemes: &[SchemeDraft], name: Option<&str>) -> Option<usize> {
    Some(match name {
        None => 0,
        Some(name) => 1 + schemes.iter().position(|scheme| scheme.name == name).unwrap_or(0),
    })
}

/// The scheme a combo index names; 0 is the built-in palette.
fn scheme_name(index: Option<usize>, names: &[String]) -> Option<String> {
    match index {
        Some(0) | None => None,
        Some(index) => names.get(index).cloned(),
    }
}

/// The combo index for a theme: 0 is the shell's colours.
fn theme_index(themes: &[ThemeDraft], name: Option<&str>) -> Option<usize> {
    Some(match name {
        None => 0,
        Some(name) => 1 + themes.iter().position(|theme| theme.name == name).unwrap_or(0),
    })
}

/// The theme a combo index names; 0 is the shell's colours.
fn theme_name(index: Option<usize>, names: &[String]) -> Option<String> {
    match index {
        Some(0) | None => None,
        Some(index) => names.get(index).cloned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(name: &str) -> ProfileDraft {
        ProfileDraft {
            name: name.to_string(),
            command: String::new(),
            args: String::new(),
            title: String::new(),
        }
    }

    fn document() -> Settings {
        settings::parse(
            r#"
                [[profiles]]
                name = "one"
                command = "one.exe"

                [[profiles]]
                name = "two"
                command = "two.exe"

                [font]
                size = 14
            "#,
        )
        .expect("settings")
    }

    fn window(settings: &Settings) -> SettingsWindow {
        let mut window = SettingsWindow {
            open: false,
            draft: None,
            error: None,
            conflict: false,
            warnings: Vec::new(),
            migrated_from: None,
        };
        window.open(settings);
        window
    }

    #[test]
    fn an_edit_reaches_the_draft_and_the_document() {
        let settings = document();
        let mut window = window(&settings);

        assert!(matches!(
            window.apply(SettingsMessage::FontSize(20.0)),
            SettingsCommand::None
        ));
        let document = window.document(&settings).expect("a valid document");
        assert_eq!(document.font.size, 20.0);
    }

    #[test]
    fn a_profile_can_be_edited_added_and_removed() {
        let settings = document();
        let mut window = window(&settings);

        window.apply(SettingsMessage::ProfileField {
            index: 1,
            field: ProfileField::Command,
            text: "zsh".to_string(),
        });
        assert_eq!(
            window.document(&settings).expect("a document").profiles[1].command,
            "zsh"
        );

        window.apply(SettingsMessage::AddProfile);
        assert_eq!(window.draft().expect("a draft").profiles.len(), 3);

        window.apply(SettingsMessage::RemoveProfile(0));
        let document = window.document(&settings).expect("a document");
        assert_eq!(document.profiles.len(), 2);
        assert_eq!(document.profiles[0].name, "two");
    }

    #[test]
    fn a_bad_chord_blocks_the_document() {
        let settings = document();
        let mut window = window(&settings);

        window.apply(SettingsMessage::AddBinding);
        let index = window.draft().expect("a draft").keybindings.len() - 1;
        window.apply(SettingsMessage::BindingKeys {
            index,
            text: "not a chord".to_string(),
        });

        assert!(window.document(&settings).is_none());
    }

    #[test]
    fn save_revert_and_close_are_commands_not_edits() {
        let settings = document();
        let mut window = window(&settings);

        assert!(matches!(window.apply(SettingsMessage::Save), SettingsCommand::Save));
        assert!(matches!(window.apply(SettingsMessage::Revert), SettingsCommand::Revert));
        assert!(window.is_open());

        assert!(matches!(window.apply(SettingsMessage::Close), SettingsCommand::Close));
        assert!(!window.is_open());
        assert!(window.draft().is_none());
    }

    #[test]
    fn a_scheme_combo_counts_the_built_in_palette() {
        let schemes = [
            SchemeDraft::new("night".to_string()),
            SchemeDraft::new("day".to_string()),
        ];

        assert_eq!(scheme_index(&schemes, None), Some(0));
        assert_eq!(scheme_index(&schemes, Some("night")), Some(1));
        assert_eq!(scheme_index(&schemes, Some("day")), Some(2));

        let names = ["built-in".to_string(), "night".to_string(), "day".to_string()];
        assert_eq!(scheme_name(Some(0), &names), None);
        assert_eq!(scheme_name(Some(1), &names), Some("night".to_string()));
        assert_eq!(scheme_name(None, &names), None);
    }

    #[test]
    fn a_theme_combo_counts_the_shell_colours() {
        let themes = [ThemeDraft::new("night".to_string())];
        assert_eq!(theme_index(&themes, None), Some(0));
        assert_eq!(theme_index(&themes, Some("night")), Some(1));

        let names = ["shell".to_string(), "night".to_string()];
        assert_eq!(theme_name(Some(0), &names), None);
        assert_eq!(theme_name(Some(1), &names), Some("night".to_string()));
    }

    #[test]
    fn a_profile_combo_has_no_built_in_row() {
        let profiles = [profile("one"), profile("two")];
        assert_eq!(profile_index(&profiles, None), None);
        assert_eq!(profile_index(&profiles, Some("two")), Some(1));
        assert_eq!(profile_index(&profiles, Some("missing")), None);
    }
}

//! The settings window: AppKit controls over the shared settings draft.
//!
//! The window edits the values this shell consumes at once — the font size and
//! the three defaults — and writes the file through the same `settings::save`
//! path startup reads. Profiles, keybindings, schemes and themes stay in the
//! file for this shell: the draft keeps them unchanged, so a save never loses
//! a row the window does not show (ADR 030's division, applied to macOS).

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSBackingStoreType, NSBezelStyle, NSButton, NSColor, NSFont, NSPopUpButton, NSTextField, NSTextView, NSWindow,
    NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use crate::config::Draft;

use super::shell::Shell;

const WIDTH: f64 = 520.0;
const HEIGHT: f64 = 420.0;
const MARGIN: f64 = 20.0;
const LABEL_WIDTH: f64 = 130.0;
const FIELD_WIDTH: f64 = 300.0;
const ROW: f64 = 30.0;

/// The controls the window owns once it has been built.
struct Built {
    window: Retained<NSWindow>,
    font: Retained<NSTextField>,
    profile: Retained<NSPopUpButton>,
    scheme: Retained<NSPopUpButton>,
    theme: Retained<NSPopUpButton>,
    status: Retained<NSTextView>,
    /// The row titles beside the "none" row, so a selection maps back to a name.
    profile_names: RefCell<Vec<String>>,
    scheme_names: RefCell<Vec<String>>,
    theme_names: RefCell<Vec<String>>,
}

/// The settings window's instance state.
pub struct Ivars {
    shell: Rc<RefCell<Shell>>,
    built: RefCell<Option<Built>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and the class is main
    // thread only, which is where every control it owns must be touched.
    #[unsafe(super(objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "WinterSettingsWindow"]
    #[ivars = Ivars]
    pub struct SettingsWindow;

    impl SettingsWindow {
        #[unsafe(method(save:))]
        fn save(&self, _sender: Option<&AnyObject>) {
            let document = {
                let shell = self.ivars().shell.borrow();
                match self.read_draft(&shell) {
                    Ok(draft) => draft.document(shell.settings()),
                    Err(error) => {
                        self.set_status(&error);
                        None
                    }
                }
            };
            let Some(document) = document else {
                return;
            };
            let result = self.ivars().shell.borrow_mut().save_document(&document);
            match result {
                Ok(()) => {
                    self.sync();
                    self.set_status("Saved.");
                }
                Err(error) => self.set_status(&error),
            }
        }

        #[unsafe(method(revert:))]
        fn revert(&self, _sender: Option<&AnyObject>) {
            self.ivars().shell.borrow_mut().reload_settings();
            self.sync();
            self.set_status("Reverted to the file on disk.");
        }

        #[unsafe(method(closeWindow:))]
        fn close_window(&self, _sender: Option<&AnyObject>) {
            if let Some(built) = &*self.ivars().built.borrow() {
                built.window.orderOut(None);
            }
        }
    }
);

impl SettingsWindow {
    /// Creates the window object; the controls appear on the first open.
    pub(crate) fn new(shell: Rc<RefCell<Shell>>, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            shell,
            built: RefCell::new(None),
        });
        // SAFETY: NSObject's designated initializer, on the main thread.
        unsafe { msg_send![super(this), init] }
    }

    /// Shows the window, building it on the first call.
    pub(crate) fn open(&self, mtm: MainThreadMarker) {
        if self.ivars().built.borrow().is_none() {
            let built = self.build(mtm);
            *self.ivars().built.borrow_mut() = Some(built);
        }
        self.sync();
        if let Some(built) = &*self.ivars().built.borrow() {
            built.window.makeKeyAndOrderFront(None);
        }
    }

    /// Re-reads the shell's settings into the controls.
    pub(crate) fn sync(&self) {
        let shell = self.ivars().shell.borrow();
        let settings = shell.settings();
        let Some(built) = &*self.ivars().built.borrow() else {
            return;
        };

        built
            .font
            .setStringValue(&NSString::from_str(&format!("{}", settings.font.size)));

        *built.profile_names.borrow_mut() = settings.profiles.iter().map(|profile| profile.name.clone()).collect();
        *built.scheme_names.borrow_mut() = settings.schemes.iter().map(|scheme| scheme.name.clone()).collect();
        *built.theme_names.borrow_mut() = settings.themes.iter().map(|theme| theme.name.clone()).collect();

        fill(
            &built.profile,
            &built.profile_names.borrow(),
            settings.default_profile.as_deref(),
        );
        fill(
            &built.scheme,
            &built.scheme_names.borrow(),
            settings.default_scheme.as_deref(),
        );
        fill(&built.theme, &built.theme_names.borrow(), settings.theme.as_deref());

        built.status.setString(&NSString::from_str(&status_text(&shell)));
    }

    fn build(&self, mtm: MainThreadMarker) -> Built {
        // SAFETY: the window and its controls are created on the main thread,
        // which AppKit requires, and this object owns every one of them.
        unsafe {
            let window = NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(WIDTH, HEIGHT)),
                NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
                NSBackingStoreType::Buffered,
                false,
            );
            window.setTitle(&NSString::from_str("Winter Settings"));
            window.setReleasedWhenClosed(false);
            window.setBackgroundColor(Some(&NSColor::windowBackgroundColor()));
            window.center();

            let content = window.contentView().expect("a window has a content view");
            let content_height = content.bounds().size.height;

            let font_label = label("Font size", MARGIN, 20.0, content_height, mtm);
            let font = NSTextField::textFieldWithString(&NSString::from_str("14"), mtm);
            font.setFrame(field_rect(0, content_height));

            let profile_label = label("Default profile", MARGIN, 20.0 + ROW, content_height, mtm);
            let profile =
                NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm), field_rect(1, content_height), false);

            let scheme_label = label("Default scheme", MARGIN, 20.0 + 2.0 * ROW, content_height, mtm);
            let scheme =
                NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm), field_rect(2, content_height), false);

            let theme_label = label("Theme", MARGIN, 20.0 + 3.0 * ROW, content_height, mtm);
            let theme =
                NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm), field_rect(3, content_height), false);

            let status = NSTextView::initWithFrame(
                NSTextView::alloc(mtm),
                NSRect::new(NSPoint::new(MARGIN, 62.0), NSSize::new(WIDTH - 2.0 * MARGIN, 180.0)),
            );
            status.setEditable(false);
            status.setSelectable(true);
            status.setRichText(false);
            status.setDrawsBackground(false);
            status.setFont(Some(&NSFont::systemFontOfSize(11.0)));

            let save = button("Save", WIDTH - MARGIN - 80.0, 20.0, self, sel!(save:), mtm);
            save.setKeyEquivalent(&NSString::from_str("\r"));
            let revert = button("Revert", WIDTH - MARGIN - 170.0, 20.0, self, sel!(revert:), mtm);
            let close = button("Close", WIDTH - MARGIN - 250.0, 20.0, self, sel!(closeWindow:), mtm);

            content.addSubview(&font_label);
            content.addSubview(&font);
            content.addSubview(&profile_label);
            content.addSubview(&profile);
            content.addSubview(&scheme_label);
            content.addSubview(&scheme);
            content.addSubview(&theme_label);
            content.addSubview(&theme);
            content.addSubview(&status);
            content.addSubview(&save);
            content.addSubview(&revert);
            content.addSubview(&close);

            Built {
                window,
                font,
                profile,
                scheme,
                theme,
                status,
                profile_names: RefCell::new(Vec::new()),
                scheme_names: RefCell::new(Vec::new()),
                theme_names: RefCell::new(Vec::new()),
            }
        }
    }

    fn read_draft(&self, shell: &Shell) -> Result<Draft, String> {
        let Some(built) = &*self.ivars().built.borrow() else {
            return Err("the settings window is not built".to_string());
        };

        let mut draft = Draft::from_settings(shell.settings());
        let text = built.font.stringValue().to_string();
        draft.font_size = parse_font_size(&text)?;

        draft.default_profile = selected(&built.profile, &built.profile_names.borrow());
        draft.default_scheme = selected(&built.scheme, &built.scheme_names.borrow());
        draft.theme = selected(&built.theme, &built.theme_names.borrow());
        Ok(draft)
    }

    fn set_status(&self, message: &str) {
        if let Some(built) = &*self.ivars().built.borrow() {
            built.status.setString(&NSString::from_str(message));
        }
    }

    /// Captures the settings window's content, for `WINTER_SCREENSHOT`.
    pub(crate) fn capture(&self, path: &Path) -> bool {
        let Some(built) = &*self.ivars().built.borrow() else {
            return false;
        };
        let Some(view) = built.window.contentView() else {
            return false;
        };
        super::capture_view(&view, path)
    }
}

fn label(text: &str, x: f64, top: f64, height: f64, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    label.setFrame(NSRect::new(
        NSPoint::new(x, height - top - 20.0),
        NSSize::new(LABEL_WIDTH, 20.0),
    ));
    label
}

fn field_rect(row: usize, height: f64) -> NSRect {
    let top = 20.0 + ROW * row as f64;
    NSRect::new(
        NSPoint::new(MARGIN + LABEL_WIDTH, height - top - 24.0),
        NSSize::new(FIELD_WIDTH, 24.0),
    )
}

fn button(
    title: &str,
    x: f64,
    y: f64,
    target: &SettingsWindow,
    action: objc2::runtime::Sel,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    // SAFETY: the button and its target live on the main thread, and the
    // selector is one this class implements.
    let button = unsafe {
        let button = NSButton::initWithFrame(
            NSButton::alloc(mtm),
            NSRect::new(NSPoint::new(x, y), NSSize::new(80.0, 28.0)),
        );
        button.setTarget(Some(target.as_ref()));
        button.setAction(Some(action));
        button
    };
    button.setTitle(&NSString::from_str(title));
    button.setBezelStyle(NSBezelStyle::Push);
    button
}

fn fill(popup: &NSPopUpButton, names: &[String], selected_name: Option<&str>) {
    popup.removeAllItems();
    popup.addItemWithTitle(&NSString::from_str("—"));
    for name in names {
        popup.addItemWithTitle(&NSString::from_str(name));
    }
    let index = selected_name
        .and_then(|name| names.iter().position(|candidate| candidate == name))
        .map_or(0, |index| index + 1);
    popup.selectItemAtIndex(index as isize);
}

fn selected(popup: &NSPopUpButton, names: &[String]) -> Option<String> {
    let index = popup.indexOfSelectedItem();
    if index <= 0 {
        return None;
    }
    names.get(index as usize - 1).cloned()
}

fn status_text(shell: &Shell) -> String {
    let mut text = String::new();
    if let Some(path) = shell.settings_path() {
        text.push_str(&format!("File: {}\n", path.display()));
    } else {
        text.push_str("File: the platform has no settings path; the defaults are in force.\n");
    }
    if let Some(path) = shell.migrated_from() {
        text.push_str(&format!("Migrated from: {}\n", path.display()));
    }
    if let Some(error) = shell.settings_error() {
        text.push_str(&format!("\nThe file was not used: {error}\n"));
    }
    for warning in shell.warnings() {
        text.push_str(&format!("Adjusted: {warning}\n"));
    }
    if text.is_empty() {
        text.push_str("Every value in the file was usable.");
    }
    text
}

/// A font size the settings window accepts.
///
/// The field is text until Save; a size outside the range a cell grid can use
/// keeps the window open and names the value, rather than writing it and
/// failing on the next start.
fn parse_font_size(text: &str) -> Result<f32, String> {
    let size = text
        .trim()
        .parse::<f32>()
        .map_err(|_| format!("`{text}` is not a font size"))?;
    if !size.is_finite() || !(4.0..=96.0).contains(&size) {
        return Err(format!("`{text}` is outside 4–96 points"));
    }
    Ok(size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_font_size_is_a_number_in_range() {
        assert_eq!(parse_font_size(" 14 "), Ok(14.0));
        assert_eq!(parse_font_size("4"), Ok(4.0));
        assert_eq!(parse_font_size("96"), Ok(96.0));
        assert!(parse_font_size("big").is_err());
        assert!(parse_font_size("3.9").is_err());
        assert!(parse_font_size("97").is_err());
        assert!(parse_font_size("NaN").is_err());
    }
}

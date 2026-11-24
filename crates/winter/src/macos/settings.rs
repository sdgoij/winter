//! The settings window: AppKit controls over the whole settings document.
//!
//! The window is an editable view of the TOML file the shell reads: Font,
//! profiles, keybindings, schemes, themes and the defaults are all there, so
//! none of them stays file-only on macOS. Save parses the text with the same
//! reader startup uses, writes it through `settings::save`, and reloads, so
//! what runs is always the normalized file (ADR 030's rule; ADR 072 records
//! why this shell edits the document rather than WinUI's rows).

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSBackingStoreType, NSBezelStyle, NSBorderType, NSButton, NSColor, NSFont, NSScrollView, NSTextField, NSTextView,
    NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use super::shell::Shell;

const WIDTH: f64 = 640.0;
const HEIGHT: f64 = 540.0;
const MARGIN: f64 = 20.0;
/// The editor sits between this (from the bottom) and the hint at the top.
const EDITOR_BOTTOM: f64 = 88.0;
/// The hint and the editor leave this much at the top.
const TOP_ROOM: f64 = 36.0;

/// The controls the window owns once it has been built.
struct Built {
    window: Retained<NSWindow>,
    document: Retained<NSTextView>,
    status: Retained<NSTextView>,
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
            let document = match self.read_document() {
                Ok(document) => document,
                Err(error) => {
                    self.set_status(&error);
                    return;
                }
            };
            match self.ivars().shell.borrow_mut().save_document(&document) {
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

    /// Re-reads the shell's settings into the editor.
    pub(crate) fn sync(&self) {
        let shell = self.ivars().shell.borrow();
        let Some(built) = &*self.ivars().built.borrow() else {
            return;
        };
        built
            .document
            .setString(&NSString::from_str(&settings::to_toml(shell.settings())));
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

            let hint = label(
                "Settings file — Save validates the TOML and writes it.",
                MARGIN,
                content_height - 26.0,
                WIDTH - 2.0 * MARGIN,
                mtm,
            );

            let scroll = NSScrollView::initWithFrame(
                NSScrollView::alloc(mtm),
                NSRect::new(
                    NSPoint::new(MARGIN, EDITOR_BOTTOM),
                    NSSize::new(WIDTH - 2.0 * MARGIN, content_height - EDITOR_BOTTOM - TOP_ROOM),
                ),
            );
            scroll.setHasVerticalScroller(true);
            scroll.setBorderType(NSBorderType::BezelBorder);

            let document = NSTextView::initWithFrame(NSTextView::alloc(mtm), scroll.contentView().bounds());
            document.setMinSize(NSSize::new(0.0, 0.0));
            document.setMaxSize(NSSize::new(f64::MAX, f64::MAX));
            document.setVerticallyResizable(true);
            document.setHorizontallyResizable(false);
            document.setAutoresizingMask(objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable);
            document.setEditable(true);
            document.setSelectable(true);
            document.setRichText(false);
            document.setAllowsUndo(true);
            // TOML is punctuation: smart quotes, dashes and autocorrection would
            // rewrite the keys the file needs to stay valid.
            document.setAutomaticQuoteSubstitutionEnabled(false);
            document.setAutomaticDashSubstitutionEnabled(false);
            document.setAutomaticTextReplacementEnabled(false);
            document.setAutomaticSpellingCorrectionEnabled(false);
            if let Some(font) = NSFont::userFixedPitchFontOfSize(12.0) {
                document.setFont(Some(&font));
            }
            // SAFETY: the text view owns its container for as long as it lives.
            if let Some(container) = document.textContainer() {
                container.setContainerSize(NSSize::new(scroll.contentSize().width, f64::MAX));
                container.setWidthTracksTextView(true);
            }
            scroll.setDocumentView(Some(&document));

            let status = NSTextView::initWithFrame(
                NSTextView::alloc(mtm),
                NSRect::new(NSPoint::new(MARGIN, 48.0), NSSize::new(WIDTH - 2.0 * MARGIN, 34.0)),
            );
            status.setEditable(false);
            status.setSelectable(true);
            status.setRichText(false);
            status.setDrawsBackground(false);
            status.setFont(Some(&NSFont::systemFontOfSize(11.0)));

            let save = button("Save", WIDTH - MARGIN - 90.0, 10.0, self, sel!(save:), mtm);
            save.setKeyEquivalent(&NSString::from_str("\r"));
            let revert = button("Revert", WIDTH - MARGIN - 190.0, 10.0, self, sel!(revert:), mtm);
            let close = button("Close", WIDTH - MARGIN - 280.0, 10.0, self, sel!(closeWindow:), mtm);

            content.addSubview(&hint);
            content.addSubview(&scroll);
            content.addSubview(&status);
            content.addSubview(&save);
            content.addSubview(&revert);
            content.addSubview(&close);

            Built {
                window,
                document,
                status,
            }
        }
    }

    fn read_document(&self) -> Result<settings::Settings, String> {
        let Some(built) = &*self.ivars().built.borrow() else {
            return Err("the settings window is not built".to_string());
        };
        parse_document(&built.document.string().to_string())
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

/// Parses the text the editor holds, with the window's error shape.
fn parse_document(text: &str) -> Result<settings::Settings, String> {
    settings::parse(text).map_err(|error| format!("The document is not valid: {error}"))
}

fn label(text: &str, x: f64, y: f64, width: f64, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    label.setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(width, 20.0)));
    label
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
            NSRect::new(NSPoint::new(x, y), NSSize::new(90.0, 28.0)),
        );
        button.setTarget(Some(target.as_ref()));
        button.setAction(Some(action));
        button
    };
    button.setTitle(&NSString::from_str(title));
    button.setBezelStyle(NSBezelStyle::Push);
    button
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_editor_reads_the_sections_the_partial_window_could_not() {
        let document = r##"
version = 1
default_profile = "shell"
default_scheme = "Mine"
theme = "Mine"

[font]
size = 18.0

[[profiles]]
name = "shell"
command = "/bin/zsh"
args = ["-l"]

[[keybindings]]
keys = "ctrl+shift+t"
action = "newTab"

[[schemes]]
name = "Mine"
foreground = "#ffffff"
background = "#000000"

[[themes]]
name = "Mine"
"##;

        let parsed = parse_document(document).expect("a valid document");
        assert_eq!(parsed.font.size, 18.0);
        assert_eq!(parsed.profiles[0].name, "shell");
        assert_eq!(parsed.profiles[0].args, ["-l"]);
        assert!(
            parsed
                .keybindings
                .iter()
                .any(|binding| binding.keys.to_string() == "ctrl+shift+t" && binding.action == settings::Action::NewTab)
        );
        assert_eq!(parsed.schemes[0].name, "Mine");
        assert_eq!(parsed.themes[0].name, "Mine");
        assert_eq!(parsed.default_profile.as_deref(), Some("shell"));
    }

    #[test]
    fn an_invalid_document_names_the_problem() {
        let error = parse_document("font = ").expect_err("a broken document");
        assert!(error.contains("not valid"), "{error}");
    }

    #[test]
    fn a_saved_document_reads_back_as_the_same_settings() {
        let original = settings::Settings::default();
        let text = settings::to_toml(&original);
        assert_eq!(parse_document(&text).expect("the writer's own text"), original);
    }
}

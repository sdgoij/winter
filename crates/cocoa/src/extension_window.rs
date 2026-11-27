//! The extensions window: what Winter found, and a switch per extension.
//!
//! The rows come from discovery rather than from the engine, so the window
//! lists an extension that is switched off, one that failed to load, and one
//! whose manifest is broken — the three cases a user has to be able to see in
//! order to fix them. Saving writes the settings file's `[extensions]` table
//! and nothing else, and says that loading changes on the next start: a live
//! realm holds registrations, so switching one off is not a reload.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSBackingStoreType, NSBezelStyle, NSBorderType, NSButton, NSButtonType, NSColor, NSControlStateValueOff,
    NSControlStateValueOn, NSFont, NSScrollView, NSTextField, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use super::extension::Found;
use super::shell::Shell;

const WIDTH: f64 = 660.0;
const HEIGHT: f64 = 460.0;
const MARGIN: f64 = 20.0;
/// One row, tall enough for a name and a line of detail.
const ROW: f64 = 44.0;
/// The hint and the master switch leave this much at the top.
const TOP_ROOM: f64 = 112.0;
/// The status line and the buttons leave this much at the bottom.
const BOTTOM: f64 = 88.0;

/// The controls the window owns once it has been built.
struct Built {
    window: Retained<NSWindow>,
    master: Retained<NSButton>,
    scroll: Retained<NSScrollView>,
    status: Retained<NSTextField>,
    rows: RefCell<Vec<RowView>>,
}

/// One row's switch, and the identifier it belongs to.
struct RowView {
    id: String,
    switchable: bool,
    switch: Retained<NSButton>,
}

/// The window's instance state.
pub struct Ivars {
    shell: Rc<RefCell<Shell>>,
    built: RefCell<Option<Built>>,
}

/// The rows' document view, flipped so the first row is the top one.
#[derive(Debug)]
struct DocumentIvars;

define_class!(
    // SAFETY: NSView has no subclassing requirements this view does not meet,
    // and the class is main-thread only, like AppKit's own views.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "WinterExtensionsDocument"]
    #[ivars = DocumentIvars]
    struct DocumentView;

    impl DocumentView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

impl DocumentView {
    /// A flipped document view for the rows.
    fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DocumentIvars);
        // SAFETY: NSView's designated initializer, on the main thread with an
        // allocated instance of this subclass.
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and the class is main
    // thread only, which is where every control it owns must be touched.
    #[unsafe(super(objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "WinterExtensionsWindow"]
    #[ivars = Ivars]
    pub struct ExtensionsWindow;

    impl ExtensionsWindow {
        #[unsafe(method(save:))]
        fn save(&self, _sender: Option<&AnyObject>) {
            let (enabled, disabled) = {
                let Some(built) = &*self.ivars().built.borrow() else {
                    return;
                };
                let enabled = built.master.state() == NSControlStateValueOn;
                let disabled: Vec<String> = built
                    .rows
                    .borrow()
                    .iter()
                    .filter(|row| row.switchable && row.switch.state() != NSControlStateValueOn)
                    .map(|row| row.id.clone())
                    .collect();
                (enabled, disabled)
            };
            match self.ivars().shell.borrow_mut().save_extensions(enabled, disabled) {
                Ok(()) => {
                    self.sync();
                    self.set_status("Saved. Extensions load when Winter next starts.");
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

impl ExtensionsWindow {
    /// Creates the window object; the controls appear on the first open.
    pub(crate) fn new(shell: Rc<RefCell<Shell>>, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            shell,
            built: RefCell::new(None),
        });
        // SAFETY: NSObject's designated initializer, on the main thread.
        unsafe { msg_send![super(this), init] }
    }

    /// Shows the window, building it on the first call and refreshing the rows.
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

    /// Re-reads the shell's settings and the disk into the window.
    pub(crate) fn sync(&self) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let (enabled, rows) = {
            let shell = self.ivars().shell.borrow();
            (shell.settings().extensions.enabled, shell.installed_extensions())
        };
        let Some(built) = &*self.ivars().built.borrow() else {
            return;
        };

        built.master.setState(if enabled {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });
        built.status.setStringValue(&NSString::from_str(&summary(&rows)));

        // The rows are rebuilt rather than updated: there is no stack view here,
        // and a rebuild is what a settings change would need anyway. The
        // document is flipped, so row zero is the top one, and it is at least
        // as tall as the clip view so a short list starts at the top.
        let width = built.scroll.contentSize().width;
        let height = (rows.len() as f64 * ROW).max(built.scroll.contentSize().height);
        let document = DocumentView::new(mtm, NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, height)));
        let mut views = Vec::with_capacity(rows.len());
        for (index, row) in rows.iter().enumerate() {
            let y = index as f64 * ROW;
            // SAFETY: the control is created on the main thread.
            let switch = NSButton::initWithFrame(
                NSButton::alloc(mtm),
                NSRect::new(NSPoint::new(MARGIN + 4.0, y + 13.0), NSSize::new(20.0, 20.0)),
            );
            switch.setButtonType(NSButtonType::Switch);
            switch.setTitle(&NSString::from_str(""));
            switch.setState(if row.enabled {
                NSControlStateValueOn
            } else {
                NSControlStateValueOff
            });
            switch.setEnabled(row.switchable);
            switch.setToolTip(Some(&NSString::from_str(if row.switchable {
                "Load this extension when Winter starts"
            } else {
                "This extension could not be read, so there is nothing to switch"
            })));

            let (title, detail) = lines(row);
            let title = NSTextField::labelWithString(&NSString::from_str(&title), mtm);
            title.setFrame(NSRect::new(
                NSPoint::new(MARGIN + 34.0, y + 4.0),
                NSSize::new(width - 2.0 * MARGIN - 34.0, 17.0),
            ));
            let detail = NSTextField::labelWithString(&NSString::from_str(&detail), mtm);
            detail.setFrame(NSRect::new(
                NSPoint::new(MARGIN + 34.0, y + 23.0),
                NSSize::new(width - 2.0 * MARGIN - 34.0, 15.0),
            ));
            detail.setFont(Some(&NSFont::systemFontOfSize(11.0)));
            if row.note.is_empty() {
                detail.setTextColor(Some(&NSColor::secondaryLabelColor()));
                detail.setToolTip(Some(&detail.stringValue()));
            } else {
                detail.setTextColor(Some(&NSColor::systemRedColor()));
                detail.setToolTip(Some(&NSString::from_str(&row.note)));
            }

            document.addSubview(&switch);
            document.addSubview(&title);
            document.addSubview(&detail);
            views.push(RowView {
                id: row.id.clone(),
                switchable: row.switchable,
                switch,
            });
        }
        built.scroll.setDocumentView(Some(&document));
        *built.rows.borrow_mut() = views;
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
            window.setTitle(&NSString::from_str("Extensions"));
            window.setReleasedWhenClosed(false);
            window.setBackgroundColor(Some(&NSColor::windowBackgroundColor()));
            window.center();

            let content = window.contentView().expect("a window has a content view");
            let content_height = content.bounds().size.height;

            let hint = NSTextField::labelWithString(
                &NSString::from_str(
                    "Anything under an `extensions/` folder is listed here. A switch takes effect the next time Winter starts.",
                ),
                mtm,
            );
            hint.setFrame(NSRect::new(
                NSPoint::new(MARGIN, content_height - 26.0),
                NSSize::new(WIDTH - 2.0 * MARGIN, 20.0),
            ));

            let master = NSButton::initWithFrame(
                NSButton::alloc(mtm),
                NSRect::new(NSPoint::new(MARGIN, content_height - 58.0), NSSize::new(280.0, 20.0)),
            );
            master.setButtonType(NSButtonType::Switch);
            master.setTitle(&NSString::from_str("Load extensions"));

            let scroll = NSScrollView::initWithFrame(
                NSScrollView::alloc(mtm),
                NSRect::new(
                    NSPoint::new(MARGIN, BOTTOM),
                    NSSize::new(WIDTH - 2.0 * MARGIN, content_height - BOTTOM - TOP_ROOM),
                ),
            );
            scroll.setHasVerticalScroller(true);
            scroll.setBorderType(NSBorderType::BezelBorder);

            let status = NSTextField::labelWithString(&NSString::from_str(""), mtm);
            status.setFrame(NSRect::new(
                NSPoint::new(MARGIN, 46.0),
                NSSize::new(WIDTH - 2.0 * MARGIN, 30.0),
            ));
            status.setFont(Some(&NSFont::systemFontOfSize(11.0)));
            status.setTextColor(Some(&NSColor::secondaryLabelColor()));

            let save = button("Save", WIDTH - MARGIN - 90.0, 10.0, self, sel!(save:), mtm);
            save.setKeyEquivalent(&NSString::from_str("\r"));
            let revert = button("Revert", WIDTH - MARGIN - 190.0, 10.0, self, sel!(revert:), mtm);
            let close = button("Close", WIDTH - MARGIN - 280.0, 10.0, self, sel!(closeWindow:), mtm);

            content.addSubview(&hint);
            content.addSubview(&master);
            content.addSubview(&scroll);
            content.addSubview(&status);
            content.addSubview(&save);
            content.addSubview(&revert);
            content.addSubview(&close);

            Built {
                window,
                master,
                scroll,
                status,
                rows: RefCell::new(Vec::new()),
            }
        }
    }

    fn set_status(&self, message: &str) {
        if let Some(built) = &*self.ivars().built.borrow() {
            built.status.setStringValue(&NSString::from_str(message));
        }
    }

    /// Captures the window's content, for `WINTER_SCREENSHOT`.
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

/// A row's headline and its detail line.
fn lines(row: &Found) -> (String, String) {
    let title = format!("{} — {}", row.name, row.id);
    let mut detail = row.kind.to_string();
    if !row.note.is_empty() {
        detail.push_str(" · ");
        detail.push_str(&row.note);
    }
    (title, detail)
}

/// What the status line says about the list as a whole.
fn summary(rows: &[Found]) -> String {
    if rows.is_empty() {
        return "No extensions found. Put one under `extensions/` in the settings folder, beside the executable, or in the working directory.".to_string();
    }
    let broken = rows.iter().filter(|row| !row.switchable).count();
    let enabled = rows.iter().filter(|row| row.enabled).count();
    if broken == 0 {
        format!("{} extension(s), {enabled} enabled.", rows.len())
    } else {
        format!("{} extension(s), {enabled} enabled, {broken} unreadable.", rows.len())
    }
}

fn button(
    title: &str,
    x: f64,
    y: f64,
    target: &ExtensionsWindow,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn found(name: &str, id: &str, kind: &'static str, enabled: bool, switchable: bool, note: &str) -> Found {
        Found {
            id: id.to_string(),
            name: name.to_string(),
            kind,
            switchable,
            enabled,
            note: note.to_string(),
        }
    }

    #[test]
    fn a_row_names_the_extension_and_what_it_ships() {
        let (title, detail) = lines(&found("Fixture", "test.a", "script", true, true, ""));
        assert_eq!(title, "Fixture — test.a");
        assert_eq!(detail, "script");
    }

    #[test]
    fn why_a_row_is_not_running_is_on_its_detail_line() {
        let (_, detail) = lines(&found(
            "Fixture",
            "test.a",
            "compiled",
            true,
            true,
            "the module imports `winter.now`, which this host does not grant",
        ));
        assert!(detail.starts_with("compiled · "), "{detail}");
        assert!(detail.contains("winter.now"), "{detail}");
    }

    #[test]
    fn the_summary_counts_unreadable_extensions_apart() {
        let rows = [
            found("A", "a.one", "script", true, true, ""),
            found("B", "b.two", "script", false, true, ""),
            found("C", "c.three", "broken", false, false, "no manifest"),
        ];
        let text = summary(&rows);
        assert!(text.contains("3 extension(s)"), "{text}");
        assert!(text.contains("1 enabled"), "{text}");
        assert!(text.contains("1 unreadable"), "{text}");
    }

    #[test]
    fn an_empty_list_says_where_to_put_one() {
        let text = summary(&[]);
        assert!(text.contains("No extensions found"), "{text}");
        assert!(text.contains("extensions/"), "{text}");
    }
}

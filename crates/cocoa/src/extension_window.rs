//! The extensions window: the Installed switches, and the registry Search tab.
//!
//! The Installed rows come from discovery rather than from the engine, so the
//! window lists an extension that is switched off, one that failed to load, and
//! one whose manifest is broken — the three cases a user has to be able to see in
//! order to fix them. Saving writes the settings file's `[extensions]` table and
//! rebuilds the engine at once.
//!
//! The Search tab is the only place a registry is read, and only once the tab is
//! first shown (ADR 088): opening the window on Installed fetches nothing. The
//! shell polls the screen from its tick, and a finished install or uninstall
//! rebuilds the engine and both lists without a restart.

use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSBackingStoreType, NSBezelStyle, NSBorderType, NSButton, NSButtonType, NSColor, NSControlStateValueOff,
    NSControlStateValueOn, NSFont, NSPopUpButton, NSScrollView, NSSearchField, NSTextField, NSTextView, NSView,
    NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize, NSString};

use super::extension::{Found, registry_screen};
use super::shell::Shell;

const WIDTH: f64 = 700.0;
const HEIGHT: f64 = 560.0;
const MARGIN: f64 = 20.0;
/// One Installed row, tall enough for a name and a line of detail.
const ROW: f64 = 44.0;
/// An Installed row that names the registry it came from, with a Remove button.
const ROW_WITH_SOURCE: f64 = 96.0;
/// The status line and the buttons leave this much at the bottom.
const BOTTOM: f64 = 96.0;
/// The gaps from the content's top edge to each control's top.
const TAB_GAP: f64 = 20.0;
const HINT_GAP: f64 = 56.0;
const MASTER_GAP: f64 = 82.0;
const INSTALLED_SCROLL_GAP: f64 = 112.0;
const SEARCH_FIELD_GAP: f64 = 60.0;
const SEARCH_SCROLL_GAP: f64 = 96.0;

/// Which tab the window shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    /// What is on disk, with the switches.
    Installed,
    /// Rows from the configured registries; the only tab that fetches.
    Search,
}

/// The controls the window owns once it has been built.
struct Built {
    window: Retained<NSWindow>,
    hint: Retained<NSTextField>,
    master: Retained<NSButton>,
    search_field: Retained<NSSearchField>,
    scroll: Retained<NSScrollView>,
    status: Retained<NSTextView>,
    installed_tab: Retained<NSButton>,
    search_tab: Retained<NSButton>,
    tab: Cell<Tab>,
    rows: RefCell<Vec<RowView>>,
    /// The identifiers the visible Remove buttons remove, by button tag.
    remove_ids: RefCell<Vec<String>>,
}

/// One Installed row's switch, and the identifier it belongs to.
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
            // The borrow must end before `sync` reads the shell again: a `match`
            // on the `borrow_mut()` temporary would keep it alive through the arm.
            let result = {
                let mut shell = self.ivars().shell.borrow_mut();
                shell.save_extensions(enabled, disabled)
            };
            match result {
                Ok(()) => {
                    self.sync();
                    self.set_status("Saved. The engine was rebuilt.");
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
                built.tab.set(Tab::Installed);
                built.search_field.setStringValue(&NSString::from_str(""));
            }
            // Nothing polls the Search screen while the window is out, and the
            // next open starts from the registries again.
            self.ivars().shell.borrow_mut().registry = None;
        }

        #[unsafe(method(tabInstalled:))]
        fn tab_installed(&self, _sender: Option<&AnyObject>) {
            self.switch_to(Tab::Installed);
        }

        #[unsafe(method(tabSearch:))]
        fn tab_search(&self, _sender: Option<&AnyObject>) {
            self.switch_to(Tab::Search);
        }

        #[unsafe(method(searchChanged:))]
        fn search_changed(&self, sender: Option<&AnyObject>) {
            let text = sender
                .and_then(|sender| sender.downcast_ref::<NSSearchField>())
                .map(|field| field.stringValue().to_string())
                .unwrap_or_default();
            if let Some(screen) = &mut self.ivars().shell.borrow_mut().registry {
                screen.set_query(text);
            }
            self.sync();
        }

        #[unsafe(method(versionPicked:))]
        fn version_picked(&self, sender: Option<&AnyObject>) {
            let Some(popup) = sender.and_then(|sender| sender.downcast_ref::<NSPopUpButton>()) else {
                return;
            };
            let (row, slot) = (popup.tag(), popup.indexOfSelectedItem());
            if row < 0 || slot < 0 {
                return;
            }
            if let Some(screen) = &mut self.ivars().shell.borrow_mut().registry {
                screen.select_index(row as usize, slot as usize);
            }
        }

        #[unsafe(method(installRow:))]
        fn install_row(&self, sender: Option<&AnyObject>) {
            let Some(button) = sender.and_then(|sender| sender.downcast_ref::<NSButton>()) else {
                return;
            };
            let row = button.tag();
            if row < 0 {
                return;
            }
            if let Some(screen) = &mut self.ivars().shell.borrow_mut().registry {
                screen.install_index(row as usize);
            }
            self.sync();
        }

        #[unsafe(method(removeRow:))]
        fn remove_row(&self, sender: Option<&AnyObject>) {
            let Some(button) = sender.and_then(|sender| sender.downcast_ref::<NSButton>()) else {
                return;
            };
            let id = {
                let built = self.ivars().built.borrow();
                let Some(built) = built.as_ref() else {
                    return;
                };
                let slot = button.tag();
                if slot < 0 {
                    return;
                }
                built.remove_ids.borrow().get(slot as usize).cloned()
            };
            let Some(id) = id else {
                return;
            };
            if let Some(screen) = &mut self.ivars().shell.borrow_mut().registry {
                screen.uninstall(&id, false);
            }
            self.sync();
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
        // The Search screen exists while the window does; opening it on the
        // Installed tab still reads no network (ADR 088).
        {
            let mut shell = self.ivars().shell.borrow_mut();
            if shell.registry.is_none() {
                let settings = shell.settings().extensions.clone();
                shell.registry = Some(registry_screen(&settings));
            }
        }
        self.sync();
        if let Some(built) = &*self.ivars().built.borrow() {
            built.window.makeKeyAndOrderFront(None);
        }
    }

    /// Switches tabs, opening the registry fetch when Search is first shown.
    fn switch_to(&self, tab: Tab) {
        if tab == Tab::Search {
            let mut shell = self.ivars().shell.borrow_mut();
            if let Some(screen) = &mut shell.registry {
                screen.open();
            }
        }
        if let Some(built) = &*self.ivars().built.borrow() {
            built.tab.set(tab);
        }
        self.sync();
    }

    /// Shows the Search tab directly, for `WINTER_OPEN_EXTENSIONS=search`.
    pub(crate) fn show_search(&self) {
        self.switch_to(Tab::Search);
    }

    /// Re-reads the shell's settings, the disk and the Search screen into the
    /// window.
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

        let tab = built.tab.get();
        let installed = tab == Tab::Installed;
        built.installed_tab.setTitle(&NSString::from_str(if installed {
            "• Installed"
        } else {
            "Installed"
        }));
        built
            .search_tab
            .setTitle(&NSString::from_str(if installed { "Search" } else { "• Search" }));
        built.hint.setHidden(!installed);
        built.master.setHidden(!installed);
        built.search_field.setHidden(installed);
        built.master.setState(if enabled {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });

        let content_height = built
            .window
            .contentView()
            .map_or(HEIGHT, |view| view.bounds().size.height);
        let top = content_height
            - if installed {
                INSTALLED_SCROLL_GAP
            } else {
                SEARCH_SCROLL_GAP
            };
        built.scroll.setFrame(NSRect::new(
            NSPoint::new(MARGIN, BOTTOM),
            NSSize::new(WIDTH - 2.0 * MARGIN, (top - BOTTOM).max(0.0)),
        ));

        match tab {
            Tab::Installed => {
                built.status.setString(&NSString::from_str(&summary(&rows)));
                self.rebuild_installed(built, &rows, mtm);
            }
            Tab::Search => self.rebuild_search(built, mtm),
        }
    }

    /// The Installed tab: one switch per extension and one that loads the
    /// feature at all, plus a record and a Remove button for a registry install.
    fn rebuild_installed(&self, built: &Built, rows: &[Found], mtm: MainThreadMarker) {
        let width = built.scroll.contentSize().width;
        let heights: Vec<f64> = rows
            .iter()
            .map(|row| if row.source.is_some() { ROW_WITH_SOURCE } else { ROW })
            .collect();
        let height = heights.iter().sum::<f64>().max(built.scroll.contentSize().height);
        let document = DocumentView::new(mtm, NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, height)));
        let mut views = Vec::with_capacity(rows.len());
        let mut remove_ids = Vec::new();
        let mut y = 0.0;
        for (index, row) in rows.iter().enumerate() {
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
                "Load this extension"
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

            if let Some(source) = &row.source {
                let provenance = NSTextField::labelWithString(&NSString::from_str(&source_text(source)), mtm);
                provenance.setFrame(NSRect::new(
                    NSPoint::new(MARGIN + 34.0, y + 42.0),
                    NSSize::new(width - 2.0 * MARGIN - 34.0, 15.0),
                ));
                provenance.setFont(Some(&NSFont::systemFontOfSize(11.0)));
                provenance.setTextColor(Some(&NSColor::secondaryLabelColor()));
                provenance.setToolTip(Some(&provenance.stringValue()));

                let remove = button("Remove", MARGIN + 34.0, y + 60.0, self, sel!(removeRow:), mtm);
                remove.setTag(remove_ids.len() as isize);
                remove_ids.push(row.id.clone());
                document.addSubview(&provenance);
                document.addSubview(&remove);
            }

            views.push(RowView {
                id: row.id.clone(),
                switchable: row.switchable,
                switch,
            });
            y += heights.get(index).copied().unwrap_or(ROW);
        }
        built.scroll.setDocumentView(Some(&document));
        *built.rows.borrow_mut() = views;
        *built.remove_ids.borrow_mut() = remove_ids;
    }

    /// The Search tab: the filter, what each registry answered, and one block per
    /// row with its releases and an install button.
    fn rebuild_search(&self, built: &Built, mtm: MainThreadMarker) {
        let (rows, status, opened, loading) = {
            let shell = self.ivars().shell.borrow();
            let Some(screen) = &shell.registry else {
                built.status.setString(&NSString::from_str(""));
                return;
            };
            let rows: Vec<(registry::SearchRow, Option<usize>)> = screen
                .rows()
                .into_iter()
                .map(|row| {
                    let picked = screen.version(&row).cloned();
                    let slot = picked.and_then(|picked| row.versions.iter().position(|version| *version == picked));
                    (row, slot)
                })
                .collect();
            (rows, status_for(screen), screen.opened(), screen.is_loading())
        };
        built.status.setString(&NSString::from_str(&status));

        let width = built.scroll.contentSize().width;
        let heights: Vec<f64> = rows.iter().map(|(row, _)| search_height(row)).collect();
        let document_height = heights.iter().sum::<f64>().max(built.scroll.contentSize().height);
        let document = DocumentView::new(
            mtm,
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, document_height)),
        );

        if rows.is_empty() && opened && !loading {
            let empty = NSTextField::labelWithString(&NSString::from_str("No extension matches."), mtm);
            empty.setFrame(NSRect::new(
                NSPoint::new(MARGIN, 4.0),
                NSSize::new(width - 2.0 * MARGIN, 17.0),
            ));
            empty.setTextColor(Some(&NSColor::secondaryLabelColor()));
            document.addSubview(&empty);
        }

        let line_width = width - 2.0 * MARGIN - 14.0;
        let mut y = 0.0;
        for (index, (row, slot)) in rows.iter().enumerate() {
            let mut cursor = y + 4.0;

            let title = NSTextField::labelWithString(&NSString::from_str(&format!("{}  ({})", row.name, row.id)), mtm);
            title.setFrame(NSRect::new(
                NSPoint::new(MARGIN + 14.0, cursor),
                NSSize::new(line_width, 18.0),
            ));
            title.setFont(Some(&NSFont::boldSystemFontOfSize(13.0)));
            title.setToolTip(Some(&title.stringValue()));
            document.addSubview(&title);
            cursor += 18.0;

            if let Some(description) = &row.description {
                let label = NSTextField::labelWithString(&NSString::from_str(description), mtm);
                label.setFrame(NSRect::new(
                    NSPoint::new(MARGIN + 14.0, cursor),
                    NSSize::new(line_width, 15.0),
                ));
                label.setFont(Some(&NSFont::systemFontOfSize(11.0)));
                label.setTextColor(Some(&NSColor::secondaryLabelColor()));
                label.setToolTip(Some(&NSString::from_str(description)));
                document.addSubview(&label);
                cursor += 16.0;
            }

            let provenance = NSTextField::labelWithString(&NSString::from_str(&provenance_text(row)), mtm);
            provenance.setFrame(NSRect::new(
                NSPoint::new(MARGIN + 14.0, cursor),
                NSSize::new(line_width, 15.0),
            ));
            provenance.setFont(Some(&NSFont::systemFontOfSize(11.0)));
            provenance.setTextColor(Some(&NSColor::secondaryLabelColor()));
            provenance.setToolTip(Some(&provenance.stringValue()));
            document.addSubview(&provenance);
            cursor += 16.0;

            if !row.dependencies.is_empty() {
                let label = NSTextField::labelWithString(
                    &NSString::from_str(&format!("Needs: {}", row.dependencies.join(", "))),
                    mtm,
                );
                label.setFrame(NSRect::new(
                    NSPoint::new(MARGIN + 14.0, cursor),
                    NSSize::new(line_width, 15.0),
                ));
                label.setFont(Some(&NSFont::systemFontOfSize(11.0)));
                label.setTextColor(Some(&NSColor::secondaryLabelColor()));
                document.addSubview(&label);
                cursor += 16.0;
            }

            if can_install(row) {
                let titles: Vec<Retained<NSString>> = row
                    .versions
                    .iter()
                    .map(|version| NSString::from_str(&version.to_string()))
                    .collect();
                let titles = NSArray::from_retained_slice(&titles);
                let versions = NSPopUpButton::initWithFrame_pullsDown(
                    NSPopUpButton::alloc(mtm),
                    NSRect::new(NSPoint::new(MARGIN + 14.0, cursor), NSSize::new(110.0, 26.0)),
                    false,
                );
                versions.addItemsWithTitles(&titles);
                versions.selectItemAtIndex(slot.unwrap_or(0) as isize);
                versions.setTag(index as isize);
                // SAFETY: the popup, its target and the selector live on the main
                // thread, and the selector is one this class implements.
                unsafe {
                    versions.setTarget(Some(self.as_ref()));
                    versions.setAction(Some(sel!(versionPicked:)));
                }
                document.addSubview(&versions);

                let label = if row.installed.is_some() { "Update" } else { "Install" };
                let install = button(label, MARGIN + 132.0, cursor + 1.0, self, sel!(installRow:), mtm);
                install.setTag(index as isize);
                document.addSubview(&install);
            } else {
                let text = if row.versions.is_empty() {
                    "No release of this extension runs on this build."
                } else {
                    "Another copy is already installed; remove it before installing this one."
                };
                let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
                label.setFrame(NSRect::new(
                    NSPoint::new(MARGIN + 14.0, cursor),
                    NSSize::new(line_width, 15.0),
                ));
                label.setFont(Some(&NSFont::systemFontOfSize(11.0)));
                label.setTextColor(Some(&NSColor::systemOrangeColor()));
                document.addSubview(&label);
            }

            y += heights.get(index).copied().unwrap_or_else(|| search_height(row));
        }
        built.scroll.setDocumentView(Some(&document));
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

            let installed_tab = button(
                "Installed",
                MARGIN,
                content_height - TAB_GAP - 28.0,
                self,
                sel!(tabInstalled:),
                mtm,
            );
            let search_tab = button(
                "Search",
                MARGIN + 98.0,
                content_height - TAB_GAP - 28.0,
                self,
                sel!(tabSearch:),
                mtm,
            );

            let hint = NSTextField::labelWithString(
                &NSString::from_str(
                    "Anything under an `extensions/` folder is listed here. A switch takes effect at once.",
                ),
                mtm,
            );
            hint.setFrame(NSRect::new(
                NSPoint::new(MARGIN, content_height - HINT_GAP - 17.0),
                NSSize::new(WIDTH - 2.0 * MARGIN, 17.0),
            ));

            let master = NSButton::initWithFrame(
                NSButton::alloc(mtm),
                NSRect::new(
                    NSPoint::new(MARGIN, content_height - MASTER_GAP - 20.0),
                    NSSize::new(280.0, 20.0),
                ),
            );
            master.setButtonType(NSButtonType::Switch);
            master.setTitle(&NSString::from_str("Load extensions"));

            let search_field = NSSearchField::initWithFrame(
                NSSearchField::alloc(mtm),
                NSRect::new(
                    NSPoint::new(MARGIN, content_height - SEARCH_FIELD_GAP - 26.0),
                    NSSize::new(WIDTH - 2.0 * MARGIN, 26.0),
                ),
            );
            search_field.setPlaceholderString(Some(&NSString::from_str("Search the registries")));
            search_field.setSendsSearchStringImmediately(true);
            search_field.setTarget(Some(self.as_ref()));
            search_field.setAction(Some(sel!(searchChanged:)));

            let scroll = NSScrollView::initWithFrame(
                NSScrollView::alloc(mtm),
                NSRect::new(
                    NSPoint::new(MARGIN, BOTTOM),
                    NSSize::new(WIDTH - 2.0 * MARGIN, content_height - INSTALLED_SCROLL_GAP - BOTTOM),
                ),
            );
            scroll.setHasVerticalScroller(true);
            scroll.setBorderType(NSBorderType::BezelBorder);

            let status = NSTextView::initWithFrame(
                NSTextView::alloc(mtm),
                NSRect::new(NSPoint::new(MARGIN, 48.0), NSSize::new(WIDTH - 2.0 * MARGIN, 40.0)),
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

            content.addSubview(&installed_tab);
            content.addSubview(&search_tab);
            content.addSubview(&hint);
            content.addSubview(&master);
            content.addSubview(&search_field);
            content.addSubview(&scroll);
            content.addSubview(&status);
            content.addSubview(&save);
            content.addSubview(&revert);
            content.addSubview(&close);

            Built {
                window,
                hint,
                master,
                search_field,
                scroll,
                status,
                installed_tab,
                search_tab,
                tab: Cell::new(Tab::Installed),
                rows: RefCell::new(Vec::new()),
                remove_ids: RefCell::new(Vec::new()),
            }
        }
    }

    fn set_status(&self, message: &str) {
        if let Some(built) = &*self.ivars().built.borrow() {
            built.status.setString(&NSString::from_str(message));
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

/// The line a registry install shows under its name.
fn source_text(source: &registry::Provenance) -> String {
    format!("Installed {} from {}", source.version, source.registry)
}

/// A Search row's provenance: which registry lists it and what is on disk.
fn provenance_text(row: &registry::SearchRow) -> String {
    let mut text = format!("{} · {}", row.registry_name, row.registry);
    match &row.installed {
        None => {}
        Some(registry::Local::Manual) => text.push_str(" · installed by hand"),
        Some(registry::Local::Registry(found)) => {
            text.push_str(&format!(" · installed {}", found.version));
            if found.registry == row.registry {
                text.push_str(" from this registry");
            }
        }
    }
    text
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

/// What the Search tab's status line says: each registry's answer and the last
/// install or uninstall.
fn status_for(screen: &registry::Screen) -> String {
    if screen.urls().is_empty() {
        return "No registries are configured. Add `[extensions] registries` to the settings file.".to_string();
    }
    if !screen.opened() {
        return "Open this tab to read the registries.".to_string();
    }
    let mut lines = Vec::new();
    if screen.is_loading() {
        lines.push("Reading the registries…".to_string());
    }
    for (url, state) in screen.urls().iter().zip(screen.states()) {
        match state {
            registry::State::Loading | registry::State::Ready { cached: false, .. } => {}
            registry::State::Ready { cached: true, note } => {
                let note = note.as_deref().unwrap_or_default();
                lines.push(format!("{url}: the last good copy is in use ({note})"));
            }
            registry::State::Failed(message) => lines.push(format!("{url}: {message}")),
        }
    }
    if let Some(status) = screen.status() {
        lines.push(match status {
            registry::Status::Working => "Working…".to_string(),
            registry::Status::Done(message) | registry::Status::Failed(message) => message.clone(),
        });
    }
    lines.join("\n")
}

/// Whether a Search row offers the version picker and an install button.
fn can_install(row: &registry::SearchRow) -> bool {
    row.installable() && !row.versions.is_empty()
}

/// How tall one Search row is: its title, the lines it has, and its controls.
fn search_height(row: &registry::SearchRow) -> f64 {
    let mut height = 4.0 + 18.0;
    if row.description.is_some() {
        height += 16.0;
    }
    height += 16.0;
    if !row.dependencies.is_empty() {
        height += 16.0;
    }
    height += if can_install(row) { 34.0 } else { 20.0 };
    height + 6.0
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
            source: None,
        }
    }

    #[test]
    fn a_row_names_the_extension_and_what_it_ships() {
        let (title, detail) = lines(&found("Failed command", "a.one", "script", true, true, ""));
        assert_eq!(title, "Failed command — a.one");
        assert_eq!(detail, "script");
    }

    #[test]
    fn why_a_row_is_not_running_is_on_its_detail_line() {
        let (_title, detail) = lines(&found(
            "Broken",
            "a.one",
            "broken",
            false,
            false,
            "the manifest is not TOML",
        ));
        assert_eq!(detail, "broken · the manifest is not TOML");
    }

    #[test]
    fn the_summary_counts_unreadable_extensions_apart() {
        let rows = [
            found("One", "a.one", "script", true, true, ""),
            found("Two", "b.two", "broken", false, false, "why"),
        ];
        let text = summary(&rows);
        assert!(text.contains("2 extension(s)"), "{text}");
        assert!(text.contains("1 enabled"), "{text}");
        assert!(text.contains("1 unreadable"), "{text}");
    }

    #[test]
    fn an_empty_list_says_where_to_put_one() {
        let text = summary(&[]);
        assert!(text.contains("No extensions found"), "{text}");
        assert!(text.contains("extensions/"), "{text}");
    }

    #[test]
    fn a_registry_row_names_its_source() {
        let source = registry::Provenance {
            registry: "https://one.example/r.toml".to_string(),
            version: "0.4.2".parse().expect("semver"),
            archive: "https://x/y.zip".to_string(),
            sha256: "a".repeat(64),
            installed: 1,
        };
        assert_eq!(source_text(&source), "Installed 0.4.2 from https://one.example/r.toml");
    }
}

//! The tab strip, the scrollbar and the pane's colours.
//!
//! AppKit owns the window, the menus and the dialogs; the chrome next to the
//! terminal is ours, as it is in the WinUI and GTK shells (ADRs 061, 064). The
//! strip is painted here from `winter::tabs`'s geometry, and the terminal's
//! pixels are drawn by [`Shell::frame`]. No AppKit control paints the tab row,
//! so a theme's fills reach it exactly as they do on Windows.

use objc2::runtime::AnyObject;
use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{
    NSAttributedStringNSStringDrawing, NSBezierPath, NSColor, NSFont, NSMutableParagraphStyle, NSTextAlignment,
};
use objc2_foundation::{
    NSAttributedString, NSAttributedStringKey, NSDictionary, NSMutableDictionary, NSPoint, NSRect, NSSize, NSString,
};
use utils::InclusiveRect;

use winter::layout::{Axis, Fraction, SplitPath};
use winter::scrollbar;
use winter::tabs::{self, TabBox};

use super::icon;

/// The traffic lights occupy this much of the strip's left edge.
///
/// The content view extends under the title bar (`FullSizeContentView`), so the
/// strip has to leave room for the window buttons AppKit draws above it. A
/// full-screen window hides them, and [`strip`] is then given no inset.
pub(crate) const LEFT_INSET: f64 = 78.0;
/// The strip's height in points.
pub(crate) const STRIP_HEIGHT: f64 = 34.0;
/// The terminal text's left inset, matching the WinUI pane (ADR 068).
pub(crate) const PANE_LEFT: f64 = 8.0;
/// The strip stops this far short of the window's right edge.
pub(crate) const RIGHT_PAD: f64 = 8.0;
/// The new-tab button's width. `tabs::NEW_TAB` is the shared figure.
pub(crate) const BUTTON: f64 = tabs::NEW_TAB as f64;
/// The profile drop-down's width.
pub(crate) const DROP_DOWN: f64 = 28.0;
/// How far a tab paints past the row, so its rounded bottom is behind the pane.
const TAB_OVERHANG: f64 = 4.0;
/// The strip's corner radius.
const RADIUS: f64 = 6.0;
/// The add, drop-down and close glyphs' box.
const GLYPH: f64 = 9.0;
/// A tab's program icon.
const TAB_ICON: f64 = 14.0;

/// Where the tabs and the two buttons sit in the strip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Strip {
    /// One box per tab, left to right.
    pub(crate) tabs: Vec<TabBox>,
    /// The new-tab button.
    pub(crate) new_tab: TabBox,
    /// The profile drop-down.
    pub(crate) menu: TabBox,
}

/// What a press on the strip asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StripHit {
    /// Show the tab at this index.
    Tab(usize),
    /// Close the tab at this index.
    Close(usize),
    /// Open the default profile in a new tab.
    New,
    /// Show the profile menu.
    Menu,
}

/// Lays the strip out for a pane `width` points wide, leaving `inset` points at
/// the left for the window buttons. A full-screen window has no buttons and
/// passes zero.
pub(crate) fn strip(width: f64, count: usize, inset: f64) -> Strip {
    let inset = inset.max(0.0);
    let usable = (width - inset - RIGHT_PAD).max(0.0);
    let reserved = BUTTON + DROP_DOWN;
    let layout = tabs::strip_layout((usable - reserved).max(0.0) as i32, count);
    let offset = inset as i32;

    let shifted = |tab: TabBox| TabBox {
        left: tab.left + offset,
        right: tab.right + offset,
    };
    let tabs = layout.tabs.into_iter().map(shifted).collect();
    let new_tab = shifted(layout.new_tab);
    let menu = TabBox {
        left: new_tab.right,
        right: new_tab.right + DROP_DOWN as i32,
    };

    Strip { tabs, new_tab, menu }
}

/// Maps a press at `x` to what it asked for.
pub(crate) fn strip_hit(layout: &Strip, x: f64) -> Option<StripHit> {
    let x = x.round() as i32;
    if layout.menu.contains(x) {
        return Some(StripHit::Menu);
    }
    if layout.new_tab.contains(x) {
        return Some(StripHit::New);
    }

    layout.tabs.iter().enumerate().find_map(|(index, tab)| {
        if !tab.contains(x) {
            return None;
        }
        if tab.right - x <= tabs::CLOSE {
            Some(StripHit::Close(index))
        } else {
            Some(StripHit::Tab(index))
        }
    })
}

/// The slot a dragged tab lands in when the pointer is at `x`.
///
/// The target is the number of *other* tabs whose midpoint lies left of the
/// pointer, so the dragged tab crosses a neighbour only once the pointer passes
/// that neighbour's middle and does not count itself. Returns `None` when
/// `from` is not a tab in the layout.
pub(crate) fn drag_target(layout: &Strip, from: usize, x: f64) -> Option<usize> {
    tabs::drag_target(&layout.tabs, from, x.round() as i32)
}

/// The strip's fills and its two label colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StripColors {
    /// The row behind the tabs, when a theme names one.
    pub(crate) row: Option<(u8, u8, u8)>,
    /// The active tab's fill.
    pub(crate) active: (u8, u8, u8),
    /// Every other tab's fill.
    pub(crate) inactive: (u8, u8, u8),
    /// The active tab's label.
    pub(crate) active_text: (u8, u8, u8),
    /// Every other tab's label.
    pub(crate) inactive_text: (u8, u8, u8),
}

/// Picks the strip's colours from the pane's own palette and a theme.
///
/// `row` is the theme's row fill, if it names one; without it AppKit's window
/// background shows through. The active tab takes the pane's fill, so the tab
/// and the terminal meet; an inactive tab is a step towards the text colour.
pub(crate) fn strip_colors(
    background: u32,
    foreground: u32,
    theme_row: Option<(u8, u8, u8)>,
    theme_active: Option<(u8, u8, u8)>,
    theme_inactive: Option<(u8, u8, u8)>,
) -> StripColors {
    let pane = rgb(background);
    let text = rgb(foreground);
    let active = theme_active.unwrap_or(pane);
    let inactive = theme_inactive.unwrap_or_else(|| blend(pane, text, 0.08));
    let active_text = contrasting(active);
    // Without a theme the row is a step off the pane rather than AppKit's
    // window colour, so a light scheme and a dark scheme both read as one
    // surface from the window's top edge down.
    let row = theme_row.or_else(|| Some(blend(pane, active_text, 0.06)));

    StripColors {
        row,
        active,
        inactive,
        active_text,
        inactive_text: contrasting(inactive),
    }
}

/// A `COLORREF` (`0x00BBGGRR`) as red, green and blue.
pub(crate) const fn rgb(color: u32) -> (u8, u8, u8) {
    (
        (color & 0xFF) as u8,
        ((color >> 8) & 0xFF) as u8,
        ((color >> 16) & 0xFF) as u8,
    )
}

/// Black or white, whichever reads on `fill`.
pub(crate) fn contrasting(fill: (u8, u8, u8)) -> (u8, u8, u8) {
    let luminance = 0.299 * f32::from(fill.0) + 0.587 * f32::from(fill.1) + 0.114 * f32::from(fill.2);
    if luminance > 140.0 {
        (20, 20, 20)
    } else {
        (230, 230, 230)
    }
}

/// Mixes two opaque colours, fraction `t` of the way from `a` to `b`.
pub(crate) fn blend(a: (u8, u8, u8), b: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    let channel = |from: u8, to: u8| {
        let from = f32::from(from);
        let to = f32::from(to);
        (from + (to - from) * t).round().clamp(0.0, 255.0) as u8
    };
    (channel(a.0, b.0), channel(a.1, b.1), channel(a.2, b.2))
}

/// Whether a pane's background calls for AppKit's dark appearance.
pub(crate) fn dark_background(colorref: u32) -> bool {
    let (red, green, blue) = rgb(colorref);
    let luminance = 0.299 * f32::from(red) + 0.587 * f32::from(green) + 0.114 * f32::from(blue);
    luminance < 128.0
}

/// The grid area, below the strip.
pub(crate) fn pane_rect(bounds: NSRect) -> NSRect {
    NSRect::new(
        NSPoint::new(0.0, STRIP_HEIGHT),
        NSSize::new(bounds.size.width, (bounds.size.height - STRIP_HEIGHT).max(0.0)),
    )
}

/// The part of `area` a pane's fraction covers.
pub(crate) fn fraction_rect(area: NSRect, fraction: Fraction) -> NSRect {
    NSRect::new(
        NSPoint::new(
            area.origin.x + area.size.width * f64::from(fraction.x),
            area.origin.y + area.size.height * f64::from(fraction.y),
        ),
        NSSize::new(
            area.size.width * f64::from(fraction.width),
            area.size.height * f64::from(fraction.height),
        ),
    )
}

/// The whole-cell area the terminal's pixels fill.
pub(crate) fn pane_grid_rect(rect: NSRect, scale: f64, cell: (u16, u16), grid: (i32, i32)) -> NSRect {
    let width = f64::from(cell.0) * f64::from(grid.0.max(1)) / scale;
    let height = f64::from(cell.1) * f64::from(grid.1.max(1)) / scale;
    NSRect::new(
        NSPoint::new(rect.origin.x + PANE_LEFT, rect.origin.y),
        NSSize::new(width, height),
    )
}

/// The half a split of `rect` gives its new, second pane.
pub(crate) fn split_second(rect: NSRect, axis: Axis) -> NSRect {
    match axis {
        Axis::Horizontal => NSRect::new(
            NSPoint::new(rect.origin.x + rect.size.width / 2.0, rect.origin.y),
            NSSize::new(rect.size.width / 2.0, rect.size.height),
        ),
        Axis::Vertical => NSRect::new(
            NSPoint::new(rect.origin.x, rect.origin.y + rect.size.height / 2.0),
            NSSize::new(rect.size.width, rect.size.height / 2.0),
        ),
    }
}

/// A pane's scrollbar track, on its right edge.
pub(crate) fn pane_scrollbar_rect(rect: NSRect) -> NSRect {
    NSRect::new(
        NSPoint::new(
            rect.origin.x + rect.size.width - f64::from(scrollbar::WIDTH),
            rect.origin.y,
        ),
        NSSize::new(f64::from(scrollbar::WIDTH), rect.size.height),
    )
}

/// The split whose divider is within `grab` points of `point`, if any.
pub(crate) fn divider_at(
    bounds: NSRect,
    splits: &[(SplitPath, Axis, Fraction, f32)],
    point: (f64, f64),
    grab: f64,
) -> Option<(SplitPath, Axis, NSRect)> {
    let area = pane_rect(bounds);
    // The deepest split wins, so a nested divider is not lost under its parent.
    for (path, axis, fraction, ratio) in splits.iter().rev() {
        let split = fraction_rect(area, *fraction);
        match axis {
            Axis::Horizontal => {
                let x = split.origin.x + split.size.width * f64::from(*ratio);
                if (point.0 - x).abs() <= grab
                    && point.1 >= split.origin.y
                    && point.1 <= split.origin.y + split.size.height
                {
                    return Some((path.clone(), *axis, split));
                }
            }
            Axis::Vertical => {
                let y = split.origin.y + split.size.height * f64::from(*ratio);
                if (point.1 - y).abs() <= grab
                    && point.0 >= split.origin.x
                    && point.0 <= split.origin.x + split.size.width
                {
                    return Some((path.clone(), *axis, split));
                }
            }
        }
    }
    None
}

/// The find bar's backdrop inside a pane.
///
/// The controls are subviews; this is the panel behind them, so the terminal's
/// text does not show through the count or beside the field. The overlay layer
/// the Metal path draws above its content uses the same geometry.
pub(crate) fn find_backdrop_rect(pane: NSRect) -> NSRect {
    let width = (pane.size.width - 16.0).clamp(0.0, 342.0);
    NSRect::new(
        NSPoint::new(pane.origin.x + pane.size.width - width - 8.0, pane.origin.y + 4.0),
        NSSize::new(width, 32.0),
    )
}

/// The command palette card's width in points.
const PALETTE_WIDTH: f64 = 480.0;
/// The card's inner padding.
const PALETTE_PAD: f64 = 8.0;
/// The query field's height.
const PALETTE_FIELD: f64 = 24.0;
/// One row's height.
const PALETTE_ROW: f64 = 24.0;
/// The list's cap, matching the GTK card's 320 points.
const PALETTE_MAX_ROWS: usize = 13;
/// The card's top edge, below the strip.
const PALETTE_TOP: f64 = STRIP_HEIGHT + 16.0;

/// The rows the card shows for `rows` entries; an empty filter keeps one line
/// for the notice.
pub(crate) fn palette_visible_rows(rows: usize) -> usize {
    rows.clamp(1, PALETTE_MAX_ROWS)
}

/// The first row drawn, so the selection stays on screen.
pub(crate) fn palette_first_visible(rows: usize, selected: usize) -> usize {
    let visible = palette_visible_rows(rows);
    if rows <= visible {
        0
    } else {
        selected.saturating_sub(visible - 1)
    }
}

/// The card's frame in the view's points.
pub(crate) fn palette_rect(bounds: NSRect, rows: usize) -> NSRect {
    let visible = palette_visible_rows(rows);
    let width = PALETTE_WIDTH.min((bounds.size.width - 32.0).max(160.0));
    let height = 2.0 * PALETTE_PAD + PALETTE_FIELD + PALETTE_ROW * visible as f64;
    NSRect::new(
        NSPoint::new(((bounds.size.width - width) / 2.0).max(8.0), PALETTE_TOP),
        NSSize::new(width, height),
    )
}

/// The query field inside the card, in the card's own coordinates (its bounds
/// origin is `0, 0`).
pub(crate) fn palette_field_rect(card: NSRect) -> NSRect {
    NSRect::new(
        NSPoint::new(PALETTE_PAD, PALETTE_PAD),
        NSSize::new((card.size.width - 2.0 * PALETTE_PAD).max(1.0), PALETTE_FIELD),
    )
}

/// The row under `point` (in the card's coordinates), as an index into the
/// filtered rows. `first` is the first row drawn.
pub(crate) fn palette_row_at(point: (f64, f64), rows: usize, first: usize) -> Option<usize> {
    let list_top = PALETTE_PAD + PALETTE_FIELD;
    if point.1 < list_top {
        return None;
    }
    let offset = ((point.1 - list_top) / PALETTE_ROW).floor() as usize;
    if offset >= palette_visible_rows(rows) {
        return None;
    }
    let index = first + offset;
    (index < rows).then_some(index)
}

/// What the palette card paints.
pub(crate) struct PalettePaint<'a> {
    pub(crate) rows: &'a [winter::palette::Entry],
    pub(crate) selected: usize,
    pub(crate) first: usize,
    pub(crate) colors: StripColors,
}

/// Draws the command palette's card, rows and selection.
pub(crate) fn paint_palette(bounds: NSRect, state: &PalettePaint<'_>, mtm: MainThreadMarker) {
    let card = state.colors.row.unwrap_or(state.colors.active);
    let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(bounds, 8.0, 8.0);
    color(card).setFill();
    path.fill();
    // A hairline, because a shadow would need a layer of its own above the
    // content layer.
    color(blend(card, state.colors.active_text, 0.18)).setStroke();
    path.setLineWidth(1.0);
    path.stroke();

    let text = state.colors.active_text;
    let dim = blend(text, card, 0.45);
    let font = NSFont::systemFontOfSize(12.0);
    let chord_font = NSFont::systemFontOfSize(11.0);
    let list_top = PALETTE_PAD + PALETTE_FIELD;
    let left = PALETTE_PAD + 4.0;
    let width = (bounds.size.width - 2.0 * PALETTE_PAD - 8.0).max(1.0);

    if state.rows.is_empty() {
        draw_text("No matching command", left, list_top + 6.0, width, dim, &font, mtm);
        return;
    }

    for offset in 0..palette_visible_rows(state.rows.len()) {
        let index = state.first + offset;
        let Some(entry) = state.rows.get(index) else {
            break;
        };
        let top = list_top + offset as f64 * PALETTE_ROW;

        if index == state.selected {
            let rect = NSRect::new(
                NSPoint::new(PALETTE_PAD, top),
                NSSize::new(bounds.size.width - 2.0 * PALETTE_PAD, PALETTE_ROW),
            );
            let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, 4.0, 4.0);
            color(blend(card, text, 0.18)).setFill();
            path.fill();
        }

        let chord = entry.chord.as_deref().map(|chord| {
            let width = text_width(chord, &chord_font, mtm);
            (chord, width)
        });
        let chord_room = chord.map_or(0.0, |(_, width)| width + 12.0);
        draw_text(&entry.label, left, top + 6.0, width - chord_room, text, &font, mtm);
        if let Some((chord, chord_width)) = chord {
            draw_text(
                chord,
                bounds.size.width - PALETTE_PAD - 4.0 - chord_width,
                top + 7.0,
                chord_width + 2.0,
                dim,
                &chord_font,
                mtm,
            );
        }
    }
}

/// The width `text` draws at with `font`.
fn text_width(text: &str, font: &NSFont, mtm: MainThreadMarker) -> f64 {
    let attributes = text_attributes((255, 255, 255), font, mtm);
    attributed(text, &attributes).map_or(0.0, |text| text.size().width)
}

/// One pane the chrome paints.
pub(crate) struct PanePaint<'a> {
    /// The pane's area in the view's points.
    pub(crate) rect: NSRect,
    /// The grid's rectangle inside the pane.
    pub(crate) grid_rect: NSRect,
    /// The pane's own fill.
    pub(crate) fill: (u8, u8, u8),
    /// The terminal's pixels, when the view blits them.
    pub(crate) frame: Option<&'a crate::frame::Frame>,
    /// The scrollbar's scroll, document height and visible rows.
    pub(crate) scroll: (i32, i32, i32),
    /// The prompt ticks.
    pub(crate) marks: &'a [winter::grid::MarkTick],
    /// The bell's opacity, while this pane flashes.
    pub(crate) bell: Option<f32>,
    /// Whether this pane's find bar is showing.
    pub(crate) find_open: bool,
    /// Whether this pane has the focus.
    pub(crate) focused: bool,
}

/// What the chrome paints from.
pub(crate) struct ChromeState<'a> {
    /// The strip's layout.
    pub(crate) strip: Strip,
    /// The active tab's index.
    pub(crate) active: usize,
    /// The tab titles, in order.
    pub(crate) titles: Vec<String>,
    /// The program each tab runs, in order, for the tabs' icons.
    pub(crate) programs: Vec<String>,
    /// The tab the pointer is over.
    pub(crate) hovered_tab: Option<usize>,
    /// The tab whose close box the pointer is over.
    pub(crate) hovered_close: Option<usize>,
    /// Whether the pointer is over the new-tab button.
    pub(crate) hovered_new: bool,
    /// Whether the pointer is over the drop-down.
    pub(crate) hovered_menu: bool,
    /// The strip's colours.
    pub(crate) colors: StripColors,
    /// The active tab's panes.
    pub(crate) panes: &'a [PanePaint<'a>],
    /// The active tab's split tree, for its dividers.
    pub(crate) splits: &'a [(SplitPath, Axis, Fraction, f32)],
    /// Whether to paint the find backdrop and the bells. The Metal path draws
    /// them as layers above its content instead.
    pub(crate) overlays: bool,
}

/// Draws the strip, the panes and their scrollbars.
pub(crate) fn paint(bounds: NSRect, state: &ChromeState<'_>, mtm: MainThreadMarker) {
    let strip_rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(bounds.size.width, STRIP_HEIGHT));

    match state.colors.row {
        Some(color) => fill(strip_rect, color),
        // Without a theme the window's own background is the row, which is
        // what makes a dark scheme look native in dark mode.
        None => {
            NSColor::windowBackgroundColor().setFill();
            NSBezierPath::fillRect(strip_rect);
        }
    }

    draw_tabs(bounds, state, mtm);
    draw_buttons(state);

    // The panes cover the tabs' overhang, so a tab meets the terminal.
    for pane in state.panes {
        fill(pane.rect, pane.fill);
    }

    for pane in state.panes {
        if let Some(frame) = pane.frame {
            let room = NSRect::new(
                pane.grid_rect.origin,
                NSSize::new(
                    pane.grid_rect
                        .size
                        .width
                        .min((pane.rect.size.width - PANE_LEFT).max(1.0)),
                    pane.grid_rect.size.height.min(pane.rect.size.height.max(1.0)),
                ),
            );
            frame.draw(room);
        }
    }

    draw_dividers(bounds, state);

    if state.overlays {
        for pane in state.panes {
            if pane.find_open {
                draw_find_backdrop(pane.rect, state.colors);
            }
        }
    }

    for pane in state.panes {
        draw_scrollbar(pane);
    }

    if state.overlays {
        for pane in state.panes {
            draw_bell(pane.rect, pane.bell);
        }
    }

    for pane in state.panes {
        if state.panes.len() > 1 && pane.focused {
            draw_focus_border(pane);
        }
    }
}

fn draw_tabs(bounds: NSRect, state: &ChromeState<'_>, mtm: MainThreadMarker) {
    let font = NSFont::systemFontOfSize(12.0);
    for (index, slots) in state.strip.tabs.iter().enumerate() {
        // A tab past the window's edge is not drawn, and so is not pressed.
        if f64::from(slots.left) >= bounds.size.width {
            break;
        }
        let Some(title) = state.titles.get(index) else {
            continue;
        };

        let mut fill_color = if index == state.active {
            state.colors.active
        } else {
            state.colors.inactive
        };
        if state.hovered_tab == Some(index) && index != state.active {
            fill_color = blend(fill_color, state.colors.active_text, 0.08);
        }
        let (text_color, close_tint) = if index == state.active {
            (state.colors.active_text, state.colors.active_text)
        } else {
            (state.colors.inactive_text, state.colors.inactive_text)
        };

        let rect = NSRect::new(
            NSPoint::new(f64::from(slots.left) + 1.0, 4.0),
            NSSize::new(f64::from(slots.width()) - 2.0, STRIP_HEIGHT - 4.0 + TAB_OVERHANG),
        );
        let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, RADIUS, RADIUS);
        color(fill_color).setFill();
        path.fill();

        let icon = state.programs.get(index).and_then(|program| icon::for_program(program));
        let title_x = if let Some(image) = &icon {
            let rect = NSRect::new(
                NSPoint::new(f64::from(slots.left) + 8.0, (STRIP_HEIGHT - TAB_ICON) / 2.0),
                NSSize::new(TAB_ICON, TAB_ICON),
            );
            image.drawInRect(rect);
            f64::from(slots.left) + 8.0 + TAB_ICON + 6.0
        } else {
            f64::from(slots.left) + 10.0
        };
        let room = f64::from(slots.right) - title_x - f64::from(tabs::CLOSE) - 6.0;
        if room > 8.0 {
            draw_text(title, title_x, 9.0, room, text_color, &font, mtm);
        }

        let close = if state.hovered_close == Some(index) {
            blend(close_tint, fill_color, 0.2)
        } else {
            close_tint
        };
        let centre = NSPoint::new(
            f64::from(slots.right) - f64::from(tabs::CLOSE) / 2.0,
            STRIP_HEIGHT / 2.0,
        );
        glyph_cross(centre, close);
    }
}

fn draw_buttons(state: &ChromeState<'_>) {
    let base = state.colors.row.unwrap_or(state.colors.active);
    let text = state.colors.active_text;

    if state.hovered_new {
        fill(button_rect(state.strip.new_tab), blend(base, text, 0.1));
    }
    if state.hovered_menu {
        fill(button_rect(state.strip.menu), blend(base, text, 0.1));
    }

    let plus_centre = NSPoint::new(
        f64::from(state.strip.new_tab.left + state.strip.new_tab.right) / 2.0,
        STRIP_HEIGHT / 2.0,
    );
    let menu_centre = NSPoint::new(
        f64::from(state.strip.menu.left + state.strip.menu.right) / 2.0,
        STRIP_HEIGHT / 2.0,
    );
    glyph_plus(plus_centre, text);
    glyph_chevron(menu_centre, text);
}

fn draw_dividers(bounds: NSRect, state: &ChromeState<'_>) {
    if state.panes.len() < 2 {
        return;
    }
    let area = pane_rect(bounds);
    for (_path, axis, fraction, ratio) in state.splits {
        let split = fraction_rect(area, *fraction);
        let rect = match axis {
            Axis::Horizontal => NSRect::new(
                NSPoint::new(
                    split.origin.x + split.size.width * f64::from(*ratio) - 0.5,
                    split.origin.y,
                ),
                NSSize::new(1.0, split.size.height),
            ),
            Axis::Vertical => NSRect::new(
                NSPoint::new(
                    split.origin.x,
                    split.origin.y + split.size.height * f64::from(*ratio) - 0.5,
                ),
                NSSize::new(split.size.width, 1.0),
            ),
        };
        NSColor::colorWithSRGBRed_green_blue_alpha(0.5, 0.5, 0.5, 0.35).setFill();
        NSBezierPath::fillRect(rect);
    }
}

fn draw_focus_border(pane: &PanePaint<'_>) {
    let rect = NSRect::new(
        NSPoint::new(pane.rect.origin.x + 0.5, pane.rect.origin.y + 0.5),
        NSSize::new(
            (pane.rect.size.width - 1.0).max(1.0),
            (pane.rect.size.height - 1.0).max(1.0),
        ),
    );
    let path = NSBezierPath::bezierPathWithRect(rect);
    color(blend(pane.fill, contrasting(pane.fill), 0.45)).setStroke();
    path.setLineWidth(1.0);
    path.stroke();
}

fn draw_scrollbar(pane: &PanePaint<'_>) {
    let (scroll, document, visible) = pane.scroll;
    let track_rect = pane_scrollbar_rect(pane.rect);
    let track = track_rect.size.height.max(0.0);
    if document <= visible || track <= 0.0 {
        return;
    }

    NSColor::colorWithSRGBRed_green_blue_alpha(0.0, 0.0, 0.0, 0.35).setFill();
    NSBezierPath::fillRect(track_rect);

    let (top, height) = scrollbar::thumb_pixels(scroll, document, visible, track);
    let thumb = NSRect::new(
        NSPoint::new(track_rect.origin.x + 1.0, track_rect.origin.y + top),
        NSSize::new((track_rect.size.width - 2.0).max(1.0), height),
    );
    let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(thumb, 2.0, 2.0);
    NSColor::colorWithSRGBRed_green_blue_alpha(0.63, 0.63, 0.63, 0.8).setFill();
    path.fill();

    // Ticks come after the thumb, the way upstream declares its mark image
    // above the scroll bar (ADR 059).
    for mark in pane.marks {
        let position = scrollbar::position(mark.row, document);
        let top = track_rect.origin.y + f64::from(position) * track - 0.75;
        let rect = NSRect::new(
            NSPoint::new(track_rect.origin.x, top),
            NSSize::new(track_rect.size.width, 1.5),
        );
        if mark.error {
            NSColor::colorWithSRGBRed_green_blue_alpha(0.92, 0.35, 0.35, 0.95).setFill();
        } else {
            NSColor::colorWithSRGBRed_green_blue_alpha(0.43, 0.67, 1.0, 0.9).setFill();
        }
        NSBezierPath::fillRect(rect);
    }
}

fn draw_bell(rect: NSRect, bell: Option<f32>) {
    let Some(alpha) = bell else {
        return;
    };
    NSColor::colorWithSRGBRed_green_blue_alpha(1.0, 1.0, 1.0, f64::from(alpha) * 0.12).setFill();
    NSBezierPath::fillRect(rect);
}

fn draw_find_backdrop(pane: NSRect, colors: StripColors) {
    let rect = find_backdrop_rect(pane);
    let fill_color = colors.row.unwrap_or(colors.active);
    let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, 6.0, 6.0);
    color(fill_color).setFill();
    path.fill();
}

fn button_rect(slots: TabBox) -> NSRect {
    NSRect::new(
        NSPoint::new(f64::from(slots.left), 4.0),
        NSSize::new(f64::from(slots.width()), STRIP_HEIGHT - 8.0),
    )
}

/// Draws `text` at `x`, `top`, elided to `room` points.
#[allow(clippy::too_many_arguments)]
fn draw_text(text: &str, x: f64, top: f64, room: f64, color: (u8, u8, u8), font: &NSFont, mtm: MainThreadMarker) {
    let attributes = text_attributes(color, font, mtm);
    let mut shown: String = text.chars().take(120).collect();
    loop {
        let Some(attributed) = attributed(&shown, &attributes) else {
            return;
        };
        let size = attributed.size();
        if size.width <= room || shown.is_empty() {
            attributed.drawAtPoint(NSPoint::new(x, top));
            return;
        }

        // The strip is one line: drop characters until the ellipsis fits.
        let trimmed = if shown.ends_with('…') {
            let mut chars: Vec<char> = shown.chars().collect();
            chars.pop();
            chars.pop();
            chars.push('…');
            chars.into_iter().collect()
        } else if shown.chars().count() > 1 {
            let mut chars: Vec<char> = shown.chars().collect();
            chars.pop();
            chars.push('…');
            chars.into_iter().collect()
        } else {
            String::new()
        };
        shown = trimmed;
    }
}

fn text_attributes(
    color: (u8, u8, u8),
    font: &NSFont,
    _mtm: MainThreadMarker,
) -> objc2::rc::Retained<NSMutableDictionary<NSAttributedStringKey, AnyObject>> {
    let attributes = NSMutableDictionary::<NSAttributedStringKey, AnyObject>::new();
    // SAFETY: the three names are AppKit's attribute keys, which are the
    // `NSString` keys the initializer and the paragraph style expect.
    unsafe {
        attributes.insert(objc2_app_kit::NSFontAttributeName, &**font);
        attributes.insert(objc2_app_kit::NSForegroundColorAttributeName, &*color_object(color));
        // A tail-truncating paragraph style is what makes an over-long title
        // end in one ellipsis rather than being clipped mid-glyph.
        if let Some(style) = paragraph_style() {
            attributes.insert(objc2_app_kit::NSParagraphStyleAttributeName, &*style);
        }
    }
    attributes
}

fn paragraph_style() -> Option<objc2::rc::Retained<NSMutableParagraphStyle>> {
    let style = NSMutableParagraphStyle::new();
    style.setLineBreakMode(objc2_app_kit::NSLineBreakMode::ByTruncatingTail);
    style.setAlignment(NSTextAlignment::Left);
    Some(style)
}

fn attributed(
    text: &str,
    attributes: &NSDictionary<NSAttributedStringKey, AnyObject>,
) -> Option<objc2::rc::Retained<NSAttributedString>> {
    let string = NSString::from_str(text);
    // SAFETY: `string` is an NSString and `attributes` is an attribute
    // dictionary keyed by the AppKit attribute names, which is what the
    // initializer expects.
    Some(unsafe {
        NSAttributedString::initWithString_attributes(NSAttributedString::alloc(), &string, Some(attributes))
    })
}

fn color(value: (u8, u8, u8)) -> objc2::rc::Retained<NSColor> {
    let channel = |value: u8| f64::from(value) / 255.0;
    NSColor::colorWithSRGBRed_green_blue_alpha(channel(value.0), channel(value.1), channel(value.2), 1.0)
}

fn color_object(value: (u8, u8, u8)) -> objc2::rc::Retained<NSColor> {
    color(value)
}

fn fill(rect: NSRect, value: (u8, u8, u8)) {
    color(value).setFill();
    NSBezierPath::fillRect(rect);
}

fn glyph_plus(centre: NSPoint, value: (u8, u8, u8)) {
    let half = GLYPH / 2.0;
    let path = NSBezierPath::bezierPath();
    path.moveToPoint(NSPoint::new(centre.x - half, centre.y));
    path.lineToPoint(NSPoint::new(centre.x + half, centre.y));
    path.moveToPoint(NSPoint::new(centre.x, centre.y - half));
    path.lineToPoint(NSPoint::new(centre.x, centre.y + half));
    path.setLineWidth(1.4);
    color(value).setStroke();
    path.stroke();
}

fn glyph_chevron(centre: NSPoint, value: (u8, u8, u8)) {
    let half = GLYPH / 2.0;
    let path = NSBezierPath::bezierPath();
    path.moveToPoint(NSPoint::new(centre.x - half, centre.y + half / 2.0));
    path.lineToPoint(NSPoint::new(centre.x, centre.y - half / 2.0));
    path.lineToPoint(NSPoint::new(centre.x + half, centre.y + half / 2.0));
    path.setLineWidth(1.4);
    color(value).setStroke();
    path.stroke();
}

fn glyph_cross(centre: NSPoint, value: (u8, u8, u8)) {
    let half = GLYPH / 2.5;
    let path = NSBezierPath::bezierPath();
    path.moveToPoint(NSPoint::new(centre.x - half, centre.y - half));
    path.lineToPoint(NSPoint::new(centre.x + half, centre.y + half));
    path.moveToPoint(NSPoint::new(centre.x - half, centre.y + half));
    path.lineToPoint(NSPoint::new(centre.x + half, centre.y - half));
    path.setLineWidth(1.4);
    color(value).setStroke();
    path.stroke();
}

/// The pane's grid for a view of `bounds` points at `scale`.
pub(crate) fn grid_for(bounds: NSRect, scale: f64, cell: (u16, u16)) -> (i32, i32) {
    pane_grid(pane_rect(bounds), scale, cell)
}

/// The grid a pane of `rect` points holds at `scale`.
pub(crate) fn pane_grid(rect: NSRect, scale: f64, cell: (u16, u16)) -> (i32, i32) {
    let width = (rect.size.width - PANE_LEFT - f64::from(scrollbar::WIDTH)).max(0.0);
    let height = rect.size.height.max(0.0);
    let columns = (width * scale / f64::from(cell.0.max(1))).floor();
    let rows = (height * scale / f64::from(cell.1.max(1))).floor();
    (whole_cells(columns), whole_cells(rows))
}

fn whole_cells(count: f64) -> i32 {
    if !count.is_finite() {
        return 1;
    }
    (count as i64).clamp(1, 1024) as i32
}

/// The viewport for a grid of `grid` cells.
pub(crate) const fn viewport(grid: (i32, i32)) -> InclusiveRect {
    InclusiveRect::new(
        0,
        0,
        if grid.0 > 1 { grid.0 - 1 } else { 0 },
        if grid.1 > 1 { grid.1 - 1 } else { 0 },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use winter::layout::PaneId;

    #[test]
    fn the_strip_starts_after_the_traffic_lights() {
        let layout = strip(900.0, 2, LEFT_INSET);
        assert_eq!(layout.tabs[0].left, LEFT_INSET as i32);
        assert_eq!(layout.tabs[0].width(), tabs::TAB_MAX);
        assert_eq!(layout.new_tab.left, layout.tabs[1].right);
        assert_eq!(layout.menu.left, layout.new_tab.right);
        assert_eq!(layout.menu.width(), DROP_DOWN as i32);
    }

    #[test]
    fn a_full_screen_strip_starts_at_the_window_edge() {
        let layout = strip(900.0, 2, 0.0);

        assert_eq!(layout.tabs[0].left, 0);
        assert_eq!(layout.tabs[1].left, tabs::TAB_MAX);
        assert_eq!(layout.new_tab.left, 2 * tabs::TAB_MAX);
        assert_eq!(
            strip_hit(&layout, 10.0),
            Some(StripHit::Tab(0)),
            "there is no button gap to drag the window by"
        );
    }

    #[test]
    fn a_press_names_its_tab_close_button_or_the_two_buttons() {
        let layout = strip(900.0, 2, LEFT_INSET);
        let first = layout.tabs[0];

        assert_eq!(strip_hit(&layout, f64::from(first.left + 4)), Some(StripHit::Tab(0)));
        assert_eq!(strip_hit(&layout, f64::from(first.right - 2)), Some(StripHit::Close(0)));
        assert_eq!(
            strip_hit(&layout, f64::from(layout.new_tab.left + 2)),
            Some(StripHit::New)
        );
        assert_eq!(
            strip_hit(&layout, f64::from(layout.menu.left + 2)),
            Some(StripHit::Menu)
        );
        assert_eq!(strip_hit(&layout, 10.0), None, "the traffic lights' room");
    }

    #[test]
    fn tabs_shrink_before_the_buttons_leave_the_row() {
        let layout = strip(500.0, 8, LEFT_INSET);
        assert_eq!(layout.tabs[0].width(), tabs::TAB_MIN);
        assert!(layout.menu.right <= 500);
    }

    #[test]
    fn an_empty_strip_keeps_the_new_tab_and_menu_buttons() {
        let layout = strip(900.0, 0, LEFT_INSET);
        assert!(layout.tabs.is_empty());
        assert_eq!(layout.new_tab.left, LEFT_INSET as i32);
        assert_eq!(layout.menu.left, layout.new_tab.right);
        assert_eq!(
            strip_hit(&layout, f64::from(layout.new_tab.left + 2)),
            Some(StripHit::New)
        );
        assert_eq!(
            strip_hit(&layout, f64::from(layout.menu.left + 2)),
            Some(StripHit::Menu)
        );
    }

    #[test]
    fn a_drag_names_the_slot_it_crossed_into() {
        let layout = strip(900.0, 3, LEFT_INSET);
        let first = layout.tabs[0];
        let second = layout.tabs[1];
        let third = layout.tabs[2];

        // Dragging the first tab: it stays first until it passes the second
        // tab's middle, then it lands second, and last past the third's.
        assert_eq!(drag_target(&layout, 0, f64::from(first.left + 2)), Some(0));
        assert_eq!(
            drag_target(&layout, 0, f64::from(second.left + second.width() / 2)),
            Some(0)
        );
        assert_eq!(
            drag_target(&layout, 0, f64::from(second.left + second.width() / 2 + 1)),
            Some(1)
        );
        assert_eq!(drag_target(&layout, 0, f64::from(third.right - 2)), Some(2));

        // The same rule mirrored: the last tab stays last while it is right of
        // the second tab's middle and lands first once it passes the first's.
        assert_eq!(drag_target(&layout, 2, f64::from(third.left + 2)), Some(2));
        assert_eq!(
            drag_target(&layout, 2, f64::from(second.left + second.width() / 2)),
            Some(1)
        );
        assert_eq!(
            drag_target(&layout, 2, f64::from(second.left + second.width() / 2 + 1)),
            Some(2)
        );
        assert_eq!(
            drag_target(&layout, 2, f64::from(first.left + first.width() / 2 - 1)),
            Some(0)
        );
        assert_eq!(
            drag_target(&layout, 2, f64::from(first.left + first.width() / 2 - 1)),
            Some(0)
        );
    }

    #[test]
    fn a_drag_that_crosses_nothing_keeps_its_slot() {
        let layout = strip(900.0, 3, LEFT_INSET);
        let middle = layout.tabs[1];

        assert_eq!(drag_target(&layout, 1, f64::from(middle.left + 2)), Some(1));
        assert_eq!(drag_target(&layout, 1, f64::from(middle.right - 2)), Some(1));
        // Left of everything, but the first tab's own middle lies beyond it.
        assert_eq!(drag_target(&layout, 1, 0.0), Some(0));
        assert_eq!(drag_target(&layout, 9, 0.0), None);
    }

    #[test]
    fn the_active_tab_takes_the_panes_fill_and_an_inactive_one_steps_towards_text() {
        // Background `0x000000` (black), foreground `0xFFFFFF` (white).
        let colors = strip_colors(0x0000_0000, 0x00FF_FFFF, None, None, None);
        assert_eq!(colors.active, (0, 0, 0));
        assert_eq!(colors.active_text, (230, 230, 230));
        assert_eq!(colors.inactive, (20, 20, 20));
        // The row is a little lighter than the pane, not AppKit's window white.
        assert_eq!(colors.row, Some((14, 14, 14)));
    }

    #[test]
    fn a_theme_fill_wins_over_the_palette() {
        let colors = strip_colors(0, 0xFFFFFF, Some((1, 2, 3)), Some((4, 5, 6)), Some((7, 8, 9)));
        assert_eq!(colors.row, Some((1, 2, 3)));
        assert_eq!(colors.active, (4, 5, 6));
        assert_eq!(colors.inactive, (7, 8, 9));
    }

    #[test]
    fn a_light_fill_gets_a_dark_label() {
        assert_eq!(contrasting((255, 255, 255)), (20, 20, 20));
        assert_eq!(contrasting((0, 0, 0)), (230, 230, 230));
    }

    #[test]
    fn a_colourref_keeps_its_channels() {
        // `0x00BBGGRR`.
        assert_eq!(rgb(0x0012_3456), (0x56, 0x34, 0x12));
    }

    #[test]
    fn the_window_is_dark_over_a_dark_pane() {
        assert!(dark_background(0x0000_0000));
        assert!(dark_background(0x0020_2020));
        assert!(!dark_background(0x00FF_FFFF));
        assert!(!dark_background(0x00E0_E0E0));
    }

    #[test]
    fn the_grid_stops_short_of_the_scrollbar_and_the_strip() {
        let bounds = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(808.0, 534.0));
        // 808 - 8 - 10 = 790 points; at scale 2 and an 8-pixel cell that is 197
        // columns. 534 - 34 = 500 points is 62 rows at 16 pixels.
        assert_eq!(grid_for(bounds, 2.0, (8, 16)), (197, 62));
    }

    #[test]
    fn a_pane_too_small_for_a_cell_still_has_one() {
        let bounds = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, 1.0));
        assert_eq!(grid_for(bounds, 1.0, (8, 16)), (1, 1));
    }

    #[test]
    fn the_viewport_covers_the_whole_grid() {
        assert_eq!(viewport((80, 24)), InclusiveRect::new(0, 0, 79, 23));
        assert_eq!(viewport((1, 1)), InclusiveRect::new(0, 0, 0, 0));
    }

    #[test]
    fn the_find_backdrop_is_a_panel_inside_the_pane() {
        let bounds = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(900.0, 600.0));
        let rect = find_backdrop_rect(pane_rect(bounds));

        assert_eq!(rect.origin, NSPoint::new(900.0 - 342.0 - 8.0, STRIP_HEIGHT + 4.0));
        assert_eq!(rect.size, NSSize::new(342.0, 32.0));
    }

    #[test]
    fn a_narrow_window_shrinks_the_find_backdrop() {
        let bounds = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(200.0, 100.0));
        let rect = find_backdrop_rect(pane_rect(bounds));

        assert_eq!(rect.origin, NSPoint::new(8.0, STRIP_HEIGHT + 4.0));
        assert_eq!(rect.size, NSSize::new(184.0, 32.0));
    }

    #[test]
    fn a_pane_fraction_maps_to_its_part_of_the_area_below_the_strip() {
        let bounds = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(800.0, 600.0));
        let area = pane_rect(bounds);
        let right_half = fraction_rect(
            area,
            Fraction {
                x: 0.5,
                y: 0.0,
                width: 0.5,
                height: 1.0,
            },
        );

        assert_eq!(right_half.origin, NSPoint::new(400.0, STRIP_HEIGHT));
        assert_eq!(right_half.size, NSSize::new(400.0, 566.0));
    }

    #[test]
    fn a_pane_computes_its_own_grid() {
        let pane = NSRect::new(NSPoint::new(400.0, STRIP_HEIGHT), NSSize::new(400.0, 566.0));
        // 400 - 8 - 10 = 382 points, at scale 2 and an 8-pixel cell that is 95
        // columns; 566 points is 70 rows at 16 pixels.
        assert_eq!(pane_grid(pane, 2.0, (8, 16)), (95, 70));
        assert_eq!(
            pane_grid_rect(pane, 2.0, (8, 16), (95, 70)).origin,
            NSPoint::new(400.0 + PANE_LEFT, STRIP_HEIGHT)
        );
    }

    #[test]
    fn a_split_gives_the_new_pane_the_second_half() {
        let rect = NSRect::new(NSPoint::new(100.0, 34.0), NSSize::new(400.0, 300.0));

        let right = split_second(rect, Axis::Horizontal);
        assert_eq!(right.origin, NSPoint::new(300.0, 34.0));
        assert_eq!(right.size, NSSize::new(200.0, 300.0));

        let down = split_second(rect, Axis::Vertical);
        assert_eq!(down.origin, NSPoint::new(100.0, 184.0));
        assert_eq!(down.size, NSSize::new(400.0, 150.0));
    }

    #[test]
    fn a_divider_is_grabbed_within_its_width() {
        let bounds = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(800.0, 600.0));
        let mut layout = winter::layout::Layout::leaf(PaneId(0));
        assert!(layout.split(PaneId(0), Axis::Horizontal, PaneId(1)));
        let splits = layout.splits();

        let hit = divider_at(bounds, &splits, (401.0, 200.0), 3.0);
        assert_eq!(hit.map(|(_, axis, _)| axis), Some(Axis::Horizontal));
        assert!(divider_at(bounds, &splits, (420.0, 200.0), 3.0).is_none());
        assert!(divider_at(bounds, &splits, (400.0, 10.0), 3.0).is_none(), "the strip");
    }

    #[test]
    fn the_palette_card_is_centred_below_the_strip() {
        let bounds = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(900.0, 600.0));
        let card = palette_rect(bounds, 3);

        assert_eq!(card.origin.x, (900.0 - PALETTE_WIDTH) / 2.0);
        assert_eq!(card.origin.y, STRIP_HEIGHT + 16.0);
        assert_eq!(card.size.width, PALETTE_WIDTH);
        assert_eq!(card.size.height, 2.0 * PALETTE_PAD + PALETTE_FIELD + 3.0 * PALETTE_ROW);
    }

    #[test]
    fn an_empty_palette_keeps_one_line_for_the_notice() {
        let bounds = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(900.0, 600.0));
        assert_eq!(palette_rect(bounds, 0).size.height, palette_rect(bounds, 1).size.height);
    }

    #[test]
    fn a_narrow_window_shrinks_the_palette_card() {
        let bounds = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(200.0, 100.0));
        let card = palette_rect(bounds, 1);

        assert_eq!(card.origin.x, 16.0, "still centred while it fits");
        assert_eq!(card.size.width, 168.0);
    }

    #[test]
    fn the_palette_keeps_the_selected_row_visible() {
        assert_eq!(
            palette_first_visible(3, 2),
            0,
            "a list shorter than the card never scrolls"
        );
        assert_eq!(palette_first_visible(20, 0), 0);
        assert_eq!(palette_first_visible(20, PALETTE_MAX_ROWS - 1), 0);
        assert_eq!(palette_first_visible(20, PALETTE_MAX_ROWS), 1);
        assert_eq!(
            palette_first_visible(20, 19),
            20 - PALETTE_MAX_ROWS,
            "the last row sits at the bottom"
        );
    }

    #[test]
    fn a_palette_click_names_the_row_under_it() {
        let list_top = PALETTE_PAD + PALETTE_FIELD;
        let point = |y: f64| (PALETTE_PAD + 1.0, y);

        assert_eq!(palette_row_at(point(list_top + 1.0), 3, 0), Some(0));
        assert_eq!(palette_row_at(point(list_top + PALETTE_ROW + 1.0), 3, 0), Some(1));
        assert_eq!(palette_row_at(point(list_top - 1.0), 3, 0), None, "the field's row");
        assert_eq!(
            palette_row_at(point(list_top + 3.0 * PALETTE_ROW + 1.0), 3, 0),
            None,
            "past the last row"
        );
        assert_eq!(palette_row_at(point(list_top + 1.0), 20, 5), Some(5), "a scrolled list");
    }
}

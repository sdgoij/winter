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

use winter::scrollbar;
use winter::tabs::{self, TabBox};

use super::icon;

/// The traffic lights occupy this much of the strip's left edge.
///
/// The content view extends under the title bar (`FullSizeContentView`), so the
/// strip has to leave room for the window buttons AppKit draws above it.
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

/// Lays the strip out for a pane `width` points wide.
pub(crate) fn strip(width: f64, count: usize) -> Strip {
    let usable = (width - LEFT_INSET - RIGHT_PAD).max(0.0);
    let reserved = BUTTON + DROP_DOWN;
    let layout = tabs::strip_layout((usable - reserved).max(0.0) as i32, count);
    let offset = LEFT_INSET as i32;

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

/// The whole-cell area the terminal's pixels fill.
pub(crate) fn grid_rect(_bounds: NSRect, scale: f64, cell: (u16, u16), grid: (i32, i32)) -> NSRect {
    let width = f64::from(cell.0) * f64::from(grid.0.max(1)) / scale;
    let height = f64::from(cell.1) * f64::from(grid.1.max(1)) / scale;
    NSRect::new(NSPoint::new(PANE_LEFT, STRIP_HEIGHT), NSSize::new(width, height))
}

/// The find bar's backdrop, behind the field, buttons and count.
///
/// The controls are subviews; this is the panel behind them, so the terminal's
/// text does not show through the count or beside the field. The overlay layer
/// the Metal path draws above its content uses the same geometry.
pub(crate) fn find_backdrop_rect(bounds: NSRect) -> NSRect {
    let width = (bounds.size.width - 16.0).clamp(0.0, 342.0);
    NSRect::new(
        NSPoint::new(bounds.size.width - width - 8.0, STRIP_HEIGHT + 4.0),
        NSSize::new(width, 32.0),
    )
}

/// What the chrome paints from.
pub(crate) struct ChromeState<'a> {
    /// The strip's layout.
    pub(crate) strip: Strip,
    /// The active tab's index.
    pub(crate) active: usize,
    /// The tab titles, in order.
    pub(crate) titles: &'a [String],
    /// The program each tab runs, in order, for the tabs' icons.
    pub(crate) programs: &'a [String],
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
    /// The pane's own fill.
    pub(crate) pane: (u8, u8, u8),
    /// The terminal's pixels.
    pub(crate) frame: Option<&'a crate::frame::Frame>,
    /// Whether to paint the find backdrop and the bell. The Metal path draws
    /// them as layers above its content instead.
    pub(crate) overlays: bool,
    /// The scale the frame was painted at.
    pub(crate) scale: f64,
    /// The grid in cells.
    pub(crate) grid: (i32, i32),
    /// The cell size in physical pixels.
    pub(crate) cell: (u16, u16),
    /// The scrollbar's scroll, document height and visible rows.
    pub(crate) scroll: (i32, i32, i32),
    /// The prompt ticks.
    pub(crate) marks: &'a [winter::grid::MarkTick],
    /// The bell's opacity, while it flashes.
    pub(crate) bell_alpha: Option<f32>,
    /// Whether the find bar's controls are showing.
    pub(crate) find_open: bool,
}

/// Draws the strip, the pane and its scrollbar.
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

    // The pane covers the tabs' overhang, so a tab meets the terminal.
    let pane = pane_rect(bounds);
    fill(pane, state.pane);

    if let Some(frame) = state.frame {
        let rect = grid_rect(bounds, state.scale, state.cell, state.grid);
        frame.draw(NSRect::new(
            rect.origin,
            NSSize::new(
                rect.size.width.min(pane.size.width),
                rect.size.height.min(pane.size.height),
            ),
        ));
    }

    if state.overlays && state.find_open {
        draw_find_backdrop(bounds, state);
    }

    draw_scrollbar(bounds, state);
    if state.overlays {
        draw_bell(pane, state);
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

fn draw_scrollbar(bounds: NSRect, state: &ChromeState<'_>) {
    let (scroll, document, visible) = state.scroll;
    let track = (bounds.size.height - STRIP_HEIGHT).max(0.0);
    if document <= visible || track <= 0.0 {
        return;
    }

    let width = f64::from(scrollbar::WIDTH);
    let left = bounds.size.width - width;
    let track_rect = NSRect::new(NSPoint::new(left, STRIP_HEIGHT), NSSize::new(width, track));
    NSColor::colorWithSRGBRed_green_blue_alpha(0.0, 0.0, 0.0, 0.35).setFill();
    NSBezierPath::fillRect(track_rect);

    let (top, height) = scrollbar::thumb_pixels(scroll, document, visible, track);
    let thumb = NSRect::new(
        NSPoint::new(left + 1.0, STRIP_HEIGHT + top),
        NSSize::new((width - 2.0).max(1.0), height),
    );
    let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(thumb, 2.0, 2.0);
    NSColor::colorWithSRGBRed_green_blue_alpha(0.63, 0.63, 0.63, 0.8).setFill();
    path.fill();

    // Ticks come after the thumb, the way upstream declares its mark image
    // above the scroll bar (ADR 059).
    for mark in state.marks {
        let position = scrollbar::position(mark.row, document);
        let top = STRIP_HEIGHT + f64::from(position) * track - 0.75;
        let rect = NSRect::new(NSPoint::new(left, top), NSSize::new(width, 1.5));
        if mark.error {
            NSColor::colorWithSRGBRed_green_blue_alpha(0.92, 0.35, 0.35, 0.95).setFill();
        } else {
            NSColor::colorWithSRGBRed_green_blue_alpha(0.43, 0.67, 1.0, 0.9).setFill();
        }
        NSBezierPath::fillRect(rect);
    }
}

fn draw_bell(pane: NSRect, state: &ChromeState<'_>) {
    let Some(alpha) = state.bell_alpha else {
        return;
    };
    NSColor::colorWithSRGBRed_green_blue_alpha(1.0, 1.0, 1.0, f64::from(alpha) * 0.12).setFill();
    NSBezierPath::fillRect(pane);
}

fn draw_find_backdrop(bounds: NSRect, state: &ChromeState<'_>) {
    let rect = find_backdrop_rect(bounds);
    let fill_color = state.colors.row.unwrap_or(state.colors.active);
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
    let width = (bounds.size.width - PANE_LEFT - f64::from(scrollbar::WIDTH)).max(0.0);
    let height = (bounds.size.height - STRIP_HEIGHT).max(0.0);
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

    #[test]
    fn the_strip_starts_after_the_traffic_lights() {
        let layout = strip(900.0, 2);
        assert_eq!(layout.tabs[0].left, LEFT_INSET as i32);
        assert_eq!(layout.tabs[0].width(), tabs::TAB_MAX);
        assert_eq!(layout.new_tab.left, layout.tabs[1].right);
        assert_eq!(layout.menu.left, layout.new_tab.right);
        assert_eq!(layout.menu.width(), DROP_DOWN as i32);
    }

    #[test]
    fn a_press_names_its_tab_close_button_or_the_two_buttons() {
        let layout = strip(900.0, 2);
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
        let layout = strip(500.0, 8);
        assert_eq!(layout.tabs[0].width(), tabs::TAB_MIN);
        assert!(layout.menu.right <= 500);
    }

    #[test]
    fn an_empty_strip_keeps_the_new_tab_and_menu_buttons() {
        let layout = strip(900.0, 0);
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
        let layout = strip(900.0, 3);
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
        let layout = strip(900.0, 3);
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
    fn the_find_backdrop_is_a_panel_below_the_strip() {
        let bounds = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(900.0, 600.0));
        let rect = find_backdrop_rect(bounds);

        assert_eq!(rect.origin, NSPoint::new(900.0 - 342.0 - 8.0, STRIP_HEIGHT + 4.0));
        assert_eq!(rect.size, NSSize::new(342.0, 32.0));
    }

    #[test]
    fn a_narrow_window_shrinks_the_find_backdrop() {
        let bounds = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(200.0, 100.0));
        let rect = find_backdrop_rect(bounds);

        assert_eq!(rect.origin, NSPoint::new(8.0, STRIP_HEIGHT + 4.0));
        assert_eq!(rect.size, NSSize::new(184.0, 32.0));
    }
}

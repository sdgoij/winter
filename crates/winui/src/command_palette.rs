//! The command palette's card, drawn over the pane.
//!
//! The entries, filter and selection are `winter::palette`; this module only
//! turns the surviving rows into XAML. The card is an overlay in the pane's
//! `Grid`, like the profile menu, so it needs no new window (ADR 038).

use std::rc::Rc;

use term::input::Key;
use windows_reactor::*;
use winter::palette;

use crate::app::{Message, key_of};

/// The palette's card: a filter field, the matching rows and a notice when
/// nothing matches.
pub(super) fn card(
    filter: &str,
    rows: &[palette::Entry],
    selected: usize,
    field: &ElementRef<TextBox>,
    sender: &LocalSender<Message>,
) -> View {
    let query = sender.clone();
    let entry = TextBox::new()
        .text(filter)
        .placeholder_text("Type a command")
        .element_ref(field)
        .on_text_changed(move |text: Rc<str>| {
            let _ = query.send(Message::PaletteQuery(text.to_string()));
        });

    let mut children: Vec<View> = Vec::with_capacity(rows.len() + 1);
    if rows.is_empty() {
        children.push(
            TextBlock::new()
                .text("No matching commands")
                .font_size(12.0)
                .foreground(Color::rgb(160, 160, 166))
                .margin(Thickness::new(8.0, 6.0, 8.0, 6.0))
                .into(),
        );
    }
    for (index, row) in rows.iter().enumerate() {
        children.push(row_view(row, index, index == selected, sender));
    }

    let list = ScrollViewer::new()
        .horizontal_scroll_bar_visibility(ScrollBarVisibility::Disabled)
        .vertical_scroll_bar_visibility(ScrollBarVisibility::Auto)
        .max_height(320.0)
        .content(StackPanel::new().spacing(2.0).children(children));

    let keys = sender.clone();
    Border::new()
        .background(Color::rgb(38, 38, 42))
        .border_brush(Color::rgb(78, 78, 86))
        .border_thickness(Thickness::uniform(1.0))
        .corner_radius(CornerRadius::uniform(8.0))
        .width(520.0)
        .padding(Thickness::uniform(10.0))
        .horizontal_alignment(HorizontalAlignment::Center)
        .vertical_alignment(VerticalAlignment::Top)
        .margin(Thickness::new(0.0, 40.0, 0.0, 0.0))
        .on_preview_key_down(RoutedCallback::new(move |info: KeyEventInfo| {
            if info.status.is_released {
                return false;
            }
            match key_of(info.key) {
                Some(Key::Escape) => {
                    let _ = keys.send(Message::PaletteClose);
                    true
                }
                Some(Key::Enter) => {
                    let _ = keys.send(Message::PaletteRun);
                    true
                }
                Some(Key::Up) => {
                    let _ = keys.send(Message::PaletteStep(false));
                    true
                }
                Some(Key::Down) => {
                    let _ = keys.send(Message::PaletteStep(true));
                    true
                }
                _ => false,
            }
        }))
        .content(StackPanel::new().spacing(8.0).children(vec![entry.into(), list.into()]))
        .into()
}

/// One row: the command's label, its chord, and the press that runs it.
fn row_view(row: &palette::Entry, index: usize, selected: bool, sender: &LocalSender<Message>) -> View {
    let chord = row.chord.clone().unwrap_or_default();
    let content = Grid::new()
        .columns([GridLength::STAR, GridLength::Auto])
        .column_spacing(12.0)
        .children(vec![
            TextBlock::new()
                .text(row.label.clone())
                .font_size(13.0)
                .grid_column(0)
                .into(),
            TextBlock::new()
                .text(chord)
                .font_size(11.0)
                .foreground(Color::rgb(150, 150, 158))
                .grid_column(1)
                .into(),
        ]);

    let choose = sender.clone();
    Border::new()
        .background(if selected {
            Color::rgb(52, 70, 92)
        } else {
            Color::transparent()
        })
        .corner_radius(CornerRadius::uniform(4.0))
        .padding(Thickness::new(8.0, 5.0, 8.0, 5.0))
        .content(content)
        .on_pointer_pressed(move |info: PointerEventInfo| {
            if info.is_left_button_pressed {
                let _ = choose.send(Message::PaletteChoose(index));
            }
        })
        .into()
}

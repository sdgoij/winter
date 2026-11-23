//! The window commands the shell asks the platform for, from `AppHost.cpp`.
//!
//! egui's viewport commands are this port's only window interface; naming the
//! ones the shell uses keeps `app.rs` from speaking the toolkit's vocabulary for
//! a title, an attention request and a close (ADR 038).

use eframe::egui;

/// Puts `title` in the window's title bar.
pub(crate) fn set_title(ctx: &egui::Context, title: &str) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.to_string()));
}

/// Asks the window manager for the user's attention, as a bell does.
pub(crate) fn request_attention(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
        egui::UserAttentionType::Informational,
    ));
}

/// Closes the window, which is what the last tab closing means.
pub(crate) fn close(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
}

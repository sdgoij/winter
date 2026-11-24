# ADR 071 — The settings dialog's Save and Revert are its own buttons

- Status: Accepted
- Date: 2026-11-24
- Related: `crates/winter/src/winui/settings.rs`,
  `crates/winter/src/winui/app.rs`, ADR 025, ADR 070

## Context

The settings dialog is a `ContentDialog` whose content is a scrolling list of
every setting: the file's status, the defaults, the profiles, the keybindings,
the schemes, the themes and the document preview. Save and Revert were the last
two rows of that list, under an "Apply" heading.

That put the dialog's only two consequential actions at the end of a page
nobody reads to the end: reaching them meant scrolling past every setting, and
a user who did not know they were there saw a dialog with one button, Close.

## Decision

- `Save` and `Revert` are the dialog's `PrimaryButtonText` and
  `SecondaryButtonText`; `Close` stays the close button. They sit in the
  command area the control draws below its content, always visible.
- `is_primary_button_enabled` is the draft being dirty **and** parseable;
  `is_secondary_button_enabled` is the draft being dirty. The flags are
  computed in `view` and passed to the dialog, so the state that disabled the
  old buttons is the state that disables these.
- The dialog's result decides the command: `Primary` is Save, `Secondary` is
  Revert, anything else is a plain close. `command_for` holds that mapping and
  is tested; the binding exposes no per-button click event, only `Closed` with
  the result.
- `on_closed` sends the command first and `SettingsMessage::Close` second,
  because Save reads the draft that Close discards.
- The two notices — the file changed on disk, and unsaved changes are
  discarded — stay in the content, next to the settings they describe.

## Rationale

A dialog's buttons belong in the dialog's button area. The old placement was
not wrong about *what* the buttons do, only about where a control that ends the
dialog is expected to be.

Keeping the enablement flags means a save that cannot be written is still
unpressable rather than failing after the dialog is gone, which is what the old
buttons did.

## Consequences

- Pressing Save or Revert now closes the dialog, because a ContentDialog closes
  on a primary or secondary click. Before, Save left the window open on the
  reloaded draft. Closing is the expected behaviour for a dialog button, and
  the state that mattered — the file written, the shell reloaded — is unchanged.
- Save cannot be pressed while a value does not parse, so the "blocked" state is
  visible on the button rather than only in the preview's message.
- The content's `max_height` (420) no longer has to leave room for a button row
  inside the scroller; it still leaves room for the dialog's command area.
- A future third command has nowhere to go: `ContentDialog` has exactly three
  buttons, and the third is Close.

## Alternatives considered

- **Keep the buttons in the content and pin the row to the bottom with its own
  `Border`.** It would need the scroller to shrink and the row to sit outside
  it, which is what the dialog's command area already is.
- **Use `PrimaryButtonCommand` with a command object.** The binding exposes no
  command type; `Closed` and its result are the whole surface.
- **Make Save the default button so Enter saves.** Enter is how a user leaves a
  text field in the content; a default of Save would write the file from a
  keystroke aimed at an edit box.

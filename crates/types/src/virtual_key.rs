//! The virtual-key codes an `INPUT_RECORD` carries.
//!
//! Values from `winuser.h`. They travel verbatim in win32-input-mode, so they
//! are part of the protocol a client reads rather than a local convenience.

/// Backspace.
pub const VK_BACK: u16 = 0x08;
/// Tab.
pub const VK_TAB: u16 = 0x09;
/// Enter.
pub const VK_RETURN: u16 = 0x0D;
/// Shift.
pub const VK_SHIFT: u16 = 0x10;
/// Control.
pub const VK_CONTROL: u16 = 0x11;
/// Alt.
pub const VK_MENU: u16 = 0x12;
/// Pause.
pub const VK_PAUSE: u16 = 0x13;
/// Caps Lock.
pub const VK_CAPITAL: u16 = 0x14;
/// Escape.
pub const VK_ESCAPE: u16 = 0x1B;
/// Page up.
pub const VK_PRIOR: u16 = 0x21;
/// Page down.
pub const VK_NEXT: u16 = 0x22;
/// End.
pub const VK_END: u16 = 0x23;
/// Home.
pub const VK_HOME: u16 = 0x24;
/// Left arrow.
pub const VK_LEFT: u16 = 0x25;
/// Up arrow.
pub const VK_UP: u16 = 0x26;
/// Right arrow.
pub const VK_RIGHT: u16 = 0x27;
/// Down arrow.
pub const VK_DOWN: u16 = 0x28;
/// Print Screen.
pub const VK_SNAPSHOT: u16 = 0x2C;
/// Insert.
pub const VK_INSERT: u16 = 0x2D;
/// Delete.
pub const VK_DELETE: u16 = 0x2E;
/// Left Windows key.
pub const VK_LWIN: u16 = 0x5B;
/// Right Windows key.
pub const VK_RWIN: u16 = 0x5C;
/// The first function key; the rest are contiguous.
pub const VK_F1: u16 = 0x70;
/// The second function key.
pub const VK_F2: u16 = 0x71;
/// The third function key.
pub const VK_F3: u16 = 0x72;
/// The fourth function key.
pub const VK_F4: u16 = 0x73;
/// The fifth function key.
pub const VK_F5: u16 = 0x74;
/// The sixth function key.
pub const VK_F6: u16 = 0x75;
/// The seventh function key.
pub const VK_F7: u16 = 0x76;
/// The eighth function key.
pub const VK_F8: u16 = 0x77;
/// The ninth function key.
pub const VK_F9: u16 = 0x78;
/// The tenth function key.
pub const VK_F10: u16 = 0x79;
/// The eleventh function key.
pub const VK_F11: u16 = 0x7A;
/// The twelfth function key.
pub const VK_F12: u16 = 0x7B;
/// Num Lock.
pub const VK_NUMLOCK: u16 = 0x90;
/// Scroll Lock.
pub const VK_SCROLL: u16 = 0x91;
/// Left Shift.
pub const VK_LSHIFT: u16 = 0xA0;
/// Right Shift.
pub const VK_RSHIFT: u16 = 0xA1;
/// Left Control.
pub const VK_LCONTROL: u16 = 0xA2;
/// Right Control.
pub const VK_RCONTROL: u16 = 0xA3;
/// Left Alt.
pub const VK_LMENU: u16 = 0xA4;
/// Right Alt.
pub const VK_RMENU: u16 = 0xA5;

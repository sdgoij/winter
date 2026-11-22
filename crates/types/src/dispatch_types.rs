//! VT parameter types, enums, and constants.
//!
//! Ported from `DispatchTypes.hpp` in the Windows Terminal C++ codebase.
//! Contains the core data types used throughout the VT state machine.

/// VT parameter integer type (i32 to match C++ VTInt).
pub type VTInt = i32;
pub type VTCoord = i32; // Alias for coordinate types

/// VT parameter (non-negative integer).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VTParameter {
    value: Option<VTInt>,
}

impl VTParameter {
    /// Creates an empty (unset) parameter.
    pub const fn new() -> Self {
        Self { value: None }
    }

    /// Creates a parameter with a value.
    pub const fn from(value: VTInt) -> Self {
        Self { value: Some(value) }
    }

    /// Returns whether this parameter has a value.
    pub const fn has_value(&self) -> bool {
        self.value.is_some()
    }

    /// Returns the value, or 0 if not set.
    pub const fn value_or(&self, default: VTInt) -> VTInt {
        if let Some(v) = self.value { v } else { default }
    }

    /// Returns the value.
    /// # Panics
    /// Panics if the parameter has no value.
    pub const fn value(&self) -> VTInt {
        self.value.expect("VTParameter has no value")
    }
}

impl Default for VTParameter {
    fn default() -> Self {
        Self::new()
    }
}

/// Sub-parameters of a single VT parameter.
///
/// Ported from `VTSubParameters` in `DispatchTypes.hpp`; `at` returns an
/// unset parameter past the end so callers never index out of bounds.
#[derive(Debug, Clone, Copy)]
pub struct VTSubParameters<'a> {
    sub_params: &'a [VTParameter],
}

impl<'a> VTSubParameters<'a> {
    /// Creates a new view over sub-parameters.
    pub const fn new(sub_params: &'a [VTParameter]) -> Self {
        Self { sub_params }
    }

    /// Returns the sub-parameter at the given index.
    pub fn at(&self, index: usize) -> VTParameter {
        self.sub_params.get(index).copied().unwrap_or_default()
    }

    /// Returns the number of sub-parameters.
    pub fn len(&self) -> usize {
        self.sub_params.len()
    }

    /// Returns whether there are no sub-parameters.
    pub fn is_empty(&self) -> bool {
        self.sub_params.is_empty()
    }

    /// Iterates over the sub-parameters.
    pub fn iter(&self) -> impl Iterator<Item = &'a VTParameter> {
        self.sub_params.iter()
    }
}

/// VT parameters collection with optional sub-parameters.
#[derive(Debug, Clone)]
pub struct VTParameters {
    params: Vec<VTParameter>,
    /// Sub-parameters in one flat buffer.
    sub_params: Vec<VTParameter>,
    /// One `(start, end)` range into `sub_params` for each parameter.
    sub_param_ranges: Vec<(u8, u8)>,
}

/// An unset parameter, used when an empty list is treated as one parameter.
const DEFAULT_PARAMETER: VTParameter = VTParameter::new();

impl VTParameters {
    /// Creates empty parameters.
    pub fn new() -> Self {
        Self {
            params: Vec::new(),
            sub_params: Vec::new(),
            sub_param_ranges: Vec::new(),
        }
    }

    /// Creates parameters from raw values.
    pub fn from_values(values: Vec<VTInt>) -> Self {
        Self {
            params: values.into_iter().map(VTParameter::from).collect(),
            sub_params: Vec::new(),
            sub_param_ranges: Vec::new(),
        }
    }

    /// Creates parameters from the parser's flat buffers.
    pub fn from_parts(params: Vec<VTParameter>, sub_params: Vec<VTParameter>, sub_param_ranges: Vec<(u8, u8)>) -> Self {
        Self {
            params,
            sub_params,
            sub_param_ranges,
        }
    }

    /// Appends a parameter.
    pub fn push(&mut self, param: VTParameter) {
        self.params.push(param);
    }

    /// Appends multiple parameters.
    pub fn extend(&mut self, params: impl IntoIterator<Item = VTParameter>) {
        self.params.extend(params);
    }

    /// Returns whether there are no parameters.
    pub fn is_empty(&self) -> bool {
        self.params.is_empty()
    }

    /// Returns the number of parameters actually collected.
    pub fn len(&self) -> usize {
        self.params.len()
    }

    /// Returns the effective parameter count.
    ///
    /// Ported from `VTParameters::size`: an empty list is equivalent to a
    /// single unspecified parameter, so `ESC [ m` resets the attributes.
    pub fn size(&self) -> usize {
        self.params.len().max(1)
    }

    /// Returns the parameter at the given index.
    pub fn at(&self, index: usize) -> VTParameter {
        self.params.get(index).copied().unwrap_or(VTParameter::default())
    }

    /// Returns the sub-parameters for the parameter at the given index.
    pub fn sub_params_for(&self, index: usize) -> Option<VTSubParameters<'_>> {
        let (start, end) = *self.sub_param_ranges.get(index)?;
        if end <= start {
            return None;
        }
        Some(VTSubParameters::new(&self.sub_params[start as usize..end as usize]))
    }

    /// Returns whether the parameter at the given index has sub-parameters.
    pub fn has_sub_params_for(&self, index: usize) -> bool {
        self.sub_param_ranges
            .get(index)
            .is_some_and(|&(start, end)| end > start)
    }

    /// Returns whether any parameter has sub-parameters.
    pub fn has_sub_params(&self) -> bool {
        !self.sub_params.is_empty()
    }

    /// Iterates over all parameters.
    ///
    /// An empty list yields one unspecified parameter, matching `for_each`.
    pub fn iter(&self) -> impl Iterator<Item = VTParameter> + '_ {
        let params: &[VTParameter] = if self.params.is_empty() {
            std::slice::from_ref(&DEFAULT_PARAMETER)
        } else {
            &self.params
        };
        params.iter().copied()
    }

    /// Returns the raw slice of parameters.
    pub fn as_slice(&self) -> &[VTParameter] {
        &self.params
    }
}

impl Default for VTParameters {
    fn default() -> Self {
        Self::new()
    }
}

impl From<Vec<VTInt>> for VTParameters {
    fn from(values: Vec<VTInt>) -> Self {
        Self::from_values(values)
    }
}

impl From<VTInt> for VTParameters {
    fn from(value: VTInt) -> Self {
        Self::from_values(vec![value])
    }
}

/// VT ID (intermediate bytes plus the final byte) for CSI/DCS/ESC sequences.
///
/// Ported from `VTID` in `DispatchTypes.hpp`: the bytes are the collected
/// intermediates followed by the final character, so `ESC ( 0` finalizes to
/// `["(", "0"]`. At most six intermediates are kept; an overflow resets the
/// identifier, whose zeroed bytes match no handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VTID {
    bytes: [u8; Self::MAX_BYTES],
    len: usize,
}

impl VTID {
    /// Maximum number of bytes (six intermediates plus the final byte).
    pub const MAX_BYTES: usize = 7;
    /// Maximum number of intermediates before the identifier is reset.
    const MAX_INTERMEDIATES: usize = 6;

    /// Creates an empty VTID.
    pub const fn new() -> Self {
        Self {
            bytes: [0; Self::MAX_BYTES],
            len: 0,
        }
    }

    /// Creates a VTID holding a single byte.
    pub const fn from_byte(byte: u8) -> Self {
        let mut id = Self::new();
        id.bytes[0] = byte;
        id.len = 1;
        id
    }

    /// Creates a VTID from the bytes of a completed sequence.
    ///
    /// The bytes are the collected intermediates followed by the final byte;
    /// bytes past [`Self::MAX_BYTES`] are ignored.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let mut id = Self::new();
        for &byte in bytes.iter().take(Self::MAX_BYTES) {
            id.bytes[id.len] = byte;
            id.len += 1;
        }
        id
    }

    /// Adds an intermediate byte.
    pub fn with_intermediate(mut self, byte: u8) -> Self {
        if self.len >= Self::MAX_INTERMEDIATES {
            // Upstream zeroes the accumulator but keeps the shift, so the
            // overflowed identifier is six zero bytes plus the final byte.
            // A zeroed id matches no handler.
            self.bytes = [0; Self::MAX_BYTES];
            self.len = Self::MAX_INTERMEDIATES;
            return self;
        }
        self.bytes[self.len] = byte;
        self.len += 1;
        self
    }

    /// Appends the final byte and returns the identifier.
    ///
    /// Ported from `VTIDBuilder::Finalize`.
    pub fn finalize_with(mut self, final_char: u8) -> Self {
        if self.len >= Self::MAX_BYTES {
            self = Self::new();
        }
        self.bytes[self.len] = final_char;
        self.len += 1;
        self
    }

    /// Returns the byte at the given index, or 0 past the end.
    pub fn byte_at(&self, index: usize) -> u8 {
        self.bytes.get(index).copied().unwrap_or(0)
    }

    /// Returns the identifier starting at `offset`.
    ///
    /// Ported from `VTID::SubSequence`.
    pub fn subsequence(&self, offset: usize) -> Self {
        if offset >= self.len {
            return Self::new();
        }
        let mut id = Self::new();
        id.bytes[..self.len - offset].copy_from_slice(&self.bytes[offset..self.len]);
        id.len = self.len - offset;
        id
    }

    /// Returns the number of bytes.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns whether the identifier is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns the bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

/// Erase type for ED/EL commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EraseType {
    /// Erase to end of line/screen.
    ToEnd,
    /// Erase from beginning of line/screen.
    FromBeginning,
    /// Erase entire line/screen.
    All,
    /// Erase scrollback buffer.
    Scrollback,
}

/// Change extent for rectangular attribute changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeExtent {
    /// Default (stream) extent.
    Default,
    /// Stream extent.
    Stream,
    /// Rectangle extent.
    Rectangle,
}

/// Taskbar state for Windows-specific actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskbarState {
    /// Clear taskbar indicator.
    Clear,
    /// Set taskbar indicator.
    Set,
    /// Error state.
    Error,
    /// Indeterminate progress.
    Indeterminate,
    /// Paused.
    Paused,
}

/// Graphics options for SGR.
///
/// Ported from `DispatchTypes::GraphicsOptions`. Values 2 and 5 are dual-purpose:
/// they select the extended-colour syntax when followed by a colour space, and
/// mean faint/blink otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum GraphicsOptions {
    Off = 0,
    Intense = 1,
    /// 2 is also faint (decreased intensity, ISO 6429).
    RGBColorOrFaint = 2,
    Italics = 3,
    /// 4 is also the extended underline-style form `SGR 4:x`.
    Underline = 4,
    /// 5 is also the xterm-256 index in `38;5:n`.
    BlinkOrXterm256Index = 5,
    RapidBlink = 6,
    Negative = 7,
    Invisible = 8,
    CrossedOut = 9,
    DoublyUnderlined = 21,
    NotIntenseOrFaint = 22,
    NotItalics = 23,
    NoUnderline = 24,
    Steady = 25,
    Positive = 27,
    Visible = 28,
    NotCrossedOut = 29,
    ForegroundBlack = 30,
    ForegroundRed = 31,
    ForegroundGreen = 32,
    ForegroundYellow = 33,
    ForegroundBlue = 34,
    ForegroundMagenta = 35,
    ForegroundCyan = 36,
    ForegroundWhite = 37,
    ForegroundExtended = 38,
    ForegroundDefault = 39,
    BackgroundBlack = 40,
    BackgroundRed = 41,
    BackgroundGreen = 42,
    BackgroundYellow = 43,
    BackgroundBlue = 44,
    BackgroundMagenta = 45,
    BackgroundCyan = 46,
    BackgroundWhite = 47,
    BackgroundExtended = 48,
    BackgroundDefault = 49,
    Overline = 53,
    NoOverline = 55,
    UnderlineColor = 58,
    UnderlineColorDefault = 59,
    BrightForegroundBlack = 90,
    BrightForegroundRed = 91,
    BrightForegroundGreen = 92,
    BrightForegroundYellow = 93,
    BrightForegroundBlue = 94,
    BrightForegroundMagenta = 95,
    BrightForegroundCyan = 96,
    BrightForegroundWhite = 97,
    BrightBackgroundBlack = 100,
    BrightBackgroundRed = 101,
    BrightBackgroundGreen = 102,
    BrightBackgroundYellow = 103,
    BrightBackgroundBlue = 104,
    BrightBackgroundMagenta = 105,
    BrightBackgroundCyan = 106,
    BrightBackgroundWhite = 107,
}

impl GraphicsOptions {
    /// Decodes an SGR option, or `None` for a value no handler owns.
    pub const fn from_int(value: VTInt) -> Option<Self> {
        Some(match value {
            0 => Self::Off,
            1 => Self::Intense,
            2 => Self::RGBColorOrFaint,
            3 => Self::Italics,
            4 => Self::Underline,
            5 => Self::BlinkOrXterm256Index,
            6 => Self::RapidBlink,
            7 => Self::Negative,
            8 => Self::Invisible,
            9 => Self::CrossedOut,
            21 => Self::DoublyUnderlined,
            22 => Self::NotIntenseOrFaint,
            23 => Self::NotItalics,
            24 => Self::NoUnderline,
            25 => Self::Steady,
            27 => Self::Positive,
            28 => Self::Visible,
            29 => Self::NotCrossedOut,
            30..=37 => Self::foreground(value),
            38 => Self::ForegroundExtended,
            39 => Self::ForegroundDefault,
            40..=47 => Self::background(value),
            48 => Self::BackgroundExtended,
            49 => Self::BackgroundDefault,
            53 => Self::Overline,
            55 => Self::NoOverline,
            58 => Self::UnderlineColor,
            59 => Self::UnderlineColorDefault,
            90..=97 => Self::bright_foreground(value),
            100..=107 => Self::bright_background(value),
            _ => return None,
        })
    }

    const fn foreground(value: VTInt) -> Self {
        match value {
            30 => Self::ForegroundBlack,
            31 => Self::ForegroundRed,
            32 => Self::ForegroundGreen,
            33 => Self::ForegroundYellow,
            34 => Self::ForegroundBlue,
            35 => Self::ForegroundMagenta,
            36 => Self::ForegroundCyan,
            _ => Self::ForegroundWhite,
        }
    }

    const fn background(value: VTInt) -> Self {
        match value {
            40 => Self::BackgroundBlack,
            41 => Self::BackgroundRed,
            42 => Self::BackgroundGreen,
            43 => Self::BackgroundYellow,
            44 => Self::BackgroundBlue,
            45 => Self::BackgroundMagenta,
            46 => Self::BackgroundCyan,
            _ => Self::BackgroundWhite,
        }
    }

    const fn bright_foreground(value: VTInt) -> Self {
        match value {
            90 => Self::BrightForegroundBlack,
            91 => Self::BrightForegroundRed,
            92 => Self::BrightForegroundGreen,
            93 => Self::BrightForegroundYellow,
            94 => Self::BrightForegroundBlue,
            95 => Self::BrightForegroundMagenta,
            96 => Self::BrightForegroundCyan,
            _ => Self::BrightForegroundWhite,
        }
    }

    const fn bright_background(value: VTInt) -> Self {
        match value {
            100 => Self::BrightBackgroundBlack,
            101 => Self::BrightBackgroundRed,
            102 => Self::BrightBackgroundGreen,
            103 => Self::BrightBackgroundYellow,
            104 => Self::BrightBackgroundBlue,
            105 => Self::BrightBackgroundMagenta,
            106 => Self::BrightBackgroundCyan,
            _ => Self::BrightBackgroundWhite,
        }
    }
}

/// DECSCA logical attribute options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LogicalAttributeOptions {
    Default = 0,
    Protected = 1,
    Unprotected = 2,
}

/// Underline style constants.
pub const SINGLE_UNDERLINE: u8 = 4;

/// Additional underline constants.
pub const UNDERLINE_NONE: u8 = 24;

/// Additional underline-related constants (not in GraphicsOptions enum).
pub const OVERLINE_NONE: u8 = 55;

/// SGR save/restore stack options (XTPUSHSGR).
///
/// Ported from `DispatchTypes::SgrSaveRestoreStackOptions`. The values are xterm
/// parameter numbers, not ordinals: 7 is inverse, 21 is doubly underlined, 30/31
/// save the foreground/background colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SgrSaveRestoreStackOptions {
    All = 0,
    Intense = 1,
    Faintness = 2,
    Italics = 3,
    Underline = 4,
    Blink = 5,
    Negative = 7,
    Invisible = 8,
    CrossedOut = 9,
    DoublyUnderlined = 21,
    SaveForegroundColor = 30,
    SaveBackgroundColor = 31,
}

impl SgrSaveRestoreStackOptions {
    /// The highest option value (upstream's `Max` alias for the last entry).
    pub const MAX: i32 = Self::SaveBackgroundColor as i32;
}

/// Status types for DSR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusType {
    /// Operating status (DSR).
    OperatingStatus,
    /// Cursor position report (CPR).
    CursorPositionReport,
    /// Extended cursor position report.
    ExtendedCursorPositionReport,
    /// Printer status.
    PrinterStatus,
    /// User-defined keys.
    UserDefinedKeys,
    /// Keyboard status.
    KeyboardStatus,
    /// Locator status.
    LocatorStatus,
    /// Locator identity.
    LocatorIdentity,
    /// Macro space report.
    MacroSpaceReport,
    /// Memory checksum.
    MemoryChecksum,
    /// Data integrity.
    DataIntegrity,
    /// Multiple session status.
    MultipleSessionStatus,
}

/// Bit that distinguishes a DEC private mode from an ANSI mode of the same number.
///
/// Ported from `DECPrivateStatus` / `DECPrivateMode` in `DispatchTypes.hpp`:
/// `DECSET ?1h` and `SM 1h` are different modes, and flattening the two onto one
/// number gives DECCKM and DECARM the same value as modes that mean something
/// else.
pub const DEC_PRIVATE_MODE: VTInt = 0x0100_0000;

/// Returns the DEC-private form of a mode number.
pub const fn dec_private_mode(number: VTInt) -> VTInt {
    DEC_PRIVATE_MODE | number
}

/// Returns the ANSI form of a mode number.
pub const fn ansi_standard_mode(number: VTInt) -> VTInt {
    number
}

/// Mode parameters for SM / DECSET / RM / DECRST / DECRQM.
///
/// Ported from `DispatchTypes::ModeParams`, values included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ModeParams {
    Irm = ansi_standard_mode(4),
    Lnm = ansi_standard_mode(20),
    DecCkm = dec_private_mode(1),
    DecAnm = dec_private_mode(2),
    DecColm = dec_private_mode(3),
    DecScnm = dec_private_mode(5),
    DecOm = dec_private_mode(6),
    DecAwm = dec_private_mode(7),
    DecArm = dec_private_mode(8),
    Att610StartCursorBlink = dec_private_mode(12),
    DecTcem = dec_private_mode(25),
    EnableDecColmSupport = dec_private_mode(40),
    DecPccm = dec_private_mode(64),
    DecNkm = dec_private_mode(66),
    DecBkm = dec_private_mode(67),
    DecLrmm = dec_private_mode(69),
    DecSdm = dec_private_mode(80),
    DecEcm = dec_private_mode(117),
    Vt200MouseMode = dec_private_mode(1000),
    ButtonEventMouseMode = dec_private_mode(1002),
    AnyEventMouseMode = dec_private_mode(1003),
    FocusEventMode = dec_private_mode(1004),
    Utf8ExtendedMode = dec_private_mode(1005),
    SgrExtendedMode = dec_private_mode(1006),
    AlternateScroll = dec_private_mode(1007),
    AlternateScreenBuffer = dec_private_mode(1049),
    BracketedPasteMode = dec_private_mode(2004),
    SynchronizedOutput = dec_private_mode(2026),
    GraphemeClusterMode = dec_private_mode(2027),
    Win32InputMode = dec_private_mode(9001),
}

impl ModeParams {
    /// Returns the raw mode value, DEC-private tag included.
    pub const fn raw(self) -> VTInt {
        self as VTInt
    }

    /// Returns the mode number as it appears on the wire, tag removed.
    pub const fn param_number(self) -> VTInt {
        self.raw() & !DEC_PRIVATE_MODE
    }

    /// Returns whether this is a DEC private mode (`CSI ? … h`).
    pub const fn is_private(self) -> bool {
        self.raw() & DEC_PRIVATE_MODE != 0
    }
}

/// Mode response values for DECRQM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeResponses {
    /// Unrecognized.
    Unrecognized = 0,
    /// Enabled.
    Enabled = 1,
    /// Disabled.
    Disabled = 2,
    /// Permanently enabled.
    PermanentlyEnabled = 3,
    /// Permanently disabled.
    PermanentlyDisabled = 4,
}

/// Character set identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharacterSets {
    /// DEC Special Character Set and Graphics.
    DecSpecialGraphics,
    /// ASCII.
    Ascii,
}

/// Coding system identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodingSystem {
    /// ISO 2022.
    Iso2022,
    /// UTF-8.
    Utf8,
}

/// Tab clear type for TBC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabClearType {
    /// Clear tab stop at current column.
    ClearCurrentColumn,
    /// Clear all tab stops.
    ClearAllColumns,
}

/// Tab set type for DECST8C.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabSetType {
    /// Set tab stops every 8 columns.
    SetEvery8Columns,
}

/// Window manipulation types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowManipulationType {
    /// Invalid.
    Invalid,
    /// De-iconify window.
    DeIconifyWindow,
    /// Iconify window.
    IconifyWindow,
    /// Refresh window.
    RefreshWindow,
    /// Resize window in characters.
    ResizeWindowInCharacters,
    /// Report text size in pixels.
    ReportTextSizeInPixels,
    /// Report character cell size.
    ReportCharacterCellSize,
    /// Report text size in characters.
    ReportTextSizeInCharacters,
}

/// Cursor styles for DECSCUSR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorStyle {
    /// User default.
    UserDefault = 0,
    /// Blinking block.
    BlinkingBlock = 1,
    /// Steady block.
    SteadyBlock = 2,
    /// Blinking underline.
    BlinkingUnderline = 3,
    /// Steady underline.
    SteadyUnderline = 4,
    /// Blinking bar (narrow).
    BlinkingBar = 5,
    /// Steady bar (narrow).
    SteadyBar = 6,
}

/// Reporting permission for DECREQTPARM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportingPermission {
    /// Unsolicited reports allowed.
    Unsolicited,
    /// Solicited reports only.
    Solicited,
}

/// Line feed type for IND/VT/LF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineFeedType {
    /// With return to first column.
    WithReturn,
    /// Without return.
    WithoutReturn,
    /// Depends on LNM mode.
    DependsOnMode,
}

/// Sixel background selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SixelBackground {
    /// Default background.
    Default,
    /// Transparent background.
    Transparent,
    /// Opaque background.
    Opaque,
}

/// DRCS erase control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrcsEraseControl {
    /// Erase all characters.
    AllChars,
    /// Erase reloaded characters.
    ReloadedChars,
    /// Erase all renditions.
    AllRenditions,
}

/// DRCS cell matrix size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrcsCellMatrix {
    /// Default size.
    Default,
    /// Invalid size.
    Invalid,
    /// 5x10 matrix.
    Size5x10,
    /// 6x10 matrix.
    Size6x10,
    /// 7x10 matrix.
    Size7x10,
}

/// DRCS font set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrcsFontSet {
    /// Default font set.
    Default,
    /// 80x24 font set.
    Size80x24,
    /// 132x24 font set.
    Size132x24,
    /// 80x36 font set.
    Size80x36,
    /// 132x36 font set.
    Size132x36,
    /// 80x48 font set.
    Size80x48,
    /// 132x48 font set.
    Size132x48,
}

/// DRCS font usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrcsFontUsage {
    /// Default usage.
    Default,
    /// Text usage.
    Text,
    /// Full cell usage.
    FullCell,
}

/// Charset size for DECAUPSS/DECRQUPSS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharsetSize {
    /// 94-character set.
    Size94,
    /// 96-character set.
    Size96,
}

/// Macro delete control for DECDMAC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacroDeleteControl {
    /// Delete specific macro ID.
    DeleteId,
    /// Delete all macros.
    DeleteAll,
}

/// Macro encoding for DECDMAC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacroEncoding {
    /// Text encoding.
    Text,
    /// Hex pair encoding.
    HexPair,
}

/// Report format for DECRQTSR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportFormat {
    /// Terminal state report.
    TerminalStateReport,
    /// Color table report.
    ColorTableReport,
}

/// Presentation report format for DECRQPSR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentationReportFormat {
    /// Cursor information report.
    CursorInformationReport,
    /// Tabulation stop report.
    TabulationStopReport,
}

/// Color item for DECAC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorItem {
    /// Normal text.
    NormalText,
    /// Window frame.
    WindowFrame,
}

/// Color model for DECAC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorModel {
    /// HLS (Hue, Lightness, Saturation).
    Hls,
    /// RGB.
    Rgb,
}

impl ColorModel {
    /// Decodes the wire value used by DECRQTSR/DECRSTS: 1 is HLS, 2 is RGB.
    pub const fn from_int(value: VTInt) -> Option<Self> {
        match value {
            1 => Some(Self::Hls),
            2 => Some(Self::Rgb),
            _ => None,
        }
    }

    /// The wire value for this model.
    pub const fn to_int(self) -> VTInt {
        match self {
            Self::Hls => 1,
            Self::Rgb => 2,
        }
    }
}

/// Default column counts for DECCOLM.
pub const DECCOLM_SET_COLUMNS: VTInt = 132;
pub const DECCOLM_RESET_COLUMNS: VTInt = 80;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vt_parameter_default() {
        let param = VTParameter::default();
        assert!(!param.has_value());
        assert_eq!(param.value_or(42), 42);
    }

    #[test]
    fn test_vt_parameter_with_value() {
        let param = VTParameter::from(123);
        assert!(param.has_value());
        assert_eq!(param.value(), 123);
        assert_eq!(param.value_or(0), 123);
    }

    #[test]
    fn test_vt_parameters_empty() {
        let params = VTParameters::new();
        assert!(params.is_empty());
        assert_eq!(params.len(), 0);
        assert_eq!(params.at(0).value_or(0), 0);
    }

    #[test]
    fn test_vt_parameters_from_values() {
        let params = VTParameters::from(vec![1, 2, 3]);
        assert_eq!(params.len(), 3);
        assert_eq!(params.at(0).value(), 1);
        assert_eq!(params.at(1).value(), 2);
        assert_eq!(params.at(2).value(), 3);
    }

    #[test]
    fn test_vtid_empty() {
        let vtid = VTID::new();
        assert!(vtid.is_empty());
        assert_eq!(vtid.len(), 0);
    }

    #[test]
    fn test_vtid_with_bytes() {
        let vtid = VTID::new()
            .with_intermediate(b'A')
            .with_intermediate(b'B')
            .finalize_with(b'C');
        assert!(!vtid.is_empty());
        assert_eq!(vtid.len(), 3);
        // Bytes are stored in collection order, intermediates then final.
        assert_eq!(vtid.byte_at(0), b'A');
        assert_eq!(vtid.byte_at(1), b'B');
        assert_eq!(vtid.byte_at(2), b'C');
        assert_eq!(vtid.as_bytes(), b"ABC");
    }

    #[test]
    fn test_vtid_subsequence() {
        let vtid = VTID::new().with_intermediate(b'(').finalize_with(b'0');
        assert_eq!(vtid.subsequence(1).as_bytes(), b"0");
        assert!(vtid.subsequence(2).is_empty());
    }

    #[test]
    fn test_vtid_intermediate_overflow_resets() {
        let mut vtid = VTID::new();
        for _ in 0..6 {
            vtid = vtid.with_intermediate(b'#');
        }
        // The seventh intermediate cannot leave room for the final byte, so
        // the accumulator is zeroed and the final goes in the last slot.
        vtid = vtid.with_intermediate(b'#');
        assert_eq!(vtid.as_bytes(), &[0, 0, 0, 0, 0, 0]);
        let vtid = vtid.finalize_with(b'X');
        assert_eq!(vtid.len(), 7);
        assert_eq!(vtid.byte_at(0), 0);
        assert_eq!(vtid.byte_at(6), b'X');
    }

    #[test]
    fn test_erase_type() {
        assert_eq!(format!("{:?}", EraseType::ToEnd), "ToEnd");
        assert_eq!(format!("{:?}", EraseType::Scrollback), "Scrollback");
    }

    #[test]
    fn test_cursor_style() {
        assert_eq!(CursorStyle::BlinkingBlock as u8, 1);
        assert_eq!(CursorStyle::SteadyBlock as u8, 2);
        assert_eq!(CursorStyle::BlinkingUnderline as u8, 3);
    }
}

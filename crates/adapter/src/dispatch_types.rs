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

/// Sub-parameters span for a single VT parameter.
#[derive(Debug, Clone)]
pub struct VTSubParameters<'a> {
    sub_params: std::slice::Iter<'a, VTParameter>,
}

impl<'a> VTSubParameters<'a> {
    /// Creates a new VTSubParameters from an iterator.
    pub fn new(params: std::slice::Iter<'a, VTParameter>) -> Self {
        Self { sub_params: params }
    }

    /// Returns the next sub-parameter.
    pub fn next_param(&mut self) -> Option<&'a VTParameter> {
        self.sub_params.next()
    }

    /// Returns whether there are more sub-parameters.
    pub fn has_more(&self) -> bool {
        self.sub_params.clone().next().is_some()
    }
}

/// VT parameters collection with optional sub-parameters.
#[derive(Debug, Clone)]
pub struct VTParameters {
    params: Vec<VTParameter>,
    /// Ranges where sub-parameters exist: (start_index, count)
    sub_param_ranges: Vec<(usize, usize)>,
}

impl VTParameters {
    /// Creates empty parameters.
    pub fn new() -> Self {
        Self {
            params: Vec::new(),
            sub_param_ranges: Vec::new(),
        }
    }

    /// Creates parameters from raw values.
    pub fn from_values(values: Vec<VTInt>) -> Self {
        Self {
            params: values.into_iter().map(VTParameter::from).collect(),
            sub_param_ranges: Vec::new(),
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

    /// Returns the number of parameters.
    pub fn len(&self) -> usize {
        self.params.len()
    }

    /// Returns the parameter at the given index.
    pub fn at(&self, index: usize) -> VTParameter {
        self.params.get(index).copied().unwrap_or(VTParameter::default())
    }

    /// Returns the sub-parameters for the parameter at the given index.
    pub fn sub_params_for(&self, index: usize) -> Option<VTSubParameters<'_>> {
        let (start, count) = self.sub_param_ranges.iter().find(|(idx, _)| *idx == index)?;
        if *count == 0 {
            return None;
        }
        let start = *start;
        let count = *count;
        if start + count > self.params.len() {
            return None;
        }
        Some(VTSubParameters::new(self.params[start..start + count].iter()))
    }

    /// Returns whether the parameter at the given index has sub-parameters.
    pub fn has_sub_params_for(&self, index: usize) -> bool {
        self.sub_params_for(index).is_some_and(|sp| sp.has_more())
    }

    /// Iterates over all parameters.
    pub fn iter(&self) -> impl Iterator<Item = VTParameter> + '_ {
        self.params.iter().copied()
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

/// VT ID (intermediate bytes) for CSI/DSC/ESC sequences.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VTID {
    bytes: [u8; Self::MAX_INTERMEDIATES],
    len: usize,
}

impl VTID {
    /// Maximum number of intermediate bytes.
    pub const MAX_INTERMEDIATES: usize = 8;

    /// Creates an empty VTID.
    pub const fn new() -> Self {
        Self {
            bytes: [0; Self::MAX_INTERMEDIATES],
            len: 0,
        }
    }

    /// Adds an intermediate byte.
    pub fn with_intermediate(mut self, byte: u8) -> Self {
        if self.len < Self::MAX_INTERMEDIATES {
            self.bytes[self.len] = byte;
            self.len += 1;
        }
        self
    }

    /// Finalizes and returns the VTID.
    pub fn finalize(self) -> Self {
        self
    }

    /// Returns the byte at the given index.
    pub fn byte_at(&self, index: usize) -> u8 {
        if index >= self.len {
            return 0;
        }
        self.bytes[index]
    }

    /// Returns the number of intermediate bytes.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns whether there are no intermediate bytes.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns the bytes as a slice.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum GraphicsOptions {
    /// Off (reset).
    Off = 0,
    /// Intense (bright).
    Intense = 1,
    /// Faint.
    Faint = 2,
    /// Italics.
    Italics = 3,
    /// Underline (single).
    Underline = 4,
    /// Blink.
    Blink = 5,
    /// Rapid blink.
    RapidBlink = 6,
    /// Reverse video.
    Negative = 7,
    /// Invisible.
    Invisible = 8,
    /// Crossed out.
    CrossedOut = 9,
    /// Doubly underlined.
    DoublyUnderlined = 10,
    /// Underlined.
    #[allow(deprecated)]
    Underlined = 11,
    /// Steady (not faint).
    Steady = 20,
    /// Not italic.
    NotItalics = 23,
    /// Not underlined.
    NoUnderline = 24,
    /// Steady (not blinking).
    SteadyBlink = 25,
    /// Positive (not reverse).
    Positive = 27,
    /// Show (not invisible).
    Visible = 28,
    /// Not crossed out.
    NotCrossedOut = 29,
    /// Overline.
    Overline = 53,
    /// Double underline (alias for DoublyUnderlined).
    #[allow(deprecated)]
    DoubleUnderline = 13,
    /// Normal color (not intense).
    NormalColor = 22,
    /// Normal intensity (not faint) - alias for NormalColor.
    #[allow(deprecated)]
    NormalIntensity = 230,
    /// No overline.
    NoOverline = 55,
    /// Underline color.
    UnderlineColor = 58,
    /// Reset underline color.
    ResetUnderlineColor = 59,
    /// Foreground colors.
    ForegroundBlack = 30,
    ForegroundRed = 31,
    ForegroundGreen = 32,
    ForegroundYellow = 33,
    ForegroundBlue = 34,
    ForegroundMagenta = 35,
    ForegroundCyan = 36,
    ForegroundWhite = 37,
    /// Default foreground.
    ForegroundDefault = 39,
    /// Background colors.
    BackgroundBlack = 40,
    BackgroundRed = 41,
    BackgroundGreen = 42,
    BackgroundYellow = 43,
    BackgroundBlue = 44,
    BackgroundMagenta = 45,
    BackgroundCyan = 46,
    BackgroundWhite = 47,
    /// Default background.
    BackgroundDefault = 49,
    /// Bright foreground colors.
    BrightForegroundBlack = 90,
    BrightForegroundRed = 91,
    BrightForegroundGreen = 92,
    BrightForegroundYellow = 93,
    BrightForegroundBlue = 94,
    BrightForegroundMagenta = 95,
    BrightForegroundCyan = 96,
    BrightForegroundWhite = 97,
    /// Bright background colors.
    BrightBackgroundBlack = 100,
    BrightBackgroundRed = 101,
    BrightBackgroundGreen = 102,
    BrightBackgroundYellow = 103,
    BrightBackgroundBlue = 104,
    BrightBackgroundMagenta = 105,
    BrightBackgroundCyan = 106,
    BrightBackgroundWhite = 107,
    /// Underline style values.
    DoublyUnderline = 21,
    UnderlineOrOverline = 12,
}

/// Underline style constants.
pub const SINGLE_UNDERLINE: u8 = 4;

/// Additional underline constants.
pub const UNDERLINE_NONE: u8 = 24;

/// Additional underline-related constants (not in GraphicsOptions enum).
pub const OVERLINE_NONE: u8 = 55;

/// SGR save/restore stack options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SgrSaveRestoreStackOptions {
    All = 0,
    Intense = 1,
    Faintness = 2,
    Italics = 3,
    Underline = 4,
    Blink = 5,
    Negative = 6,
    Invisible = 7,
    CrossedOut = 8,
    DoublyUnderlined = 9,
    SaveForegroundColor = 10,
    SaveBackgroundColor = 11,
    Max = 12,
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

/// Mode parameters for SM/R/DESET/DECSET.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeParams {
    /// Insert/Replace mode (IRM).
    Irm = 4,
    /// Line feed/new line mode (LNM).
    Lnm = 20,
    /// Cursor keys mode (DECCKM).
    DecCkm = 25,
    /// ANSI mode (DECANM).
    DecAnm = 47,
    /// Set number of columns (DECCOLM).
    DecColm = 3,
    /// Screen mode (DECSCNM).
    DecScnm = 5,
    /// Origin mode (DECOM).
    DecOm = 6,
    /// Auto wrap mode (DECAWM).
    DecAwm = 7,
    /// Auto repeat mode (DECARM).
    DecArm = 8,
    /// Start cursor blink (ATT610).
    Att610StartCursorBlink = 9,
    /// Text cursor enable mode (DECTCEM).
    DecTcem = 12,
    /// Enable DECCOLM support.
    EnableDecColmSupport = 81,
    /// Page cursor coupling mode.
    DecPccm = 85,
    /// Numeric keypad mode (DECNKM).
    DecNkm = 66,
    /// Backarrow key mode (DECBKM).
    DecBkm = 82,
    /// Left/right margin mode.
    DecLrmm = 2048,
    /// Sixel display mode.
    DecSdm = 48,
    /// Erase color mode.
    DecEcm = 1040,
    /// VT200 mouse mode.
    Vt200MouseMode = 9_000,
    /// Button event mouse mode.
    ButtonEventMouseMode = 1000,
    /// Any event mouse mode.
    AnyEventMouseMode = 1002,
    /// Focus event mode.
    FocusEventMode = 1004,
    /// UTF-8 extended mode.
    Utf8ExtendedMode = 2004,
    /// SGR extended mode.
    SgrExtendedMode = 2006,
    /// Alternate scroll.
    AlternateScroll = 1007,
    /// Alternate screen buffer.
    AlternateScreenBuffer = 1049,
    /// Bracketed paste mode.
    BracketedPasteMode = 2005,
    /// Synchronized output.
    SynchronizedOutput = 2001,
    /// Grapheme cluster mode.
    GraphemeClusterMode = 2027,
    /// Win32 input mode.
    Win32InputMode = 2012,
}

impl ModeParams {
    /// Returns the parameter number for this mode.
    pub fn param_number(&self) -> VTInt {
        match self {
            ModeParams::Irm => 4,
            ModeParams::Lnm => 20,
            ModeParams::DecCkm => 25,
            ModeParams::DecAnm => 47,
            ModeParams::DecColm => 3,
            ModeParams::DecScnm => 5,
            ModeParams::DecOm => 6,
            ModeParams::DecAwm => 7,
            ModeParams::DecArm => 8,
            ModeParams::Att610StartCursorBlink => 9,
            ModeParams::DecTcem => 12,
            ModeParams::EnableDecColmSupport => 81,
            ModeParams::DecPccm => 85,
            ModeParams::DecNkm => 66,
            ModeParams::DecBkm => 82,
            ModeParams::DecLrmm => 2048,
            ModeParams::DecSdm => 48,
            ModeParams::DecEcm => 1040,
            ModeParams::Vt200MouseMode => 9,
            ModeParams::ButtonEventMouseMode => 1000,
            ModeParams::AnyEventMouseMode => 1002,
            ModeParams::FocusEventMode => 1004,
            ModeParams::Utf8ExtendedMode => 2004,
            ModeParams::SgrExtendedMode => 2006,
            ModeParams::AlternateScroll => 1007,
            ModeParams::AlternateScreenBuffer => 1049,
            ModeParams::BracketedPasteMode => 2004,
            ModeParams::SynchronizedOutput => 2001,
            ModeParams::GraphemeClusterMode => 2027,
            ModeParams::Win32InputMode => 2012,
        }
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
        let vtid = VTID::new().with_intermediate(b'A').with_intermediate(b'B').finalize();
        assert!(!vtid.is_empty());
        assert_eq!(vtid.len(), 2);
        // Bytes are stored in little-endian order
        assert_eq!(vtid.byte_at(0), b'A');
        assert_eq!(vtid.byte_at(1), b'B');
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

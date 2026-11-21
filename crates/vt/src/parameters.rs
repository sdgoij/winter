//! VT parameter parsing.
//!
//! Ported from VT parameter handling in the Windows Terminal C++ VT module.

/// Parsed VT parameters (CSI, OSC, DCS).
///
/// Parameters are stored as `Vec<i32>` to match the C++ `int` convention.
/// An empty parameter list defaults to a single unspecified parameter (value 0).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VtParameters {
    params: Vec<i32>,
}

impl VtParameters {
    /// Creates a new empty parameter list.
    pub fn new() -> Self {
        Self { params: Vec::new() }
    }

    /// Creates a parameter list from a string (split on `;`).
    ///
    /// - Empty string → single unspecified param (value 0)
    /// - "1;2;3" → [1, 2, 3]
    /// - "5" → [5]
    /// - Missing trailing params default to 0 (handled by `param()`)
    pub fn from_string(s: &str) -> Self {
        if s.is_empty() {
            return Self { params: vec![0] };
        }

        let params: Vec<i32> = s.split(';').map(|p| p.parse().unwrap_or(0)).collect();

        Self { params }
    }

    /// Pushes a parameter value.
    pub fn push_param(&mut self, value: i32) {
        self.params.push(value);
    }

    /// Gets the parameter at the given index.
    ///
    /// Returns 0 (the default unspecified value) if the index is out of range.
    pub fn param(&self, index: usize) -> i32 {
        self.params.get(index).copied().unwrap_or(0)
    }

    /// Gets the number of parameters.
    pub fn param_count(&self) -> usize {
        self.params.len()
    }

    /// Returns whether the parameter list is empty.
    ///
    /// Note: an empty list is normalized to `[0]`, so this only returns true
    /// if the list has been explicitly cleared.
    pub fn is_empty(&self) -> bool {
        self.params.is_empty()
    }

    /// Returns whether all parameters are unspecified (zero).
    pub fn all_unspecified(&self) -> bool {
        self.params.iter().all(|&p| p == 0)
    }

    /// Returns whether the parameter at the given index is unspecified.
    pub fn is_unspecified_param(&self, index: usize) -> bool {
        self.param(index) == 0
    }

    /// Returns an iterator over the parameters.
    pub fn iter(&self) -> impl Iterator<Item = &i32> {
        self.params.iter()
    }

    /// Converts parameters to the semicolon-separated form the wire uses.
    ///
    /// Kept inherent rather than routed through `Display` because the callers
    /// build a parameter list, not a human-readable rendering.
    #[allow(clippy::inherent_to_string)]
    pub fn to_string(&self) -> String {
        self.params.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(";")
    }

    /// Clears all parameters.
    pub fn clear(&mut self) {
        self.params.clear();
    }
}

impl Default for VtParameters {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_is_empty() {
        let params = VtParameters::new();
        assert!(params.is_empty());
        assert_eq!(params.param_count(), 0);
    }

    #[test]
    fn test_from_empty_string() {
        let params = VtParameters::from_string("");
        assert!(!params.is_empty());
        assert_eq!(params.param_count(), 1);
        assert_eq!(params.param(0), 0);
    }

    #[test]
    fn test_from_single_param() {
        let params = VtParameters::from_string("5");
        assert_eq!(params.param_count(), 1);
        assert_eq!(params.param(0), 5);
        assert_eq!(params.param(1), 0);
    }

    #[test]
    fn test_from_multiple_params() {
        let params = VtParameters::from_string("1;2;3");
        assert_eq!(params.param_count(), 3);
        assert_eq!(params.param(0), 1);
        assert_eq!(params.param(1), 2);
        assert_eq!(params.param(2), 3);
        assert_eq!(params.param(3), 0);
    }

    #[test]
    fn test_push_param() {
        let mut params = VtParameters::new();
        params.push_param(42);
        assert_eq!(params.param_count(), 1);
        assert_eq!(params.param(0), 42);
    }

    #[test]
    fn test_out_of_range_returns_zero() {
        let params = VtParameters::from_string("1");
        assert_eq!(params.param(100), 0);
    }

    #[test]
    fn test_all_unspecified() {
        let params = VtParameters::from_string("");
        assert!(params.all_unspecified());

        let params2 = VtParameters::from_string("0;0;0");
        assert!(params2.all_unspecified());

        let params3 = VtParameters::from_string("1;0");
        assert!(!params3.all_unspecified());
    }

    #[test]
    fn test_to_string_roundtrip() {
        let original = "5;10;20";
        let params = VtParameters::from_string(original);
        assert_eq!(params.to_string(), "5;10;20");
    }

    #[test]
    fn test_clear() {
        let mut params = VtParameters::from_string("1;2;3");
        params.clear();
        assert!(params.is_empty());
        assert_eq!(params.param(0), 0);
    }

    #[test]
    fn test_iter() {
        let params = VtParameters::from_string("1;2;3");
        let collected: Vec<i32> = params.iter().copied().collect();
        assert_eq!(collected, vec![1, 2, 3]);
    }

    #[test]
    fn test_negative_values() {
        let params = VtParameters::from_string("-1;0;1");
        assert_eq!(params.param(0), -1);
    }

    #[test]
    fn test_negative_values_actual() {
        let params = VtParameters::from_string("-1;0;1");
        assert_eq!(params.param(0), -1);
        assert_eq!(params.param(1), 0);
        assert_eq!(params.param(2), 1);
    }

    #[test]
    fn test_invalid_params_default_to_zero() {
        let params = VtParameters::from_string("1;abc;3");
        assert_eq!(params.param(0), 1);
        assert_eq!(params.param(1), 0);
        assert_eq!(params.param(2), 3);
    }

    #[test]
    fn test_is_unspecified_param() {
        let params = VtParameters::from_string("0;1;0");
        assert!(params.is_unspecified_param(0));
        assert!(!params.is_unspecified_param(1));
        assert!(params.is_unspecified_param(2));
        assert!(params.is_unspecified_param(3));
    }
}

//! `Alias` — the console's `doskey`-style command aliases.
//!
//! Ported from `alias.cpp` and `alias.h`. Upstream keeps one table keyed by
//! application, with a case-insensitive comparison on both the application and
//! the alias; the `ConsoleAlias` A/W API routines around it only convert code
//! pages and probe buffer sizes, which a portable host does not need.
//!
//! The expansion is the part a cooked read uses: the first word of a line
//! names an alias, the remaining words are `$1`-`$9` and `$*` parameters, and
//! `$l`, `$g`, `$b` and `$t` stand for `<`, `>`, `|` and a line break.

use std::collections::BTreeMap;

/// Upstream's `args[10]`.
const MAX_ARGUMENTS: usize = 10;

/// Why an alias could not be added.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AliasError {
    /// The alias name was empty, upstream's `E_INVALIDARG`.
    EmptySource,
}

impl std::fmt::Display for AliasError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptySource => formatter.write_str("an alias name cannot be empty"),
        }
    }
}

impl std::error::Error for AliasError {}

/// The result of expanding a line that began with an alias.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AliasExpansion {
    /// The expansion, with the trailing line break upstream appends.
    pub text: String,
    /// How many lines the expansion occupies.
    pub line_count: usize,
}

/// One application's aliases.
pub type AliasList = BTreeMap<String, String>;

/// The console's aliases, one table per application.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AliasTable {
    data: BTreeMap<String, AliasList>,
}

impl AliasTable {
    /// Creates an empty table.
    pub const fn new() -> Self {
        Self { data: BTreeMap::new() }
    }

    /// Adds an alias, or removes it when `target` is empty.
    ///
    /// The application and alias names are compared case-insensitively; the
    /// target keeps its case.
    pub fn add(&mut self, exe_name: &str, source: &str, target: &str) -> Result<(), AliasError> {
        if source.is_empty() {
            return Err(AliasError::EmptySource);
        }

        let exe_name = exe_name.to_lowercase();
        let source = source.to_lowercase();

        if target.is_empty() {
            if let Some(list) = self.data.get_mut(&exe_name) {
                list.remove(&source);
            }
        } else {
            self.data
                .entry(exe_name)
                .or_default()
                .insert(source, target.to_string());
        }

        Ok(())
    }

    /// Returns the target for an alias, if one exists and is not empty.
    pub fn get(&self, exe_name: &str, source: &str) -> Option<&str> {
        self.data
            .get(&exe_name.to_lowercase())?
            .get(&source.to_lowercase())
            .map(String::as_str)
            .filter(|target| !target.is_empty())
    }

    /// Iterates over an application's aliases as `(source, target)`.
    pub fn aliases(&self, exe_name: &str) -> impl Iterator<Item = (&str, &str)> + '_ {
        self.data
            .get(&exe_name.to_lowercase())
            .into_iter()
            .flat_map(|list| list.iter().map(|(source, target)| (source.as_str(), target.as_str())))
    }

    /// Iterates over the applications that have aliases.
    pub fn exes(&self) -> impl Iterator<Item = &str> + '_ {
        self.data.keys().map(String::as_str)
    }

    /// Returns whether no application has an alias.
    pub fn is_empty(&self) -> bool {
        self.data.values().all(AliasList::is_empty)
    }

    /// Removes every alias for one application.
    pub fn clear_exe(&mut self, exe_name: &str) {
        if let Some(list) = self.data.get_mut(&exe_name.to_lowercase()) {
            list.clear();
        }
    }

    /// Removes cmd.exe's aliases, upstream's `s_ClearCmdExeAliases`.
    pub fn clear_cmd_exe_aliases(&mut self) {
        self.clear_exe("cmd.exe");
    }

    /// Removes every alias.
    pub fn clear(&mut self) {
        self.data.clear();
    }

    /// Expands a line that begins with an alias.
    ///
    /// Returns `None` when the application has no aliases, the line is empty,
    /// or the first word is not an alias. The expansion always ends with
    /// `\r\n`, and `line_count` counts the lines it produced, as upstream's
    /// `s_MatchAndCopyAlias` does.
    pub fn expand(&self, source_text: &str, exe_name: &str) -> Option<AliasExpansion> {
        let application = self.data.get(&exe_name.to_lowercase())?;
        if application.is_empty() {
            return None;
        }

        let arguments = split_arguments(source_text);
        let first = arguments.first()?;
        let target = application.get(&first.text.to_lowercase())?;
        if target.is_empty() {
            return None;
        }

        let mut text = String::new();
        let mut lines = 0;
        let mut characters = target.chars();

        while let Some(character) = characters.next() {
            if character != '$' {
                text.push(character);
                continue;
            }

            let Some(escaped) = characters.next() else {
                // A trailing `$` is literal.
                text.push('$');
                break;
            };

            match escaped.to_ascii_lowercase() {
                '1'..='9' => {
                    let index = escaped as usize - '0' as usize;
                    if let Some(argument) = arguments.get(index) {
                        text.push_str(argument.text);
                    }
                }
                '*' => {
                    // Everything from the first parameter to the end of the
                    // line, with its original spacing.
                    if let Some(argument) = arguments.get(1) {
                        text.push_str(&source_text[argument.start..]);
                    }
                }
                'l' => text.push('<'),
                'g' => text.push('>'),
                'b' => text.push('|'),
                't' => {
                    text.push_str("\r\n");
                    lines += 1;
                }
                _ => {
                    // An unknown escape keeps the dollar and the character.
                    text.push('$');
                    text.push(escaped);
                }
            }
        }

        text.push_str("\r\n");
        lines += 1;

        Some(AliasExpansion {
            text,
            line_count: lines,
        })
    }
}

struct Argument<'a> {
    text: &'a str,
    start: usize,
}

/// Splits on spaces exactly as upstream does: runs of spaces collapse and a
/// tab is part of a word.
fn split_arguments(source: &str) -> Vec<Argument<'_>> {
    let mut arguments = Vec::new();
    let mut begin = 0;

    while begin < source.len() {
        let end = source[begin..].find(' ').map_or(source.len(), |offset| begin + offset);
        let text = &source[begin..end];
        if text.is_empty() {
            break;
        }

        arguments.push(Argument { text, start: begin });
        if arguments.len() >= MAX_ARGUMENTS {
            break;
        }

        match source[end..].find(|character| character != ' ') {
            Some(offset) => begin = end + offset,
            None => break,
        }
    }

    arguments
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> AliasTable {
        let mut aliases = AliasTable::new();
        aliases.add("cmd.exe", "foo", "echo $1 $2").expect("a name");
        aliases
    }

    #[test]
    fn an_alias_expands_with_its_parameters() {
        let expansion = table().expand("foo a b", "cmd.exe").expect("an alias");
        assert_eq!(expansion.text, "echo a b\r\n");
        assert_eq!(expansion.line_count, 1);
    }

    #[test]
    fn a_missing_parameter_expands_to_nothing() {
        let expansion = table().expand("foo a", "cmd.exe").expect("an alias");
        assert_eq!(expansion.text, "echo a \r\n");
    }

    #[test]
    fn an_unknown_alias_does_not_expand() {
        assert!(table().expand("bar a b", "cmd.exe").is_none());
    }

    #[test]
    fn an_unknown_application_does_not_expand() {
        assert!(table().expand("foo a b", "powershell.exe").is_none());
    }

    #[test]
    fn the_application_and_alias_match_case_insensitively() {
        let mut aliases = AliasTable::new();
        aliases.add("CMD.EXE", "FOO", "dir").expect("a name");

        assert_eq!(aliases.get("cmd.exe", "foo"), Some("dir"));
        assert_eq!(
            aliases.expand("Foo x", "cmd.exe").map(|e| e.text),
            Some("dir\r\n".to_string())
        );
    }

    #[test]
    fn star_appends_every_parameter_with_its_spacing() {
        let mut aliases = AliasTable::new();
        aliases.add("cmd.exe", "foo", "echo $*").expect("a name");
        let expansion = aliases.expand("foo  a   b", "cmd.exe").expect("an alias");
        assert_eq!(expansion.text, "echo a   b\r\n");
    }

    #[test]
    fn the_escape_sequences_map_to_their_characters() {
        let mut aliases = AliasTable::new();
        aliases.add("cmd.exe", "foo", "a $l b $g c $b d").expect("a name");
        let expansion = aliases.expand("foo", "cmd.exe").expect("an alias");
        assert_eq!(expansion.text, "a < b > c | d\r\n");
    }

    #[test]
    fn a_line_break_escape_counts_a_line() {
        let mut aliases = AliasTable::new();
        aliases.add("cmd.exe", "foo", "a$tb").expect("a name");
        let expansion = aliases.expand("foo", "cmd.exe").expect("an alias");
        assert_eq!(expansion.text, "a\r\nb\r\n");
        assert_eq!(expansion.line_count, 2);
    }

    #[test]
    fn an_unknown_escape_keeps_the_dollar_and_the_character() {
        let mut aliases = AliasTable::new();
        aliases.add("cmd.exe", "foo", "a $q").expect("a name");
        let expansion = aliases.expand("foo", "cmd.exe").expect("an alias");
        assert_eq!(expansion.text, "a $q\r\n");
    }

    #[test]
    fn a_trailing_dollar_is_literal() {
        let mut aliases = AliasTable::new();
        aliases.add("cmd.exe", "foo", "cost$").expect("a name");
        let expansion = aliases.expand("foo", "cmd.exe").expect("an alias");
        assert_eq!(expansion.text, "cost$\r\n");
    }

    #[test]
    fn an_empty_target_removes_the_alias() {
        let mut aliases = table();
        aliases.add("cmd.exe", "foo", "").expect("a name");
        assert!(aliases.get("cmd.exe", "foo").is_none());
        assert!(aliases.expand("foo a b", "cmd.exe").is_none());
    }

    #[test]
    fn an_empty_source_is_rejected() {
        let mut aliases = AliasTable::new();
        assert_eq!(aliases.add("cmd.exe", "", "dir"), Err(AliasError::EmptySource));
    }

    #[test]
    fn a_leading_space_produces_no_arguments() {
        assert!(table().expand(" foo", "cmd.exe").is_none());
    }

    #[test]
    fn a_tab_does_not_split_arguments() {
        // The first word is "foo\ta", which is not the alias "foo".
        assert!(table().expand("foo\ta", "cmd.exe").is_none());
    }

    #[test]
    fn clearing_cmd_exe_leaves_other_applications() {
        let mut aliases = AliasTable::new();
        aliases.add("cmd.exe", "foo", "dir").expect("a name");
        aliases.add("powershell.exe", "ls", "Get-ChildItem").expect("a name");

        aliases.clear_cmd_exe_aliases();

        assert!(aliases.get("cmd.exe", "foo").is_none());
        assert_eq!(aliases.get("powershell.exe", "ls"), Some("Get-ChildItem"));
        assert!(!aliases.is_empty());
    }

    #[test]
    fn the_applications_and_their_aliases_can_be_listed() {
        let mut aliases = table();
        aliases.add("cmd.exe", "bar", "dir").expect("a name");

        assert_eq!(aliases.exes().collect::<Vec<_>>(), ["cmd.exe"]);
        let pairs: Vec<(&str, &str)> = aliases.aliases("cmd.exe").collect();
        assert_eq!(pairs, [("bar", "dir"), ("foo", "echo $1 $2")]);

        aliases.clear();
        assert!(aliases.is_empty());
        assert_eq!(aliases.exes().count(), 0);
    }
}

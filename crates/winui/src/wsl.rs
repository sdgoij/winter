//! WSL distributions, offered as profiles.
//!
//! Windows Terminal generates a profile per installed distribution
//! (`WslDistroGenerator.cpp`). That list belongs to the machine, not to the
//! settings file, so it is merged into what the shell offers and never written
//! back: a save edits the file the user wrote, and a distribution that appears
//! or disappears shows up in the menu without touching it.

use std::os::windows::process::CommandExt;
use std::process::Command;

use settings::Profile;

/// The program every distribution profile runs.
const WSL: &str = "wsl.exe";
/// `CREATE_NO_WINDOW`: without it the query flashes a console window.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A profile per installed distribution, in the order `wsl.exe` lists them.
pub(super) fn profiles() -> Vec<Profile> {
    let Ok(output) = Command::new(WSL)
        .args(["-l", "-q"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    else {
        return Vec::new();
    };
    parse(&output.stdout)
}

/// The profiles the query's output names.
fn parse(output: &[u8]) -> Vec<Profile> {
    decode(output)
        .lines()
        .map(|line| line.trim().trim_start_matches('\u{feff}'))
        .filter(|name| !name.is_empty())
        .map(profile)
        .collect()
}

/// The profile that starts the distribution `name`.
fn profile(name: &str) -> Profile {
    Profile {
        name: name.to_string(),
        command: WSL.to_string(),
        args: vec!["-d".to_string(), name.to_string()],
        // Without a title every distribution's tab would be called `wsl`.
        title: Some(name.to_string()),
    }
}

/// The query's text, whichever way it encoded it.
///
/// `wsl.exe` writes UTF-16LE when its output is redirected and UTF-8 when the
/// console code page is already UTF-8, and which one it picks has changed
/// between releases. A UTF-16LE stream of ASCII names has a zero in every
/// second byte, so the bytes decide instead of the version.
fn decode(output: &[u8]) -> String {
    let body = output.strip_prefix(&[0xFF, 0xFE]).unwrap_or(output);
    let pairs = body.len() / 2;
    let zeros = body
        .iter()
        .skip(1)
        .step_by(2)
        .take(pairs)
        .filter(|byte| **byte == 0)
        .count();
    if pairs > 0 && zeros == pairs {
        let units: Vec<u16> = body
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(body).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bytes `wsl.exe` writes when it redirects its own output.
    fn utf16(text: &str) -> Vec<u8> {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in text.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn a_utf16_list_becomes_one_profile_per_distro() {
        let profiles = parse(&utf16("Ubuntu\r\nDebian\r\n"));

        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0].name, "Ubuntu");
        assert_eq!(profiles[0].command, "wsl.exe");
        assert_eq!(profiles[0].args, vec!["-d".to_string(), "Ubuntu".to_string()]);
        assert_eq!(profiles[0].title.as_deref(), Some("Ubuntu"));
        assert_eq!(profiles[1].name, "Debian");
    }

    #[test]
    fn a_utf8_list_reads_the_same_way() {
        assert_eq!(parse(b"Ubuntu\nDebian\n"), parse(&utf16("Ubuntu\nDebian\n")));
    }

    #[test]
    fn blank_lines_and_a_bom_are_not_distros() {
        let profiles = parse(&utf16("\r\nUbuntu\r\n\r\n"));

        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].name, "Ubuntu");
    }

    #[test]
    fn a_machine_without_distros_offers_none() {
        assert!(parse(b"").is_empty());
    }
}

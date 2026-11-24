//! Shell integration: the hook that makes PowerShell send FinalTerm marks.
//!
//! Windows PowerShell emits no `OSC 133` of its own, and conhost cannot forward
//! one, so a PowerShell profile is started with a small script that emits the
//! marks behind the scrollbar's ticks and their red error state (ADR 060). The
//! script is embedded in the binary and written to the app's cache directory,
//! so an installed binary needs no data files.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use server::pty::PtyCommand;

/// The script, embedded so the hook never depends on the install layout.
const SCRIPT: &str = include_str!("../shell-integration/winter.ps1");

/// The variable that keeps the hook off a shell; `0` turns it off.
pub const OPT_OUT: &str = "WINTER_SHELL_INTEGRATION";

/// The switches that already decide what PowerShell runs. `-File` and
/// `-Command` take everything after them, so a hook appended there would
/// either be ignored or steal the user's own.
const OWN_COMMAND: [&str; 9] = [
    "command",
    "c",
    "file",
    "f",
    "encodedcommand",
    "e",
    "ec",
    "noninteractive",
    "noni",
];

/// The path the hook was written to, or `None` when it could not be written.
///
/// Called once per process: the second tab reuses the first one's answer.
pub fn installed() -> Option<PathBuf> {
    static WRITTEN: OnceLock<Option<PathBuf>> = OnceLock::new();
    WRITTEN
        .get_or_init(|| {
            if std::env::var_os(OPT_OUT).is_some_and(|value| value == "0") {
                return None;
            }
            let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?;
            write_script(&base.join("winter")).ok()
        })
        .clone()
}

/// Writes the script into `directory`, keeping an up-to-date copy.
fn write_script(directory: &Path) -> std::io::Result<PathBuf> {
    let path = directory.join("shell-integration.ps1");
    if std::fs::read_to_string(&path).ok().as_deref() != Some(SCRIPT) {
        std::fs::create_dir_all(directory)?;
        std::fs::write(&path, SCRIPT)?;
    }
    Ok(path)
}

/// Returns `command` with the hook attached when it is a PowerShell that can
/// carry one.
///
/// `script` is where the hook was written. `None` returns every command as the
/// settings file wrote it.
pub fn attach(command: PtyCommand, script: Option<&Path>) -> PtyCommand {
    let Some(script) = script else {
        return command;
    };
    if !is_powershell(command.program()) || chooses_its_own_command(&command) {
        return command;
    }

    let arguments: Vec<OsString> = command.arguments().map(OsStr::to_os_string).collect();
    let no_exit = arguments.iter().any(|argument| switch_is(argument, "noexit"));
    let mut hooked = PtyCommand::new(command.program()).args(arguments);
    if !no_exit {
        hooked = hooked.arg("-NoExit");
    }
    hooked.arg("-File").arg(script.as_os_str())
}

/// The switches whose next argument is their value, not a command.
const VALUE_SWITCHES: [&str; 6] = [
    "executionpolicy",
    "ep",
    "workingdirectory",
    "wd",
    "windowstyle",
    "configurationname",
];

/// Whether `program` is one of PowerShell's executables.
fn is_powershell(program: &OsStr) -> bool {
    let name = Path::new(program)
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let stem = name.strip_suffix(".exe").unwrap_or(&name);
    stem == "powershell" || stem == "pwsh"
}

/// Whether the arguments already name what to run.
fn chooses_its_own_command(command: &PtyCommand) -> bool {
    let mut expects_value = false;
    for argument in command.arguments() {
        let Some(text) = argument.to_str() else {
            return true;
        };
        if expects_value {
            expects_value = false;
            continue;
        }
        match text.strip_prefix('-').or_else(|| text.strip_prefix('/')) {
            Some(flag) => {
                if OWN_COMMAND.iter().any(|known| flag.eq_ignore_ascii_case(known)) {
                    return true;
                }
                expects_value = VALUE_SWITCHES.iter().any(|known| flag.eq_ignore_ascii_case(known));
            }
            // A bare word is a script or a command of its own.
            None => return true,
        }
    }
    false
}

/// Whether `argument` is the switch `name`, in any case and either prefix.
fn switch_is(argument: &OsStr, name: &str) -> bool {
    argument
        .to_str()
        .and_then(|text| text.strip_prefix('-').or_else(|| text.strip_prefix('/')))
        .is_some_and(|flag| flag.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hook_reads_utf8_files() {
        // Windows PowerShell 5.1 decodes a BOM-less UTF-8 file with the system
        // ANSI code page; the hook names UTF-8 so `cat` is readable.
        assert!(SCRIPT.contains("$PSDefaultParameterValues['Get-Content:Encoding'] = 'UTF8'"));
    }

    fn script() -> PathBuf {
        PathBuf::from(r"C:\cache\winter\shell-integration.ps1")
    }

    fn arguments(command: PtyCommand) -> Vec<String> {
        command
            .arguments()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn a_powershell_profile_is_started_with_the_hook() {
        let hooked = attach(PtyCommand::new("powershell.exe"), Some(&script()));

        assert_eq!(hooked.program(), "powershell.exe");
        assert_eq!(
            arguments(hooked),
            vec![
                "-NoExit".to_string(),
                "-File".to_string(),
                script().to_string_lossy().into_owned()
            ]
        );
    }

    #[test]
    fn a_pwsh_profile_is_hooked_the_same_way() {
        for program in ["pwsh", "pwsh.exe", "PWSH.EXE"] {
            let hooked = attach(PtyCommand::new(program), Some(&script()));
            assert_eq!(hooked.program(), program);
            assert_eq!(arguments(hooked)[0], "-NoExit", "{program}");
        }
    }

    #[test]
    fn a_shell_named_by_an_absolute_path_is_still_a_pwsh() {
        // `Path::file_name` reads the host's separator, so the path has to be
        // one the host would actually use to start a shell.
        #[cfg(windows)]
        let installed = r"C:\Program Files\PowerShell\7\pwsh.exe";
        #[cfg(unix)]
        let installed = "/usr/bin/pwsh";

        let hooked = attach(PtyCommand::new(installed), Some(&script()));
        assert_eq!(arguments(hooked).first().map(String::as_str), Some("-NoExit"));
    }

    #[test]
    fn the_user_s_own_arguments_come_first() {
        let hooked = attach(
            PtyCommand::new("powershell.exe").args(["-NoLogo", "-WorkingDirectory", "C:\\"]),
            Some(&script()),
        );

        assert_eq!(
            arguments(hooked),
            vec![
                "-NoLogo".to_string(),
                "-WorkingDirectory".to_string(),
                "C:\\".to_string(),
                "-NoExit".to_string(),
                "-File".to_string(),
                script().to_string_lossy().into_owned(),
            ]
        );
    }

    #[test]
    fn an_existing_no_exit_is_not_repeated() {
        let hooked = attach(PtyCommand::new("powershell.exe").args(["-NoExit"]), Some(&script()));

        assert_eq!(arguments(hooked).iter().filter(|arg| *arg == "-NoExit").count(), 1);
    }

    #[test]
    fn a_shell_the_hook_cannot_help_is_left_alone() {
        for program in ["cmd.exe", "bash", "zsh", "wsl.exe", "sh"] {
            let command = PtyCommand::new(program).args(["-l"]);
            let attached = attach(command.clone(), Some(&script()));
            assert_eq!(attached, command, "{program}");
        }
    }

    #[test]
    fn a_profile_that_names_its_own_command_is_left_alone() {
        for args in [
            vec!["-Command", "Get-Date"],
            vec!["-c", "Get-Date"],
            vec!["-File", "setup.ps1"],
            vec!["-f", "setup.ps1"],
            vec!["-EncodedCommand", "AA=="],
            vec!["-NonInteractive"],
            vec!["setup.ps1"],
        ] {
            let command = PtyCommand::new("powershell.exe").args(args.clone());
            let attached = attach(command.clone(), Some(&script()));
            assert_eq!(attached, command, "{args:?}");
        }
    }

    #[test]
    fn with_no_script_every_command_is_left_alone() {
        let command = PtyCommand::new("powershell.exe");
        assert_eq!(attach(command.clone(), None), command);
    }

    #[test]
    fn the_script_lands_in_its_directory_and_is_refreshed() {
        let directory =
            std::env::temp_dir().join(format!("winter-shell-integration-{}-{}", std::process::id(), line!()));
        let _ = std::fs::remove_dir_all(&directory);

        let path = write_script(&directory).expect("write");
        assert_eq!(path, directory.join("shell-integration.ps1"));
        assert_eq!(std::fs::read_to_string(&path).expect("read"), SCRIPT);

        // A stale copy is replaced, so an upgrade does not keep an old script.
        std::fs::write(&path, "old").expect("replace");
        write_script(&directory).expect("rewrite");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), SCRIPT);

        std::fs::remove_dir_all(&directory).expect("clean up");
    }
}

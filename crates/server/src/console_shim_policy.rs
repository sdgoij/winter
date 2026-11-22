//! `ConsoleShimPolicy` — the compatibility shims a client needs.
//!
//! Ported from `server/ConsoleShimPolicy.cpp`. Upstream reads the client's
//! image name from its process handle; here the platform layer supplies the
//! name, which keeps the classification itself portable.

/// Which compatibility shims apply to the connected client.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConsoleShimPolicy {
    is_cmd: bool,
    is_powershell: bool,
}

impl ConsoleShimPolicy {
    /// Classifies the client whose executable is named `client_name`.
    ///
    /// This is the file name alone, as upstream passes
    /// `processName.filename()`; a full path is not recognised. The comparison
    /// is ASCII case-insensitive, matching `til::equals_insensitive_ascii`. A
    /// name that cannot be read is neither cmd nor PowerShell, which is what
    /// upstream's catch block leaves behind.
    pub fn new(client_name: &str) -> Self {
        Self {
            is_cmd: client_name.eq_ignore_ascii_case("cmd.exe"),
            // Both Windows PowerShell and PowerShell Core need the long-tail
            // shims for versions that do not use `ESC [ 3 J` for Clear-Host.
            is_powershell: client_name.eq_ignore_ascii_case("powershell.exe")
                || client_name.eq_ignore_ascii_case("pwsh.exe"),
        }
    }

    /// Returns whether the client is literally `cmd.exe`.
    pub const fn is_cmd_exe(&self) -> bool {
        self.is_cmd
    }

    /// Returns whether the client is `powershell.exe` or `pwsh.exe`.
    pub const fn is_powershell_exe(&self) -> bool {
        self.is_powershell
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmd_is_recognised_in_any_case() {
        for name in ["cmd.exe", "CMD.EXE", "Cmd.Exe"] {
            let policy = ConsoleShimPolicy::new(name);
            assert!(policy.is_cmd_exe(), "{name} should be cmd");
            assert!(!policy.is_powershell_exe(), "{name} should not be PowerShell");
        }
    }

    #[test]
    fn both_powershells_are_recognised() {
        for name in ["powershell.exe", "POWERSHELL.EXE", "pwsh.exe", "PWSH.EXE"] {
            let policy = ConsoleShimPolicy::new(name);
            assert!(policy.is_powershell_exe(), "{name} should be PowerShell");
            assert!(!policy.is_cmd_exe(), "{name} should not be cmd");
        }
    }

    #[test]
    fn anything_else_gets_no_shims() {
        for name in ["", "bash", "cmd", "wsl.exe"] {
            let policy = ConsoleShimPolicy::new(name);
            assert!(!policy.is_cmd_exe());
            assert!(!policy.is_powershell_exe());
        }
    }

    #[test]
    fn a_full_path_is_the_callers_job_to_reduce() {
        // Upstream compares the file name, not the path.
        let policy = ConsoleShimPolicy::new(r"C:\Windows\System32\cmd.exe");
        assert!(!policy.is_cmd_exe());
    }
}

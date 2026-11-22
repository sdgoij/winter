//! ConPTY — the Windows pseudo terminal.
//!
//! This is the port's own backend (ADR 016); upstream has no pty. It creates a
//! pseudoconsole, starts the client attached to it, and keeps the two pipe ends
//! the terminal reads and writes.
//!
//! `unsafe` is confined to this module by the allow on its declaration in
//! `lib.rs`.

use std::ffi::{OsStr, OsString, c_void};
use std::fs::File;
use std::io::{self, Read, Write};
use std::mem::{self, size_of};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::FromRawHandle;
use std::os::windows::process::ExitStatusExt;
use std::process::ExitStatus;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::Console::{COORD, ClosePseudoConsole, CreatePseudoConsole, HPCON, ResizePseudoConsole};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, INFINITE,
    InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
    PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW, TerminateProcess, UpdateProcThreadAttribute,
    WaitForSingleObject,
};

use crate::device_comm::DeviceComm;
use crate::pty::{PtyCommand, PtySize};

/// A running client attached to a ConPTY pseudoconsole.
pub struct Pty {
    /// The write end of the client's input pipe.
    input: File,
    /// The read end of the client's output pipe.
    output: File,
    /// The pseudoconsole; closed once, after the client exits.
    pseudo_console: HPCON,
    /// The client process.
    process: HANDLE,
    /// Whether the client has been waited for.
    reaped: bool,
}

impl Pty {
    /// Starts `command` in a new pseudoconsole of `size`.
    pub fn spawn(command: &PtyCommand, size: PtySize) -> io::Result<Self> {
        let (input_read, input_write) = create_pipe()?;
        let (output_read, output_write) = create_pipe()?;

        let mut pseudo_console: HPCON = 0;
        // SAFETY: the four handles are live pipes we own, and the out-pointer
        // is a valid HPCON slot.
        let result = unsafe { CreatePseudoConsole(coord(size), input_read, output_write, 0, &mut pseudo_console) };
        if result < 0 {
            // The pseudoconsole did not take the handles.
            close(input_read);
            close(input_write);
            close(output_read);
            close(output_write);
            return Err(hresult_error(result));
        }

        // ConPTY holds its own copies; ours are no longer needed.
        close(input_read);
        close(output_write);

        // SAFETY: the attribute list is built, used and deleted within this
        // function; every pointer is valid for the call it is passed to.
        let process = unsafe { start_client(command, pseudo_console) };
        let process = match process {
            Ok(process) => process,
            Err(error) => {
                // SAFETY: the pseudoconsole is live and owned here.
                unsafe { ClosePseudoConsole(pseudo_console) };
                close(input_write);
                close(output_read);
                return Err(error);
            }
        };

        // SAFETY: both handles are valid pipe ends and are owned by the files.
        let input = unsafe { File::from_raw_handle(input_write) };
        // SAFETY: as above.
        let output = unsafe { File::from_raw_handle(output_read) };

        let mut pty = Self {
            input,
            output,
            pseudo_console,
            process,
            reaped: false,
        };
        pty.resize(size)?;
        Ok(pty)
    }

    /// Returns an independent handle for reading the client's output.
    ///
    /// A terminal reads on its own thread while it waits for the client, so
    /// the read end can be duplicated and moved out of the `Pty`.
    pub fn try_clone_reader(&self) -> io::Result<PtyReader> {
        Ok(PtyReader(self.output.try_clone()?))
    }

    /// Closes the pseudoconsole and ends the read side.
    ///
    /// A reader blocked in `read` returns end of file once this is called.
    pub fn close(&mut self) {
        self.close_pseudo_console();
    }

    /// Resizes the pseudoconsole.
    pub fn resize(&mut self, size: PtySize) -> io::Result<()> {
        // SAFETY: the pseudoconsole is live until `close_pseudo_console`.
        let result = unsafe { ResizePseudoConsole(self.pseudo_console, coord(size)) };
        if result < 0 { Err(hresult_error(result)) } else { Ok(()) }
    }

    /// Waits for the client to exit and returns its status.
    ///
    /// The pseudoconsole stays open so the output pipe can be drained to its
    /// end of file first; `Drop` closes it.
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        // SAFETY: the process handle is live until the drop.
        let wait = unsafe { WaitForSingleObject(self.process, INFINITE) };
        if wait != 0 {
            return Err(io::Error::last_os_error());
        }

        let mut code: u32 = 0;
        // SAFETY: the handle and the out-pointer are valid.
        if unsafe { GetExitCodeProcess(self.process, &mut code) } == 0 {
            return Err(io::Error::last_os_error());
        }

        self.reaped = true;
        Ok(ExitStatus::from_raw(code))
    }

    /// Terminates the client.
    pub fn kill(&mut self) -> io::Result<()> {
        // SAFETY: the process handle is live.
        if unsafe { TerminateProcess(self.process, 1) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let _ = self.wait();
        Ok(())
    }

    /// Returns the client's exit status once it has exited, without blocking.
    ///
    /// The pseudoconsole keeps its output pipe open after the client exits, so
    /// this is the only way to notice that the shell is gone.
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        // SAFETY: the process handle is live until the drop, and a zero
        // timeout makes the call a poll.
        match unsafe { WaitForSingleObject(self.process, 0) } {
            WAIT_OBJECT_0 => {
                let mut code: u32 = 0;
                // SAFETY: the handle and the out-pointer are valid.
                if unsafe { GetExitCodeProcess(self.process, &mut code) } == 0 {
                    return Err(io::Error::last_os_error());
                }
                self.reaped = true;
                Ok(Some(ExitStatus::from_raw(code)))
            }
            WAIT_TIMEOUT => Ok(None),
            _ => Err(io::Error::last_os_error()),
        }
    }

    /// Closes the pseudoconsole exactly once.
    fn close_pseudo_console(&mut self) {
        if self.pseudo_console != 0 {
            // SAFETY: the handle is live and cleared below, so it is closed
            // once.
            unsafe { ClosePseudoConsole(self.pseudo_console) };
            self.pseudo_console = 0;
        }
    }
}

impl Read for Pty {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.output.read(buf)
    }
}

/// The read half of a [`Pty`], safe to move to another thread.
pub struct PtyReader(File);

impl Read for PtyReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl Write for Pty {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.input.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.input.flush()
    }
}

impl DeviceComm for Pty {
    fn resize(&mut self, size: PtySize) -> io::Result<()> {
        Self::resize(self, size)
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        if !self.reaped {
            // Closing the pseudoconsole with a live client can block, so stop
            // it first.
            let _ = self.kill();
        }
        self.close_pseudo_console();
        if !self.process.is_null() {
            close(self.process);
            self.process = null_mut();
        }
    }
}

/// Starts the client with the pseudoconsole attached.
///
/// # Safety
///
/// `pseudo_console` must be a live pseudoconsole. The attribute list, command
/// line and startup information are all live for the duration of the call.
unsafe fn start_client(command: &PtyCommand, pseudo_console: HPCON) -> io::Result<HANDLE> {
    let mut attribute_size: usize = 0;
    // SAFETY: a null list with a size pointer is the documented size query.
    unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut attribute_size) };
    if attribute_size == 0 {
        return Err(io::Error::last_os_error());
    }

    // The list is opaque; 16-byte alignment is plenty for it.
    let mut buffer = vec![0u128; attribute_size.div_ceil(size_of::<u128>())];
    let attribute_list = buffer.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;

    // SAFETY: the buffer is at least `attribute_size` bytes.
    if unsafe { InitializeProcThreadAttributeList(attribute_list, 1, 0, &mut attribute_size) } == 0 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: the list is initialized with room for one attribute, and the
    // value outlives the call.
    let updated = unsafe {
        UpdateProcThreadAttribute(
            attribute_list,
            0,
            PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
            pseudo_console as *const c_void,
            size_of::<HPCON>(),
            null_mut(),
            null_mut(),
        )
    };
    if updated == 0 {
        // SAFETY: the list was initialized above.
        unsafe { DeleteProcThreadAttributeList(attribute_list) };
        return Err(io::Error::last_os_error());
    }

    // SAFETY: a zeroed STARTUPINFOEXW with its cb filled in is the documented
    // initial state.
    let mut startup: STARTUPINFOEXW = unsafe { mem::zeroed() };
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    // Null standard handles with this flag mean the client inherits none; the
    // pseudoconsole supplies its console handles instead. Without the flag the
    // client inherits this process's console and its output escapes the pty.
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.lpAttributeList = attribute_list;

    let mut command_line = wide(&command_line(command));
    // SAFETY: a zeroed PROCESS_INFORMATION is the documented initial state.
    let mut process_info: PROCESS_INFORMATION = unsafe { mem::zeroed() };

    // SAFETY: every pointer refers to live storage for the call; the command
    // line is mutable as required.
    let created = unsafe {
        CreateProcessW(
            std::ptr::null(),
            command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            EXTENDED_STARTUPINFO_PRESENT,
            std::ptr::null(),
            std::ptr::null(),
            &startup.StartupInfo,
            &mut process_info,
        )
    };

    // SAFETY: the list was initialized above and is no longer needed.
    unsafe { DeleteProcThreadAttributeList(attribute_list) };

    if created == 0 {
        return Err(io::Error::last_os_error());
    }

    // The thread handle is not used.
    close(process_info.hThread);
    Ok(process_info.hProcess)
}

/// Builds the command line `CreateProcessW` parses.
fn command_line(command: &PtyCommand) -> OsString {
    let mut line = quote(command.program());
    for argument in command.arguments() {
        line.push(' ');
        line.push_str(&quote(argument));
    }
    OsString::from(line)
}

/// Quotes one command-line argument.
///
/// Follows the MSVC rules: an argument with whitespace or a quote is wrapped,
/// backslashes before a quote are doubled, and trailing backslashes are doubled
/// before the closing quote.
fn quote(argument: &OsStr) -> String {
    let text = argument.to_string_lossy();
    if !text.is_empty() && !text.contains([' ', '\t', '"']) {
        return text.into_owned();
    }

    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    let mut backslashes = 0;
    for character in text.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                quoted.push_str(&"\\".repeat(backslashes));
                quoted.push(character);
                backslashes = 0;
            }
        }
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
}

/// Converts a size to the `COORD` ConPTY expects.
fn coord(size: PtySize) -> COORD {
    COORD {
        X: size.cols as i16,
        Y: size.rows as i16,
    }
}

/// Creates an anonymous pipe.
fn create_pipe() -> io::Result<(HANDLE, HANDLE)> {
    let mut read: HANDLE = null_mut();
    let mut write: HANDLE = null_mut();
    // SAFETY: both handles are valid out-pointers.
    if unsafe { CreatePipe(&mut read, &mut write, std::ptr::null(), 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((read, write))
}

/// Closes a handle, ignoring failure.
fn close(handle: HANDLE) {
    if !handle.is_null() {
        // SAFETY: the handle is non-null and owned by the caller.
        unsafe { CloseHandle(handle) };
    }
}

/// Wraps an `OsStr` for a wide-character API.
fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(std::iter::once(0)).collect()
}

/// Converts a failed HRESULT to an `io::Error`.
fn hresult_error(result: i32) -> io::Error {
    io::Error::other(format!("HRESULT {result:#010x}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell() -> OsString {
        std::env::var_os("COMSPEC").unwrap_or_else(|| OsString::from("cmd.exe"))
    }

    #[test]
    fn arguments_are_quoted_for_create_process() {
        assert_eq!(quote(OsStr::new("cmd.exe")), "cmd.exe");
        assert_eq!(quote(OsStr::new("echo hi")), "\"echo hi\"");
        assert_eq!(quote(OsStr::new("")), "\"\"");
        assert_eq!(quote(OsStr::new("a\"b")), "\"a\\\"b\"");
        assert_eq!(quote(OsStr::new(r"a\b")), r"a\b");
        assert_eq!(quote(OsStr::new(r"a b\")), "\"a b\\\\\"");
    }

    #[test]
    fn a_command_line_joins_program_and_arguments() {
        let command = PtyCommand::new("program.exe").args(["one", "two words"]);
        assert_eq!(command_line(&command), OsString::from("program.exe one \"two words\""));
    }

    #[test]
    fn a_command_writes_to_its_output() {
        let mut pty = Pty::spawn(
            &PtyCommand::new(shell()).args(["/d", "/c", "echo hello"]),
            PtySize::default(),
        )
        .expect("spawn");

        let status = pty.wait().expect("wait");
        assert!(status.success());
        pty.close();

        let mut output = Vec::new();
        pty.read_to_end(&mut output).expect("read");
        let text = String::from_utf8_lossy(&output);
        assert!(text.contains("hello"), "output was {text:?}");
    }

    #[test]
    fn the_exit_status_is_reported() {
        let mut pty = Pty::spawn(
            &PtyCommand::new(shell()).args(["/d", "/c", "exit 3"]),
            PtySize::default(),
        )
        .expect("spawn");

        assert_eq!(pty.wait().expect("wait").code(), Some(3));
    }

    #[test]
    fn input_reaches_the_client() {
        let mut pty = Pty::spawn(
            &PtyCommand::new(shell()).args(["/v:on", "/d", "/c", "set /p x= & echo got:!x!"]),
            PtySize::default(),
        )
        .expect("spawn");

        pty.write_all(b"abc\r").expect("write");

        let status = pty.wait().expect("wait");
        assert!(status.success());
        pty.close();

        let mut output = Vec::new();
        pty.read_to_end(&mut output).expect("read");
        let text = String::from_utf8_lossy(&output);
        assert!(text.contains("got:abc"), "output was {text:?}");
    }

    #[test]
    fn resize_is_accepted_while_the_client_runs() {
        let mut pty = Pty::spawn(
            &PtyCommand::new(shell()).args(["/d", "/c", "set /p x="]),
            PtySize::default(),
        )
        .expect("spawn");

        pty.resize(PtySize::new(100, 30)).expect("resize");
        pty.write_all(b"\r").expect("write");
        pty.wait().expect("wait");
        pty.close();
        let mut output = Vec::new();
        pty.read_to_end(&mut output).expect("read");
    }
}

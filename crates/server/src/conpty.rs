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
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{
    CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, HANDLE_FLAG_INHERIT,
    SetHandleInformation, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Storage::FileSystem::{SYNCHRONIZE, WriteFile};
use windows_sys::Win32::System::Console::{COORD, ClosePseudoConsole, CreatePseudoConsole, HPCON, ResizePseudoConsole};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess, GetExitCodeProcess,
    INFINITE, InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOEXW, TerminateProcess,
    UpdateProcThreadAttribute, WaitForSingleObject,
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
    console: Console,
    /// The client process.
    process: HANDLE,
    /// Whether the client has been waited for.
    reaped: bool,
}

/// The console host behind a pty.
enum Console {
    /// The inbox `CreatePseudoConsole`, which keeps a screen buffer and
    /// re-renders it as VT.
    Inbox(HPCON),
    /// A headless `OpenConsole.exe` started by this process. Its newer VT
    /// engine forwards sequences it cannot render, which is what carries an
    /// image protocol to the terminal (ADR 055).
    Packed(Packed),
}

/// The ABI `HPCON` points at.
///
/// The OS reads the reference handle out of this structure when it processes
/// `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`, which is how the client gets its
/// console. `winconpty` uses the same layout.
#[repr(C)]
struct Packed {
    /// The out-of-band signal pipe, our side.
    signal: HANDLE,
    /// What console clients inherit; closing it lets the host exit.
    reference: HANDLE,
    /// The console host process.
    host: HANDLE,
}

impl Console {
    /// The value `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE` takes.
    ///
    /// It is only valid while the `Console` is alive and has not moved, which
    /// is the duration of one `CreateProcessW` call.
    fn attribute(&self) -> HPCON {
        match self {
            Console::Inbox(pseudo_console) => *pseudo_console,
            Console::Packed(packed) => std::ptr::from_ref(packed) as HPCON,
        }
    }

    fn resize(&mut self, size: PtySize) -> io::Result<()> {
        match self {
            Console::Inbox(pseudo_console) => {
                // SAFETY: the pseudoconsole is live until `close`.
                let result = unsafe { ResizePseudoConsole(*pseudo_console, coord(size)) };
                if result < 0 { Err(hresult_error(result)) } else { Ok(()) }
            }
            Console::Packed(packed) => {
                // The host reads three machine integers, not a COORD.
                let packet = [PTY_SIGNAL_RESIZE_WINDOW, size.cols, size.rows];
                let mut written = 0u32;
                // SAFETY: the signal handle is live and owned by the console.
                let result = unsafe {
                    WriteFile(
                        packed.signal,
                        packet.as_ptr().cast(),
                        size_of::<[u16; 3]>() as u32,
                        &mut written,
                        null_mut(),
                    )
                };
                if result == 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(())
                }
            }
        }
    }

    /// Closes the console exactly once; clearing the handles makes it idempotent.
    fn close(&mut self) {
        match self {
            Console::Inbox(pseudo_console) => {
                if *pseudo_console != 0 {
                    // SAFETY: the handle is live and cleared below.
                    unsafe { ClosePseudoConsole(*pseudo_console) };
                    *pseudo_console = 0;
                }
            }
            Console::Packed(packed) => {
                close(packed.signal);
                packed.signal = null_mut();
                close(packed.reference);
                packed.reference = null_mut();
                close(packed.host);
                packed.host = null_mut();
            }
        }
    }
}

impl Pty {
    /// Starts `command` in a new pseudoconsole of `size`.
    pub fn spawn(command: &PtyCommand, size: PtySize) -> io::Result<Self> {
        let (input_read, input_write) = create_pipe()?;
        let (output_read, output_write) = create_pipe()?;

        let mut console = match packed_console(size, input_read, output_write) {
            Ok(Some(console)) => console,
            // No console host to launch, or it would not start: the inbox
            // pseudoconsole still runs the session, it just cannot forward an
            // image sequence (ADR 055).
            Ok(None) | Err(_) => match inbox_console(size, input_read, output_write) {
                Ok(console) => console,
                Err(error) => {
                    close(input_read);
                    close(input_write);
                    close(output_read);
                    close(output_write);
                    return Err(error);
                }
            },
        };

        // The console holds its own copies; ours are no longer needed.
        close(input_read);
        close(output_write);

        let attribute = console.attribute();
        // SAFETY: `start_client` builds and deletes its own attribute list, and
        // the console — which `attribute` points into — outlives the call.
        let process = match unsafe { start_client(command, attribute) } {
            Ok(process) => process,
            Err(error) => {
                console.close();
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
            console,
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
        self.console.resize(size)
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
        self.console.close();
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

/// `PTY_SIGNAL_RESIZE_WINDOW`, one of the out-of-band messages the headless
/// console host reads from its signal pipe.
const PTY_SIGNAL_RESIZE_WINDOW: u16 = 8;

/// `GENERIC_ALL`, which the console server handle is opened with.
const GENERIC_ALL: u32 = 0x1000_0000;
/// The object attributes the console device is opened with.
const OBJ_CASE_INSENSITIVE: u32 = 0x40;
const OBJ_INHERIT: u32 = 0x2;
/// The share and open options for the console device.
const FILE_SHARE_READ_WRITE_DELETE: u32 = 0x7;
const FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x20;

/// `ntdll!NtOpenFile`, the only way to open a console device handle.
type NtOpenFile =
    unsafe extern "system" fn(*mut HANDLE, u32, *const ObjectAttributes, *mut IoStatusBlock, u32, u32) -> i32;

#[repr(C)]
struct UnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *mut u16,
}

#[repr(C)]
struct ObjectAttributes {
    length: u32,
    root_directory: HANDLE,
    object_name: *mut UnicodeString,
    attributes: u32,
    security_descriptor: *mut c_void,
    security_quality_of_service: *mut c_void,
}

#[repr(C)]
struct IoStatusBlock {
    status: isize,
    information: usize,
}

/// The inbox pseudoconsole, which renders the client's screen itself.
fn inbox_console(size: PtySize, input: HANDLE, output: HANDLE) -> io::Result<Console> {
    let mut pseudo_console: HPCON = 0;
    // SAFETY: the handles are live pipes we own, and the out-pointer is a
    // valid HPCON slot.
    let result = unsafe { CreatePseudoConsole(coord(size), input, output, 0, &mut pseudo_console) };
    if result < 0 {
        return Err(hresult_error(result));
    }
    Ok(Console::Inbox(pseudo_console))
}

/// Starts a headless `OpenConsole.exe` and returns the console it hosts.
///
/// `Ok(None)` means no console host is installed; the caller falls back to the
/// inbox pseudoconsole.
fn packed_console(size: PtySize, input: HANDLE, output: HANDLE) -> io::Result<Option<Console>> {
    let Some(host) = console_host() else {
        return Ok(None);
    };

    let server = open_console_device("\\Device\\ConDrv\\Server", GENERIC_ALL, null_mut(), true, 0)?;
    let reference = match open_console_device(
        "\\Reference",
        GENERIC_READ | GENERIC_WRITE | SYNCHRONIZE,
        server,
        false,
        FILE_SYNCHRONOUS_IO_NONALERT,
    ) {
        Ok(reference) => reference,
        Err(error) => {
            close(server);
            return Err(error);
        }
    };

    let (signal_host, signal_ours) = match create_signal_pipe() {
        Ok((signal_host, signal_ours)) => (signal_host, signal_ours),
        Err(error) => {
            close(server);
            close(reference);
            return Err(error);
        }
    };

    let input_copy = match duplicate_inheritable(input) {
        Ok(input_copy) => input_copy,
        Err(error) => {
            close(server);
            close(reference);
            close(signal_host);
            close(signal_ours);
            return Err(error);
        }
    };
    let output_copy = match duplicate_inheritable(output) {
        Ok(output_copy) => output_copy,
        Err(error) => {
            close(server);
            close(reference);
            close(signal_host);
            close(signal_ours);
            close(input_copy);
            return Err(error);
        }
    };

    let host_process = start_host(host.as_os_str(), size, signal_host, server, input_copy, output_copy);

    // The host inherited its copies; ours are done either way.
    close(server);
    close(signal_host);
    close(input_copy);
    close(output_copy);

    let host_process = match host_process {
        Ok(process) => process,
        Err(error) => {
            close(reference);
            close(signal_ours);
            return Err(error);
        }
    };

    Ok(Some(Console::Packed(Packed {
        signal: signal_ours,
        reference,
        host: host_process,
    })))
}

/// Finds a console host that can forward an image sequence.
///
/// The inbox `conhost.exe` cannot; the newer `OpenConsole.exe` can. It is
/// looked for where the environment points, beside this executable, in its
/// `x64` subdirectory, and finally in the app's cache directory.
///
/// A packaged release has the host beside `winter.exe`, so it needs neither
/// Windows Terminal nor the cache (ADR 055).
fn console_host() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let directory = executable.parent()?;
    let configured = std::env::var_os("WINTER_CONSOLE_HOST").map(PathBuf::from);
    let cache = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    console_host_in(directory, configured.as_deref(), cache.as_deref())
}

/// Picks the first host that exists from the documented order.
///
/// The sources are arguments rather than lookups, so the order can be tested
/// without touching the environment or this executable.
fn console_host_in(directory: &Path, configured: Option<&Path>, local_app_data: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = configured.filter(|path| path.is_file()) {
        return Some(path.to_path_buf());
    }

    for candidate in [
        directory.join("OpenConsole.exe"),
        directory.join("x64").join("OpenConsole.exe"),
    ] {
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    let cached = local_app_data?.join("winter").join("OpenConsole.exe");
    cached.is_file().then_some(cached)
}

/// Launches the console host attached to our handles.
fn start_host(
    host: &OsStr,
    size: PtySize,
    signal: HANDLE,
    server: HANDLE,
    input: HANDLE,
    output: HANDLE,
) -> io::Result<HANDLE> {
    let mut command = format!(
        "{} --headless --width {} --height {} --signal 0x{:x} --server 0x{:x}",
        quote(host),
        size.cols,
        size.rows,
        signal as usize,
        server as usize,
    )
    .encode_utf16()
    .chain(std::iter::once(0))
    .collect::<Vec<u16>>();

    // SAFETY: a zeroed STARTUPINFOEXW with its cb filled in is the documented
    // initial state.
    let mut startup: STARTUPINFOEXW = unsafe { mem::zeroed() };
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = input;
    startup.StartupInfo.hStdOutput = output;
    startup.StartupInfo.hStdError = output;

    // Only the handles the host needs may cross the process boundary.
    let inherited = [server, input, output, signal];

    let mut attribute_size = 0usize;
    // SAFETY: a null list with a size pointer is the documented size query.
    unsafe { InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut attribute_size) };
    if attribute_size == 0 {
        return Err(io::Error::last_os_error());
    }

    let mut buffer = vec![0u128; attribute_size.div_ceil(size_of::<u128>())];
    let attribute_list = buffer.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;
    // SAFETY: the buffer is at least `attribute_size` bytes.
    if unsafe { InitializeProcThreadAttributeList(attribute_list, 1, 0, &mut attribute_size) } == 0 {
        return Err(io::Error::last_os_error());
    }
    startup.lpAttributeList = attribute_list;

    // SAFETY: the list has room for one attribute and the slice outlives it.
    let updated = unsafe {
        UpdateProcThreadAttribute(
            attribute_list,
            0,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
            inherited.as_ptr().cast(),
            size_of::<[HANDLE; 4]>(),
            null_mut(),
            null_mut(),
        )
    };
    if updated == 0 {
        // SAFETY: the list was initialized above.
        unsafe { DeleteProcThreadAttributeList(attribute_list) };
        return Err(io::Error::last_os_error());
    }

    // SAFETY: every pointer refers to live storage for the call; the command
    // line is mutable as required, and the handles are inheritable.
    let mut process_info: PROCESS_INFORMATION = unsafe { mem::zeroed() };
    let created = unsafe {
        CreateProcessW(
            std::ptr::null(),
            command.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
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
    close(process_info.hThread);
    Ok(process_info.hProcess)
}

/// Opens a console device object and returns its handle.
fn open_console_device(name: &str, access: u32, parent: HANDLE, inheritable: bool, options: u32) -> io::Result<HANDLE> {
    let open = nt_open_file().ok_or_else(|| io::Error::other("ntdll!NtOpenFile is missing"))?;

    let mut buffer: Vec<u16> = name.encode_utf16().collect();
    let mut object_name = UnicodeString {
        length: (buffer.len() * size_of::<u16>()) as u16,
        maximum_length: ((buffer.len() + 1) * size_of::<u16>()) as u16,
        buffer: buffer.as_mut_ptr(),
    };
    let attributes = ObjectAttributes {
        length: size_of::<ObjectAttributes>() as u32,
        root_directory: parent,
        object_name: std::ptr::from_mut(&mut object_name),
        attributes: OBJ_CASE_INSENSITIVE | if inheritable { OBJ_INHERIT } else { 0 },
        security_descriptor: null_mut(),
        security_quality_of_service: null_mut(),
    };
    let mut status = IoStatusBlock {
        status: 0,
        information: 0,
    };
    let mut handle: HANDLE = null_mut();

    // SAFETY: every pointer refers to live storage for the call; the name
    // buffer outlives it.
    let result = unsafe {
        open(
            &mut handle,
            access,
            &attributes,
            &mut status,
            FILE_SHARE_READ_WRITE_DELETE,
            options,
        )
    };
    if result < 0 {
        return Err(io::Error::other(format!("NtOpenFile({name}) failed: {result:#010x}")));
    }
    Ok(handle)
}

/// Loads `ntdll!NtOpenFile` once.
fn nt_open_file() -> Option<NtOpenFile> {
    static LOADED: std::sync::OnceLock<Option<NtOpenFile>> = std::sync::OnceLock::new();

    *LOADED.get_or_init(|| {
        // SAFETY: the module name is NUL-terminated UTF-16, and the symbol
        // has the documented NtOpenFile signature.
        unsafe {
            let ntdll_name: Vec<u16> = "ntdll.dll".encode_utf16().chain(std::iter::once(0)).collect();
            let ntdll = GetModuleHandleW(ntdll_name.as_ptr());
            if ntdll.is_null() {
                return None;
            }
            let symbol = GetProcAddress(ntdll, c"NtOpenFile".as_ptr().cast());
            symbol.map(|function| mem::transmute::<_, NtOpenFile>(function))
        }
    })
}

/// Creates the out-of-band signal pipe, with the host's end inheritable.
fn create_signal_pipe() -> io::Result<(HANDLE, HANDLE)> {
    let (host, ours) = create_pipe()?;
    // SAFETY: the handle is live and owned here.
    if unsafe { SetHandleInformation(host, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) } == 0 {
        let error = io::Error::last_os_error();
        close(host);
        close(ours);
        return Err(error);
    }
    Ok((host, ours))
}

/// Duplicates a handle so the console host can inherit it.
fn duplicate_inheritable(handle: HANDLE) -> io::Result<HANDLE> {
    let mut copy: HANDLE = null_mut();
    // SAFETY: both process handles are the current process, and the out-pointer
    // is valid.
    let result = unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            handle,
            GetCurrentProcess(),
            &mut copy,
            0,
            1,
            DUPLICATE_SAME_ACCESS,
        )
    };
    if result == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(copy)
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

    /// A scratch directory that removes itself.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("winter-host-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("scratch directory");
            Self(path)
        }

        fn touch(&self, relative: &str) -> PathBuf {
            let path = self.0.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("parent directory");
            }
            std::fs::write(&path, b"host").expect("host file");
            path
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_console_host_beside_the_executable_is_preferred() {
        let exe = Scratch::new("beside");
        let cache = Scratch::new("beside-cache");
        let beside = exe.touch("OpenConsole.exe");
        cache.touch("winter/OpenConsole.exe");

        assert_eq!(console_host_in(exe.path(), None, Some(cache.path())), Some(beside));
    }

    #[test]
    fn a_configured_host_wins_over_the_sibling() {
        let exe = Scratch::new("configured");
        let other = Scratch::new("configured-other");
        exe.touch("OpenConsole.exe");
        let configured = other.touch("OpenConsole.exe");

        assert_eq!(console_host_in(exe.path(), Some(&configured), None), Some(configured));
    }

    #[test]
    fn a_configured_path_that_is_not_a_file_is_ignored() {
        let exe = Scratch::new("configured-missing");
        let beside = exe.touch("OpenConsole.exe");

        assert_eq!(
            console_host_in(exe.path(), Some(Path::new("does-not-exist.exe")), None),
            Some(beside)
        );
    }

    #[test]
    fn the_x64_directory_is_a_fallback() {
        let exe = Scratch::new("x64");
        let nested = exe.touch("x64/OpenConsole.exe");

        assert_eq!(console_host_in(exe.path(), None, None), Some(nested));
    }

    #[test]
    fn the_app_cache_is_the_last_resort() {
        let exe = Scratch::new("cache");
        let cache = Scratch::new("cache-store");
        assert_eq!(console_host_in(exe.path(), None, Some(cache.path())), None);

        let cached = cache.touch("winter/OpenConsole.exe");
        assert_eq!(console_host_in(exe.path(), None, Some(cache.path())), Some(cached));
    }

    // Requires a console host that forwards image sequences. Point
    // `WINTER_CONSOLE_HOST` at one (the packaged OpenConsole.exe) and run with
    // `--ignored`.
    #[test]
    #[ignore = "needs WINTER_CONSOLE_HOST pointing at OpenConsole.exe"]
    fn a_packed_console_forwards_an_image_sequence() {
        if std::env::var_os("WINTER_CONSOLE_HOST").is_none() {
            return;
        }

        let script = r#"
Add-Type -Namespace N -Name K -MemberDefinition '[DllImport("kernel32.dll")] public static extern IntPtr GetStdHandle(int n); [DllImport("kernel32.dll")] public static extern bool GetConsoleMode(IntPtr h, out uint m); [DllImport("kernel32.dll")] public static extern bool SetConsoleMode(IntPtr h, uint m);'
$h = [N.K]::GetStdHandle(-11)
[uint32]$mode = 0
[N.K]::GetConsoleMode($h, [ref]$mode) | Out-Null
[N.K]::SetConsoleMode($h, $mode -bor 4) | Out-Null
$b = [byte[]](27,80,113,27,92,27,95,71,97,27,92)
$s = [Console]::OpenStandardOutput()
$s.Write($b, 0, $b.Length)
$s.Flush()
"#;
        let path = std::env::temp_dir().join(format!("winter-image-{}.ps1", std::process::id()));
        std::fs::write(&path, script).expect("script");

        let mut pty = Pty::spawn(
            &PtyCommand::new("powershell.exe")
                .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
                .arg(&path),
            PtySize::default(),
        )
        .expect("spawn");

        let mut reader = pty.try_clone_reader().expect("reader");
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        let sink = std::sync::Arc::clone(&seen);
        let _reading = std::thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            loop {
                match reader.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(read) => {
                        sink.lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .extend_from_slice(&chunk[..read]);
                    }
                    Err(_) => break,
                }
            }
        });

        std::thread::sleep(std::time::Duration::from_secs(5));
        let output = seen.lock().unwrap_or_else(|error| error.into_inner()).clone();
        let _ = pty.kill();
        let _ = std::fs::remove_file(&path);

        assert!(
            output.windows(3).any(|window| window == b"\x1bPq") || output.windows(3).any(|window| window == b"\x1b_G"),
            "the image sequences did not survive the console host: {:?}",
            String::from_utf8_lossy(&output)
        );
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

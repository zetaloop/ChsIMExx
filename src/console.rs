use std::mem::size_of;

use windows::Win32::{
    Foundation::{
        ERROR_ACCESS_DENIED, ERROR_INVALID_HANDLE, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
    },
    Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation},
    System::{
        Console::{
            ATTACH_PARENT_PROCESS, AllocConsole, AttachConsole, FreeConsole, GetStdHandle,
            STD_OUTPUT_HANDLE, WriteConsoleW,
        },
        Threading::{GetCurrentProcess, OpenProcessToken},
    },
};

pub struct ConsoleSession {
    release_on_drop: bool,
}

impl ConsoleSession {
    pub fn attach_temporary() -> Option<Self> {
        Self::attach(true)
    }

    pub fn ensure() -> Option<Self> {
        if let Some(session) = Self::attach(false) {
            return Some(session);
        }
        unsafe {
            if AllocConsole().is_ok() {
                Some(Self {
                    release_on_drop: false,
                })
            } else {
                None
            }
        }
    }

    fn attach(release_on_drop: bool) -> Option<Self> {
        unsafe {
            match AttachConsole(ATTACH_PARENT_PROCESS) {
                Ok(()) => Some(Self { release_on_drop }),
                Err(_) => match GetLastError() {
                    err if err == ERROR_ACCESS_DENIED => Some(Self {
                        release_on_drop: false,
                    }),
                    err if err == ERROR_INVALID_HANDLE => None,
                    _ => None,
                },
            }
        }
    }

    pub fn println(&self, message: &str) {
        write_console_line(message);
    }
}

impl Drop for ConsoleSession {
    fn drop(&mut self) {
        if self.release_on_drop {
            let _ = unsafe { FreeConsole() };
        }
    }
}

fn write_console_line(message: &str) {
    unsafe {
        let handle = match GetStdHandle(STD_OUTPUT_HANDLE) {
            Ok(handle) => handle,
            Err(_) => return,
        };
        if handle == INVALID_HANDLE_VALUE || handle.is_invalid() {
            return;
        }
        let mut buffer: Vec<u16> = message.encode_utf16().collect();
        buffer.push('\r' as u16);
        buffer.push('\n' as u16);
        let mut written = 0;
        let _ = WriteConsoleW(handle, &buffer, Some(&mut written), None);
    }
}

fn is_elevated() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
        .is_ok();
        let _ = windows::Win32::Foundation::CloseHandle(token);
        ok && elevation.TokenIsElevated != 0
    }
}

pub fn console_prefix() -> &'static str {
    if is_elevated() {
        "[ChsIMExx Admin]"
    } else {
        "[ChsIMExx]"
    }
}

pub fn toast_title() -> &'static str {
    if is_elevated() {
        "ChsIME++ (Admin)"
    } else {
        "ChsIME++"
    }
}

pub fn log_to_console(message: &str) {
    if let Some(console) = ConsoleSession::attach_temporary() {
        console.println(&format!("\r\n{} {message}", console_prefix()));
    }
}

pub fn log_error(message: &str) {
    eprintln!("{message}");
    let prefix = console_prefix();
    if let Some(console) = ConsoleSession::attach_temporary() {
        console.println(&format!("\r\n{prefix}[错误] {message}"));
    } else if let Some(console) = ConsoleSession::ensure() {
        console.println(&format!("\r\n{prefix}[错误] {message}"));
    }
}

#![windows_subsystem = "windows"]

use std::{env, mem::size_of, process};

use windows::{
    Data::Xml::Dom::XmlDocument,
    UI::Notifications::{ToastNotification, ToastNotificationManager},
    Win32::{
        Foundation::{
            CloseHandle, ERROR_ACCESS_DENIED, ERROR_INVALID_HANDLE, GetLastError, HANDLE, HWND,
            INVALID_HANDLE_VALUE, LPARAM, LRESULT, WAIT_ABANDONED, WAIT_EVENT, WAIT_FAILED,
            WAIT_OBJECT_0, WAIT_TIMEOUT, WPARAM,
        },
        System::Console::{
            ATTACH_PARENT_PROCESS, AllocConsole, AttachConsole, FreeConsole, GetStdHandle,
            STD_OUTPUT_HANDLE, WriteConsoleW,
        },
        System::Threading::{
            CreateEventW, CreateMutexW, EVENT_MODIFY_STATE, INFINITE, MUTEX_MODIFY_STATE,
            OpenEventW, OpenMutexW, ReleaseMutex, ResetEvent, SYNCHRONIZATION_SYNCHRONIZE,
            SetEvent, WaitForSingleObject,
        },
        UI::{
            Input::{
                Ime::{IME_CMODE_NATIVE, ImmGetDefaultIMEWnd},
                KeyboardAndMouse::{
                    GetAsyncKeyState, GetKeyboardLayout, HKL, INPUT, INPUT_0, INPUT_KEYBOARD,
                    KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, SendInput,
                    VIRTUAL_KEY,
                },
            },
            WindowsAndMessaging::{
                CallNextHookEx, DispatchMessageW, GetClassNameW, GetForegroundWindow,
                GetWindowThreadProcessId, HC_ACTION, KBDLLHOOKSTRUCT, MSG,
                MsgWaitForMultipleObjects, PM_REMOVE, PeekMessageW, PostMessageW, QS_ALLINPUT,
                SendMessageW, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx,
                WH_KEYBOARD_LL, WM_CHAR,
            },
        },
    },
    core::w,
    core::{self, HSTRING, PCWSTR},
};

const WM_KEYDOWN: u32 = 0x0100;
const WM_KEYUP: u32 = 0x0101;
const WM_SYSKEYDOWN: u32 = 0x0104;
const WM_SYSKEYUP: u32 = 0x0105;

const VK_SHIFT: i32 = 0x10;
const VK_CONTROL: i32 = 0x11;
const VK_MENU: i32 = 0x12; // Alt / AltGr
const VK_OEM_4: u32 = 0xDB; // [
const VK_OEM_6: u32 = 0xDD; // ]

const LLKHF_INJECTED: u32 = 0x10;
const LANG_CHINESE: u32 = 0x04; // 低 10 位是主语言 ID，0x04 代表中文

const STOP_EVENT_NAME: PCWSTR = w!("Global\\ChsIMExxStop");
const INSTANCE_MUTEX_NAME: PCWSTR = w!("Global\\ChsIMExxMutex");
const POWERSHELL_APP_ID: &str =
    "{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe";
const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    process::exit(match run() {
        Ok(()) => 0,
        Err(code) => code,
    });
}

fn run() -> Result<(), i32> {
    match parse_command()? {
        Command::Run => run_start(),
        Command::Stop => run_stop(),
        Command::Version => run_version(),
    }
}

enum Command {
    Run,
    Stop,
    Version,
}

fn parse_command() -> Result<Command, i32> {
    let mut args = env::args().skip(1);
    match args.next() {
        None => Ok(Command::Run),
        Some(arg) => {
            let cmd = arg.as_str();
            let rest_has_extra = args.next().is_some();

            let result = match cmd {
                "start" | "--start" => Some(Command::Run),
                "stop" | "--stop" => Some(Command::Stop),
                "version" | "--version" => Some(Command::Version),
                _ => None,
            };

            match (result, rest_has_extra) {
                (Some(command), false) => Ok(command),
                (Some(_), true) => {
                    log_error("额外参数无法识别");
                    Err(1)
                }
                (None, _) => {
                    log_error(&format!("未知参数：{cmd}"));
                    Err(1)
                }
            }
        }
    }
}

fn run_start() -> Result<(), i32> {
    let mut guard = InstanceGuard::new().map_err(|err| {
        log_error(&format!("创建同步对象失败: {err:?}"));
        1
    })?;

    let state = guard.acquire().map_err(|msg| {
        log_error(&msg);
        1
    })?;

    let hook = unsafe {
        SetWindowsHookExW(WH_KEYBOARD_LL, Some(low_level_keyboard_proc), None, 0).map_err(
            |err| {
                log_error(&format!("安装键盘钩子失败: {err:?}"));
                1
            },
        )?
    };

    let message = match state {
        InstanceState::Fresh => "已开启",
        InstanceState::Restarted => "已重新开启",
    };
    notify(message);
    log_to_console(message);

    unsafe {
        run_message_loop(guard.stop_event());
        let _ = UnhookWindowsHookEx(hook);
    }

    Ok(())
}

fn run_stop() -> Result<(), i32> {
    match signal_shutdown_request() {
        Ok(Some(_)) => {
            const MESSAGE: &str = "已关闭";
            notify(MESSAGE);
            log_to_console(MESSAGE);
            Ok(())
        }
        Ok(None) => {
            const MESSAGE: &str = "无需关闭";
            notify(MESSAGE);
            log_to_console(MESSAGE);
            Ok(())
        }
        Err(msg) => {
            log_error(&msg);
            Err(1)
        }
    }
}

fn run_version() -> Result<(), i32> {
    let message = format!("v{VERSION}");
    notify(&message);
    if let Some(console) = ConsoleSession::ensure() {
        console.println(&format!("\r\n[ChsIME] {message}"));
        Ok(())
    } else {
        Err(1)
    }
}

enum InstanceState {
    Fresh,
    Restarted,
}

struct InstanceGuard {
    mutex: HANDLE,
    stop_event: HANDLE,
    owns_mutex: bool,
}

impl InstanceGuard {
    fn new() -> core::Result<Self> {
        let mutex = unsafe { CreateMutexW(None, false, INSTANCE_MUTEX_NAME)? };
        let stop_event = unsafe { CreateEventW(None, true, false, STOP_EVENT_NAME)? };
        Ok(Self {
            mutex,
            stop_event,
            owns_mutex: false,
        })
    }

    fn acquire(&mut self) -> Result<InstanceState, String> {
        let wait = unsafe { WaitForSingleObject(self.mutex, 0) };
        if wait == WAIT_OBJECT_0 || wait == WAIT_ABANDONED {
            self.owns_mutex = true;
            unsafe {
                ResetEvent(self.stop_event).map_err(|_| "无法重置停止事件".to_string())?;
            }
            return Ok(InstanceState::Fresh);
        }

        if wait == WAIT_TIMEOUT {
            unsafe {
                SetEvent(self.stop_event).map_err(|_| "无法通知旧实例退出".to_string())?;
            }

            let wait = unsafe { WaitForSingleObject(self.mutex, 10_000) };
            if wait == WAIT_OBJECT_0 || wait == WAIT_ABANDONED {
                self.owns_mutex = true;
                unsafe {
                    ResetEvent(self.stop_event).map_err(|_| "无法重置停止事件".to_string())?;
                }
                return Ok(InstanceState::Restarted);
            } else if wait == WAIT_TIMEOUT {
                return Err("等待旧实例退出超时".into());
            } else {
                return Err("等待旧实例退出失败".into());
            }
        }

        if wait == WAIT_FAILED {
            Err("检测实例状态失败".into())
        } else {
            Err("未知的等待状态".into())
        }
    }

    fn stop_event(&self) -> HANDLE {
        self.stop_event
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = ResetEvent(self.stop_event);
            if self.owns_mutex {
                let _ = ReleaseMutex(self.mutex);
            }
            let _ = CloseHandle(self.mutex);
            let _ = CloseHandle(self.stop_event);
        }
    }
}

fn notify(message: &str) {
    if let Err(err) = send_toast(message) {
        log_error(&format!("发送通知失败: {err:?}"));
    }
}

fn log_to_console(message: &str) {
    if let Some(console) = ConsoleSession::attach_temporary() {
        console.println(&format!("\r\n[ChsIME] {message}"));
    }
}

fn log_error(message: &str) {
    eprintln!("{message}");
    if let Some(console) = ConsoleSession::attach_temporary() {
        console.println(&format!("\r\n[ChsIME][错误] {message}"));
    } else if let Some(console) = ConsoleSession::ensure() {
        console.println(&format!("\r\n[ChsIME][错误] {message}"));
    }
}

fn send_toast(message: &str) -> core::Result<()> {
    let xml = format!(
        "<toast><visual><binding template=\"ToastGeneric\"><text>ChsIMExx</text><text>{}</text></binding></visual></toast>",
        message
    );

    let doc = XmlDocument::new()?;
    doc.LoadXml(&HSTRING::from(xml))?;
    let toast = ToastNotification::CreateToastNotification(&doc)?;
    let notifier =
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(POWERSHELL_APP_ID))?;
    notifier.Show(&toast)?;
    Ok(())
}

fn run_message_loop(stop_event: HANDLE) {
    unsafe {
        let mut msg = MSG::default();
        let handles = [stop_event];
        let queue_index = WAIT_EVENT(WAIT_OBJECT_0.0 + handles.len() as u32);

        loop {
            let wait = MsgWaitForMultipleObjects(Some(&handles), false, INFINITE, QS_ALLINPUT);
            if wait == WAIT_OBJECT_0 {
                break;
            } else if wait == queue_index {
                drain_message_queue(&mut msg);
            } else if wait == WAIT_FAILED {
                break;
            } else {
                drain_message_queue(&mut msg);
            }
        }
    }
}

fn drain_message_queue(msg: &mut MSG) {
    unsafe {
        while PeekMessageW(msg, None, 0, 0, PM_REMOVE).into() {
            let _ = TranslateMessage(msg);
            DispatchMessageW(msg);
        }
    }
}

fn signal_shutdown_request() -> Result<Option<()>, String> {
    unsafe {
        let event = OpenEventW(
            EVENT_MODIFY_STATE | SYNCHRONIZATION_SYNCHRONIZE,
            false,
            STOP_EVENT_NAME,
        )
        .ok();

        if let Some(event) = event {
            SetEvent(event).map_err(|_| "无法发送停止请求".to_string())?;
            let _ = CloseHandle(event);

            if let Ok(mutex) = OpenMutexW(
                SYNCHRONIZATION_SYNCHRONIZE | MUTEX_MODIFY_STATE,
                false,
                INSTANCE_MUTEX_NAME,
            ) {
                let wait = WaitForSingleObject(mutex, 10_000);
                if wait == WAIT_OBJECT_0 || wait == WAIT_ABANDONED {
                    let _ = ReleaseMutex(mutex);
                }
                let _ = CloseHandle(mutex);
            }
            Ok(Some(()))
        } else {
            Ok(None)
        }
    }
}

struct ConsoleSession {
    release_on_drop: bool,
}

impl ConsoleSession {
    fn attach_temporary() -> Option<Self> {
        Self::attach(true)
    }

    fn ensure() -> Option<Self> {
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

    fn println(&self, message: &str) {
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

unsafe extern "system" fn low_level_keyboard_proc(
    n_code: i32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    if n_code == HC_ACTION as i32 {
        let kb = unsafe { &*(l_param.0 as *const KBDLLHOOKSTRUCT) };

        let vk = kb.vkCode;
        let msg = w_param.0 as u32;

        let is_keydown = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
        let is_keyup = msg == WM_KEYUP || msg == WM_SYSKEYUP;

        let is_bracket = vk == VK_OEM_4 || vk == VK_OEM_6;

        let shift_down = unsafe { GetAsyncKeyState(VK_SHIFT) < 0 };
        let other_modifiers_down =
            unsafe { GetAsyncKeyState(VK_CONTROL) < 0 || GetAsyncKeyState(VK_MENU) < 0 };

        if (is_keydown || is_keyup)
            && is_bracket
            && shift_down
            && !other_modifiers_down
            && kb.flags.0 & LLKHF_INJECTED == 0
            && let Some(hwnd) = unsafe { is_chinese_input_for_foreground() }
        {
            if is_keydown {
                let ch = if vk == VK_OEM_4 { '「' } else { '」' };
                unsafe {
                    let mut class_name = [0u16; 256];
                    let len = GetClassNameW(hwnd, &mut class_name);
                    let class_str = String::from_utf16_lossy(&class_name[..len as usize]);
                    let use_post = class_str.starts_with("Qt");
                    if use_post {
                        let _ = PostMessageW(Some(hwnd), WM_CHAR, WPARAM(ch as usize), LPARAM(1));
                    } else {
                        send_unicode_char(ch);
                    }
                }
            }
            return LRESULT(1);
        }
    }

    unsafe { CallNextHookEx(None, n_code, w_param, l_param) }
}

unsafe fn is_chinese_input_for_foreground() -> Option<HWND> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        return None;
    }

    let mut _pid = 0u32;
    let tid = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut _pid)) };

    let hkl: HKL = unsafe { GetKeyboardLayout(tid) };
    let lang_id = hkl.0 as u32 & 0xFFFF;
    let primary_lang = lang_id & 0x3FF;
    if primary_lang != LANG_CHINESE {
        return None;
    }

    let ime_hwnd = unsafe { ImmGetDefaultIMEWnd(hwnd) };
    if ime_hwnd.0.is_null() {
        return None;
    }

    const WM_IME_CONTROL: u32 = 0x0283;
    const IMC_GETCONVERSIONMODE: usize = 0x0001;

    let mode = unsafe {
        SendMessageW(
            ime_hwnd,
            WM_IME_CONTROL,
            Some(WPARAM(IMC_GETCONVERSIONMODE)),
            Some(LPARAM(0)),
        )
    }
    .0 as u32;

    if mode & IME_CMODE_NATIVE.0 != 0 {
        Some(hwnd)
    } else {
        None
    }
}

unsafe fn send_unicode_char(ch: char) {
    let mut inputs = [INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0),
                wScan: ch as u16,
                dwFlags: KEYBD_EVENT_FLAGS(KEYEVENTF_UNICODE.0),
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }];

    let _ = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };

    inputs[0].Anonymous.ki.dwFlags = KEYBD_EVENT_FLAGS(KEYEVENTF_UNICODE.0 | KEYEVENTF_KEYUP.0);
    let _ = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
}

use std::mem::size_of;

use windows::Win32::Foundation::{WAIT_EVENT, WAIT_FAILED, WAIT_OBJECT_0};
use windows::Win32::System::Threading::INFINITE;
use windows::Win32::UI::WindowsAndMessaging::QS_ALLINPUT;
use windows::Win32::{
    Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM},
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
            GetWindowThreadProcessId, HC_ACTION, KBDLLHOOKSTRUCT, MSG, MsgWaitForMultipleObjects,
            PM_REMOVE, PeekMessageW, PostMessageW, SendMessageW, TranslateMessage, WM_CHAR,
        },
    },
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
const LANG_CHINESE: u32 = 0x04;

pub unsafe extern "system" fn low_level_keyboard_proc(
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

pub fn run_message_loop(stop_event: HANDLE) {
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

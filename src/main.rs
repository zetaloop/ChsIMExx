#![windows_subsystem = "windows"]

use std::mem::size_of;

use windows::Win32::{
    Foundation::{LPARAM, LRESULT, WPARAM},
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
            CallNextHookEx, DispatchMessageW, GetForegroundWindow, GetMessageW,
            GetWindowThreadProcessId, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, MSG, SendMessageW,
            SetWindowsHookExW, TranslateMessage, WH_KEYBOARD_LL,
        },
    },
};

const WM_KEYDOWN: u32 = 0x0100;
const WM_KEYUP: u32 = 0x0101;
const WM_SYSKEYDOWN: u32 = 0x0104;
const WM_SYSKEYUP: u32 = 0x0105;

const VK_SHIFT: i32 = 0x10;
const VK_OEM_4: u32 = 0xDB; // [
const VK_OEM_6: u32 = 0xDD; // ]

const LLKHF_INJECTED: u32 = 0x10;
const LANG_CHINESE: u32 = 0x04; // 低 10 位是主语言 ID，0x04 代表中文

static mut HOOK_HANDLE: Option<HHOOK> = None;

fn main() {
    unsafe {
        // 安装低层键盘钩子
        HOOK_HANDLE = Some(
            SetWindowsHookExW(WH_KEYBOARD_LL, Some(low_level_keyboard_proc), None, 0)
                .expect("SetWindowsHookExW failed"),
        );

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).into() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe extern "system" fn low_level_keyboard_proc(
    n_code: i32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    if n_code == HC_ACTION as i32 {
        // l_param 指向 KBDLLHOOKSTRUCT
        let kb = unsafe { &*(l_param.0 as *const KBDLLHOOKSTRUCT) };

        let vk = kb.vkCode;
        let msg = w_param.0 as u32;

        let is_keydown = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
        let is_keyup = msg == WM_KEYUP || msg == WM_SYSKEYUP;

        let is_bracket = vk == VK_OEM_4 || vk == VK_OEM_6;

        // 只关心 Shift+[ 和 Shift+]
        let shift_down = unsafe { GetAsyncKeyState(VK_SHIFT) < 0 };

        if (is_keydown || is_keyup)
            && is_bracket
            && shift_down
            && kb.flags.0 & LLKHF_INJECTED == 0
            && unsafe { is_chinese_input_for_foreground() }
        {
            // 在中文输入法下拦截原按键
            if is_keydown {
                let ch = if vk == VK_OEM_4 { '「' } else { '」' };
                unsafe {
                    send_unicode_char(ch);
                }
            }
            return LRESULT(1); // 非零表示吃掉消息
        }
    }

    unsafe { CallNextHookEx(HOOK_HANDLE, n_code, w_param, l_param) }
}

// 只在：当前键盘布局是中文，并且 IME 处于“中文模式”时返回 true
unsafe fn is_chinese_input_for_foreground() -> bool {
    // 先锁定前台窗口
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        return false;
    }

    // 通过线程拿到 HKL，只在主语言是中文时继续往下
    let mut _pid = 0u32;
    let tid = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut _pid)) };

    let hkl: HKL = unsafe { GetKeyboardLayout(tid) };
    let lang_id = hkl.0 as u32 & 0xFFFF;
    let primary_lang = lang_id & 0x3FF;
    if primary_lang != LANG_CHINESE {
        // ENG、美式键盘等直接视为“非中文模式”
        return false;
    }

    // 拿到这个窗口对应的 IME 窗口句柄
    let ime_hwnd = unsafe { ImmGetDefaultIMEWnd(hwnd) };
    if ime_hwnd.0.is_null() {
        // 理论上不太会发生，如果拿不到就保守地当作“非中文模式”
        return false;
    }

    // 通过 WM_IME_CONTROL 询问当前转换模式
    const WM_IME_CONTROL: u32 = 0x0283;
    const IMC_GETCONVERSIONMODE: usize = 0x0001; // wParam 值

    let mode = unsafe {
        SendMessageW(
            ime_hwnd,
            WM_IME_CONTROL,
            Some(WPARAM(IMC_GETCONVERSIONMODE)),
            Some(LPARAM(0)),
        )
    }
    .0 as u32;

    // 只有在“本地语言模式”（微软拼音的中文模式）下，这一位才为 1
    mode & IME_CMODE_NATIVE.0 != 0
}

// 通过 SendInput 发送一个 Unicode 字符
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

    // keydown
    let _ = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };

    // keyup
    inputs[0].Anonymous.ki.dwFlags = KEYBD_EVENT_FLAGS(KEYEVENTF_UNICODE.0 | KEYEVENTF_KEYUP.0);
    let _ = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
}

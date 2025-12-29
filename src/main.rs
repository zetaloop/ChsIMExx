#![windows_subsystem = "windows"]

mod console;
mod hook;
mod instance;
mod notify;

use std::{env, process};

use windows::Win32::UI::WindowsAndMessaging::{
    SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL,
};

use console::{ConsoleSession, console_prefix, log_error, log_to_console};
use hook::{low_level_keyboard_proc, run_message_loop};
use instance::{InstanceGuard, InstanceState, signal_shutdown_request};
use notify::notify;

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
        console.println(&format!("\r\n{} {message}", console_prefix()));
        Ok(())
    } else {
        Err(1)
    }
}

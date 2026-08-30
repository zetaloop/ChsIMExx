#![windows_subsystem = "windows"]

mod console;
mod foreground;
mod hook;
mod instance;
mod notify;
mod startup;

use std::{env, process};

use windows::Win32::UI::{
    Accessibility::UnhookWinEvent,
    WindowsAndMessaging::{SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL},
};

use console::{ConsoleSession, console_prefix, is_elevated, log_error, log_to_console};
use foreground::allow_foreground_activation;
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
        Command::Install => run_install(),
        Command::Uninstall => run_uninstall(),
        Command::Version => run_version(),
    }
}

enum Command {
    Run,
    Stop,
    Install,
    Uninstall,
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
                "install" | "--install" => Some(Command::Install),
                "uninstall" | "--uninstall" => Some(Command::Uninstall),
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
    if !is_elevated()
        && startup::start().map_err(|msg| {
            log_error(&msg);
            1
        })?
    {
        return Ok(());
    }

    let mut guard = InstanceGuard::new().map_err(|err| {
        log_error(&format!("创建同步对象失败: {err:?}"));
        1
    })?;

    let state = guard.acquire().map_err(|msg| {
        log_error(&msg);
        1
    })?;

    let foreground_hook = allow_foreground_activation().map_err(|msg| {
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
        let _ = UnhookWinEvent(foreground_hook);
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

fn run_install() -> Result<(), i32> {
    if !is_elevated() {
        return run_elevated("install");
    }

    startup::install().map_err(|msg| {
        log_error(&msg);
        1
    })?;

    const MESSAGE: &str = "已安装";
    notify(MESSAGE);
    log_to_console(MESSAGE);
    Ok(())
}

fn run_uninstall() -> Result<(), i32> {
    if !is_elevated() {
        return run_elevated("uninstall");
    }

    signal_shutdown_request().map_err(|msg| {
        log_error(&msg);
        1
    })?;
    startup::uninstall().map_err(|msg| {
        log_error(&msg);
        1
    })?;

    const MESSAGE: &str = "已卸载";
    notify(MESSAGE);
    log_to_console(MESSAGE);
    Ok(())
}

fn run_elevated(command: &str) -> Result<(), i32> {
    match startup::elevate(command) {
        Ok(0) => Ok(()),
        Ok(code) => {
            log_error(&format!("管理员操作失败，退出代码: {code}"));
            Err(code)
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

use std::{env, ffi::OsStr, mem::size_of, os::windows::ffi::OsStrExt, path::Path};

use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_FILE_NOT_FOUND, WAIT_FAILED},
        System::{
            Com::{
                CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
                CoUninitialize,
            },
            TaskScheduler::{
                IRegisteredTask, ITaskFolder, ITaskService, TASK_CREATE_OR_UPDATE,
                TASK_LOGON_INTERACTIVE_TOKEN, TaskScheduler,
            },
            Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject},
            Variant::VARIANT,
        },
        UI::{
            Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW},
            WindowsAndMessaging::SW_SHOWNORMAL,
        },
    },
    core::{BSTR, HRESULT, PCWSTR},
};

const TASK_NAME: &str = "ChsIMExx";

struct ComSession;

impl ComSession {
    fn new() -> windows::core::Result<Self> {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
        Ok(Self)
    }
}

impl Drop for ComSession {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

struct Scheduler {
    root: ITaskFolder,
    user: String,
    _com: ComSession,
}

impl Scheduler {
    fn connect() -> windows::core::Result<Self> {
        let com = ComSession::new()?;
        let service: ITaskService =
            unsafe { CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER)? };
        let empty = VARIANT::default();
        unsafe { service.Connect(&empty, &empty, &empty, &empty)? };

        let username = unsafe { service.ConnectedUser()?.to_string() };
        let domain = unsafe { service.ConnectedDomain()?.to_string() };
        let user = if domain.is_empty() {
            username
        } else {
            format!("{domain}\\{username}")
        };
        let root = unsafe { service.GetFolder(&BSTR::from("\\"))? };

        Ok(Self {
            root,
            user,
            _com: com,
        })
    }

    fn task(&self) -> windows::core::Result<Option<IRegisteredTask>> {
        match unsafe { self.root.GetTask(&BSTR::from(TASK_NAME)) } {
            Ok(task) => Ok(Some(task)),
            Err(err) if err.code() == HRESULT::from_win32(ERROR_FILE_NOT_FOUND.0) => Ok(None),
            Err(err) => Err(err),
        }
    }
}

pub fn start() -> Result<bool, String> {
    let scheduler = Scheduler::connect().map_err(|err| format!("连接任务计划程序失败: {err:?}"))?;
    let Some(task) = scheduler
        .task()
        .map_err(|err| format!("读取开机自启任务失败: {err:?}"))?
    else {
        return Ok(false);
    };

    unsafe { task.Run(&VARIANT::default()) }
        .map_err(|err| format!("启动开机自启任务失败: {err:?}"))?;
    Ok(true)
}

pub fn install() -> Result<(), String> {
    let executable = env::current_exe().map_err(|err| format!("获取程序路径失败: {err}"))?;
    let scheduler = Scheduler::connect().map_err(|err| format!("连接任务计划程序失败: {err:?}"))?;
    let xml = task_xml(&scheduler.user, &executable)?;
    let empty = VARIANT::default();
    let user = VARIANT::from(BSTR::from(scheduler.user.as_str()));
    let task = unsafe {
        scheduler.root.RegisterTask(
            &BSTR::from(TASK_NAME),
            &BSTR::from(xml),
            TASK_CREATE_OR_UPDATE.0,
            &user,
            &empty,
            TASK_LOGON_INTERACTIVE_TOKEN,
            &empty,
        )
    }
    .map_err(|err| format!("注册开机自启任务失败: {err:?}"))?;

    unsafe { task.Run(&empty) }.map_err(|err| format!("启动开机自启任务失败: {err:?}"))?;
    Ok(())
}

pub fn uninstall() -> Result<(), String> {
    let scheduler = Scheduler::connect().map_err(|err| format!("连接任务计划程序失败: {err:?}"))?;
    if scheduler
        .task()
        .map_err(|err| format!("读取开机自启任务失败: {err:?}"))?
        .is_some()
    {
        unsafe { scheduler.root.DeleteTask(&BSTR::from(TASK_NAME), 0) }
            .map_err(|err| format!("删除开机自启任务失败: {err:?}"))?;
    }
    Ok(())
}

pub fn elevate(command: &str) -> Result<i32, String> {
    let executable = env::current_exe().map_err(|err| format!("获取程序路径失败: {err}"))?;
    let executable = wide(executable.as_os_str());
    let command = wide(OsStr::new(command));
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: windows::core::w!("runas"),
        lpFile: PCWSTR(executable.as_ptr()),
        lpParameters: PCWSTR(command.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };

    unsafe { ShellExecuteExW(&mut info) }.map_err(|err| format!("请求管理员权限失败: {err:?}"))?;
    if info.hProcess.is_invalid() {
        return Err("管理员进程句柄无效".to_string());
    }

    let result = unsafe {
        if WaitForSingleObject(info.hProcess, INFINITE) == WAIT_FAILED {
            Err(format!(
                "等待管理员操作失败: {:?}",
                windows::core::Error::from_thread()
            ))
        } else {
            let mut exit_code = 0;
            GetExitCodeProcess(info.hProcess, &mut exit_code)
                .map(|()| exit_code as i32)
                .map_err(|err| format!("读取管理员操作结果失败: {err:?}"))
        }
    };
    let _ = unsafe { CloseHandle(info.hProcess) };
    result
}

fn task_xml(user: &str, executable: &Path) -> Result<String, String> {
    let executable = executable
        .to_str()
        .ok_or_else(|| format!("程序路径不是有效的 Unicode: {}", executable.display()))?;
    let user = escape_xml(user);
    let executable = escape_xml(executable);

    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.4" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Triggers>
    <LogonTrigger>
      <UserId>{user}</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>Parallel</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{executable}</Command>
    </Exec>
  </Actions>
</Task>"#
    ))
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

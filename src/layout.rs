use std::{
    cell::Cell,
    mem::{offset_of, size_of},
    slice,
    thread::{self, JoinHandle},
};

use windows::{
    Win32::{
        Foundation::{ERROR_ALREADY_EXISTS, LPARAM, WPARAM},
        System::{Diagnostics::Etw::*, Threading::INFINITE},
        UI::{
            Input::KeyboardAndMouse::{
                GetKeyboardLayout, GetKeyboardLayoutList, HKL, UnloadKeyboardLayout,
            },
            WindowsAndMessaging::{
                GetForegroundWindow, GetWindowThreadProcessId, PostMessageW,
                WM_INPUTLANGCHANGEREQUEST,
            },
        },
    },
    core::{Error, GUID, PCWSTR, PWSTR, Result, w},
};

const INPUT_CORE: GUID = GUID::from_u128(0x5fb75eac_9f0b_550c_339f_fc21fde966cd);
const SESSION: PCWSTR = w!("ChsIMExx-InputCore");
const US: usize = 0x04090409;

#[repr(C)]
struct Properties {
    trace: EVENT_TRACE_PROPERTIES,
    name: [u16; 1024],
}

impl Properties {
    fn new() -> Self {
        Self {
            trace: EVENT_TRACE_PROPERTIES {
                Wnode: WNODE_HEADER {
                    BufferSize: size_of::<Self>() as u32,
                    Flags: WNODE_FLAG_TRACED_GUID,
                    ..Default::default()
                },
                LogFileMode: EVENT_TRACE_REAL_TIME_MODE,
                LoggerNameOffset: offset_of!(Self, name) as u32,
                ..Default::default()
            },
            name: [0; 1024],
        }
    }
}

pub struct Monitor {
    session: CONTROLTRACE_HANDLE,
    reader: Option<PROCESSTRACE_HANDLE>,
    worker: Option<JoinHandle<()>>,
}

impl Monitor {
    pub fn start() -> Result<Self> {
        unsafe {
            let mut properties = Properties::new();
            let mut session = CONTROLTRACE_HANDLE::default();
            let status = StartTraceW(&mut session, SESSION, &mut properties.trace);
            if status == ERROR_ALREADY_EXISTS {
                // The instance mutex is held; this session outlived its owner.
                ControlTraceW(
                    CONTROLTRACE_HANDLE::default(),
                    SESSION,
                    &mut properties.trace,
                    EVENT_TRACE_CONTROL_STOP,
                )
                .ok()?;
                properties = Properties::new();
                StartTraceW(&mut session, SESSION, &mut properties.trace).ok()?;
            } else {
                status.ok()?;
            }
            let mut monitor = Self {
                session,
                reader: None,
                worker: None,
            };
            let mut log = EVENT_TRACE_LOGFILEW {
                LoggerName: PWSTR(SESSION.0.cast_mut()),
                Anonymous1: EVENT_TRACE_LOGFILEW_0 {
                    ProcessTraceMode: PROCESS_TRACE_MODE_REAL_TIME
                        | PROCESS_TRACE_MODE_EVENT_RECORD,
                },
                Anonymous2: EVENT_TRACE_LOGFILEW_1 {
                    EventRecordCallback: Some(on_event),
                },
                ..Default::default()
            };
            let reader = OpenTraceW(&mut log);
            if reader.Value == u64::MAX {
                return Err(Error::from_thread());
            }
            monitor.reader = Some(reader);
            EnableTraceEx2(
                session,
                &INPUT_CORE,
                EVENT_CONTROL_CODE_ENABLE_PROVIDER.0,
                4,
                0x4000,
                0,
                INFINITE,
                None,
            )
            .ok()?;
            monitor.worker = Some(
                thread::Builder::new()
                    .spawn(move || {
                        inspect_layouts(None);
                        let result = ProcessTrace(&[reader], None, None);
                        if let Err(err) = result.ok() {
                            crate::console::log_error(&format!("读取键盘布局事件失败: {err}"));
                        }
                    })
                    .map_err(|err| {
                        Error::new(windows::Win32::Foundation::E_FAIL, err.to_string())
                    })?,
            );
            Ok(monitor)
        }
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        unsafe {
            if let Some(reader) = self.reader.take() {
                let _ = CloseTrace(reader);
            }
            let _ = ControlTraceW(
                self.session,
                None,
                &mut Properties::new().trace,
                EVENT_TRACE_CONTROL_STOP,
            );
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

unsafe extern "system" fn on_event(record: *mut EVENT_RECORD) {
    let record = unsafe { &*record };
    if record.EventHeader.ProviderId != INPUT_CORE || record.ExtendedDataCount == 0 {
        return;
    }
    let extensions =
        unsafe { slice::from_raw_parts(record.ExtendedData, record.ExtendedDataCount.into()) };
    for ext in extensions {
        if u32::from(ext.ExtType) != EVENT_HEADER_EXT_TYPE_EVENT_SCHEMA_TL || ext.DataSize == 0 {
            continue;
        }
        let schema =
            unsafe { slice::from_raw_parts(ext.DataPtr as *const u8, ext.DataSize.into()) };
        if let Some(change) = layout_event(schema) {
            let activation =
                (change == Change::Activate && record.UserDataLength >= 4).then(|| {
                    // InputCore's activation payload starts with the new 32-bit HKL.
                    let hkl = unsafe { record.UserData.cast::<u32>().read_unaligned() };
                    (record.EventHeader.ThreadId, hkl)
                });
            inspect_layouts(activation);
        }
        break;
    }
}

#[derive(PartialEq)]
enum Change {
    Load,
    Activate,
}

fn layout_event(schema: &[u8]) -> Option<Change> {
    // TraceLogging metadata: u16 size, variable-length tags, NUL-terminated UTF-8 name.
    let tags = schema.get(2..)?;
    let end = tags.iter().position(|b| b & 0x80 == 0)?;
    let name = &tags[end + 1..];
    match name.split(|&b| b == 0).next()? {
        b"Keyboard::LoadKeyboardLayout" => Some(Change::Load),
        b"Keyboard::ActivateKeyboardLayout" => Some(Change::Activate),
        _ => None,
    }
}

fn inspect_layouts(activation: Option<(u32, u32)>) {
    // Startup and all event callbacks run on the ProcessTrace thread.
    thread_local! { static REQUESTED: Cell<Option<u32>> = const { Cell::new(None) }; }
    if let Some((tid, hkl)) = activation
        && hkl as usize != US
        && REQUESTED.get() == Some(tid)
    {
        REQUESTED.set(None);
    }
    if let Err(err) = REQUESTED.with(reconcile) {
        crate::console::log_to_console(&format!("调整键盘布局失败: {err}"));
    }
}

fn reconcile(requested: &Cell<Option<u32>>) -> Result<()> {
    unsafe {
        let count = GetKeyboardLayoutList(None);
        if count == 0 {
            return Err(Error::from_thread());
        }
        let mut loaded = vec![HKL::default(); count as usize];
        let copied = GetKeyboardLayoutList(Some(&mut loaded));
        if copied == 0 {
            return Err(Error::from_thread());
        }
        loaded.truncate(copied as usize);
        let hwnd = GetForegroundWindow();
        let tid = GetWindowThreadProcessId(hwnd, None);
        let active = if tid != 0 {
            GetKeyboardLayout(tid)
        } else {
            HKL::default()
        };
        if let Some(id) = requested.get()
            && GetKeyboardLayout(id).0 as usize != US
        {
            requested.set(None);
        }
        let pending = requested
            .get()
            .is_some_and(|id| id == tid || active.0 as usize != US);
        if pending {
            return Ok(());
        }
        let Some(chinese) = loaded.iter().find(|hkl| hkl.0 as usize & 0x3ff == 4) else {
            return Ok(());
        };
        if active.0 as usize == US {
            PostMessageW(
                Some(hwnd),
                WM_INPUTLANGCHANGEREQUEST,
                WPARAM(0),
                LPARAM(chinese.0 as isize),
            )?;
            requested.set(Some(tid));
        } else if let Some(&us) = loaded.iter().find(|hkl| hkl.0 as usize == US) {
            UnloadKeyboardLayout(us)?;
        }
        Ok(())
    }
}

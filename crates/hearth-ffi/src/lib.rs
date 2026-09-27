//! A C ABI over [`hearth_core::Core`] for the native macOS app. Everything
//! crosses as JSON: commands take the same names and camelCase arguments as
//! the Tauri frontend's `invoke`, and events arrive as the same arrays the
//! Svelte app receives. See `include/hearth.h`.

use hearth_core::Core;
use serde_json::Value;
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::Path;
use std::sync::Arc;

pub type EventCallback = extern "C" fn(ctx: *mut c_void, json: *const c_char);
pub type ReplyCallback = extern "C" fn(ctx: *mut c_void, ok: bool, json: *const c_char);

pub struct Handle {
    core: Arc<Core>,
}

/// The caller's context pointer. The caller promises it may be used from any
/// thread, which is what the header documents.
#[derive(Clone, Copy)]
struct Ctx(*mut c_void);
unsafe impl Send for Ctx {}
unsafe impl Sync for Ctx {}

fn cstring(s: String) -> CString {
    CString::new(s).unwrap_or_else(|e| {
        let mut bytes = e.into_vec();
        bytes.retain(|&b| b != 0);
        CString::new(bytes).unwrap_or_default()
    })
}

unsafe fn str_arg<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        return None;
    }
    CStr::from_ptr(p).to_str().ok()
}

/// Open the core. On failure returns null and, if `error` is non-null, stores
/// a message there that must be released with `hearth_string_free`.
///
/// # Safety
/// `config_dir` and `data_dir` must be valid NUL-terminated UTF-8 strings.
/// `on_event` is called from background threads with `ctx`.
#[no_mangle]
pub unsafe extern "C" fn hearth_open(
    config_dir: *const c_char,
    data_dir: *const c_char,
    on_event: EventCallback,
    ctx: *mut c_void,
    error: *mut *mut c_char,
) -> *mut Handle {
    logger::init();
    let fail = |msg: String| {
        if !error.is_null() {
            *error = cstring(msg).into_raw();
        }
        std::ptr::null_mut()
    };
    let (Some(config_dir), Some(data_dir)) = (str_arg(config_dir), str_arg(data_dir)) else {
        return fail("invalid directory".into());
    };
    let ctx = Ctx(ctx);
    let sink = move |events: &[hearth_core::irc::events::Event]| match serde_json::to_string(events) {
        Ok(json) => {
            let ctx = ctx;
            on_event(ctx.0, cstring(json).as_ptr());
        }
        Err(e) => log::error!("could not encode events: {e}"),
    };
    match Core::open(Path::new(config_dir), Path::new(data_dir), sink) {
        Ok(core) => Box::into_raw(Box::new(Handle { core: Arc::new(core) })),
        Err(e) => fail(format!("{e:#}")),
    }
}

/// Run a command asynchronously. `reply` is called exactly once, from a
/// background thread, with `ok` and a JSON result (or error message string).
///
/// # Safety
/// `handle` must come from `hearth_open` and not yet be closed. `command` and
/// `args_json` must be valid NUL-terminated strings (`args_json` may be null).
#[no_mangle]
pub unsafe extern "C" fn hearth_call(
    handle: *const Handle,
    command: *const c_char,
    args_json: *const c_char,
    reply: ReplyCallback,
    ctx: *mut c_void,
) {
    let ctx = Ctx(ctx);
    let respond = move |result: Result<Value, String>| {
        let ctx = ctx;
        let (ok, text) = match result {
            Ok(v) => (true, v.to_string()),
            Err(e) => (false, Value::String(e).to_string()),
        };
        reply(ctx.0, ok, cstring(text).as_ptr());
    };
    let Some(handle) = handle.as_ref() else {
        return respond(Err("hearth is not open".into()));
    };
    let Some(command) = str_arg(command).map(str::to_owned) else {
        return respond(Err("invalid command".into()));
    };
    let args = match str_arg(args_json).filter(|s| !s.is_empty()) {
        None => Value::Object(Default::default()),
        Some(s) => match serde_json::from_str(s) {
            Ok(v) => v,
            Err(e) => return respond(Err(format!("invalid arguments: {e}"))),
        },
    };
    let core = handle.core.clone();
    hearth_core::runtime::spawn(async move { respond(core.call(&command, args).await) });
}

/// Whether closing the window should keep Hearth running in the menu bar.
///
/// # Safety
/// `handle` must come from `hearth_open` and not yet be closed.
#[no_mangle]
pub unsafe extern "C" fn hearth_close_to_tray(handle: *const Handle) -> bool {
    handle.as_ref().is_some_and(|h| h.core.close_to_tray())
}

/// Quit every connection, flush the log and free the handle.
///
/// # Safety
/// `handle` must come from `hearth_open`; it is invalid afterwards.
#[no_mangle]
pub unsafe extern "C" fn hearth_close(handle: *mut Handle) {
    if handle.is_null() {
        return;
    }
    let handle = Box::from_raw(handle);
    handle.core.shutdown();
}

/// # Safety
/// `s` must be a string returned by this library, or null.
#[no_mangle]
pub unsafe extern "C" fn hearth_string_free(s: *mut c_char) {
    if !s.is_null() {
        drop(CString::from_raw(s));
    }
}

mod logger {
    //! `log` output to stderr, which lands in Console.app for a GUI process.

    struct Stderr;

    impl log::Log for Stderr {
        fn enabled(&self, m: &log::Metadata) -> bool {
            m.level() <= log::max_level()
        }

        fn log(&self, r: &log::Record) {
            if self.enabled(r.metadata()) {
                eprintln!("[hearth {} {}] {}", r.level(), r.target(), r.args());
            }
        }

        fn flush(&self) {}
    }

    pub fn init() {
        if log::set_logger(&Stderr).is_ok() {
            log::set_max_level(if cfg!(debug_assertions) { log::LevelFilter::Info } else { log::LevelFilter::Warn });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    extern "C" fn no_events(_: *mut c_void, _: *const c_char) {}

    extern "C" fn on_reply(ctx: *mut c_void, ok: bool, json: *const c_char) {
        let tx = unsafe { &*(ctx as *const mpsc::Sender<(bool, String)>) };
        let text = unsafe { CStr::from_ptr(json) }.to_string_lossy().into_owned();
        tx.send((ok, text)).unwrap();
    }

    #[test]
    fn calls_round_trip_through_the_c_abi() {
        let dir = std::env::temp_dir().join(format!("hearth-ffi-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = CString::new(dir.to_str().unwrap()).unwrap();
        let mut error = std::ptr::null_mut();
        let h = unsafe { hearth_open(path.as_ptr(), path.as_ptr(), no_events, std::ptr::null_mut(), &mut error) };
        assert!(!h.is_null());

        let (tx, rx) = mpsc::channel::<(bool, String)>();
        let ctx = &tx as *const _ as *mut c_void;
        let call = |cmd: &str, args: &str| {
            let (cmd, args) = (CString::new(cmd).unwrap(), CString::new(args).unwrap());
            unsafe { hearth_call(h, cmd.as_ptr(), args.as_ptr(), on_reply, ctx) };
            rx.recv_timeout(Duration::from_secs(5)).unwrap()
        };
        let (ok, cfg) = call("get_config", "");
        assert!(ok);
        assert!(cfg.contains("\"servers\":[]"));
        assert_eq!(call("connect", r#"{"server":"x"}"#), (false, "\"unknown server\"".into()));
        assert!(!call("send", "{not json").0);

        unsafe { hearth_close(h) };
        let _ = std::fs::remove_dir_all(dir);
    }
}

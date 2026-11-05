//! libsecret, loaded at run time. Bun stores secrets by calling the same
//! functions with the same schema, which is what makes the items shared; going
//! through the `keyring` crate would give them other attributes.
//!
//! The struct layouts follow libsecret's public headers.

use std::ffi::{c_char, c_int, c_uint, c_void, CStr, CString};
use std::ptr;
use std::sync::OnceLock;

#[repr(C)]
struct Attribute {
    name: *const c_char,
    kind: c_int,
}

#[repr(C)]
struct Schema {
    name: *const c_char,
    flags: c_int,
    attributes: [Attribute; 32],
    reserved: c_int,
    reserved_ptrs: [*mut c_void; 7],
}

// Only ever points at string literals.
unsafe impl Sync for Schema {}

#[repr(C)]
struct GError {
    domain: c_uint,
    code: c_int,
    message: *mut c_char,
}

const UNUSED: Attribute = Attribute { name: ptr::null(), kind: 0 };
const STRING: c_int = 0;

static SCHEMA: Schema = Schema {
    name: c"com.oven-sh.bun.Secret".as_ptr(),
    flags: 0,
    attributes: {
        let mut a = [UNUSED; 32];
        a[0] = Attribute { name: c"service".as_ptr(), kind: STRING };
        a[1] = Attribute { name: c"account".as_ptr(), kind: STRING };
        a
    },
    reserved: 0,
    reserved_ptrs: [ptr::null_mut(); 7],
};

type Store = unsafe extern "C" fn(*const Schema, *const c_char, *const c_char, *const c_char, *mut c_void, *mut *mut GError, ...) -> c_int;
type Lookup = unsafe extern "C" fn(*const Schema, *mut c_void, *mut *mut GError, ...) -> *mut c_char;
type Clear = unsafe extern "C" fn(*const Schema, *mut c_void, *mut *mut GError, ...) -> c_int;

struct Lib {
    store: Store,
    lookup: Lookup,
    clear: Clear,
    free_password: unsafe extern "C" fn(*mut c_char),
    free_error: unsafe extern "C" fn(*mut GError),
}

fn open(names: &[&CStr]) -> *mut c_void {
    for name in names {
        // SAFETY: a NUL-terminated name; the handle is deliberately never closed.
        let handle = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_LAZY | libc::RTLD_GLOBAL) };
        if !handle.is_null() {
            return handle;
        }
    }
    ptr::null_mut()
}

fn symbol(handle: *mut c_void, name: &CStr) -> Option<*mut c_void> {
    // SAFETY: `handle` came from dlopen and `name` is NUL-terminated.
    let sym = unsafe { libc::dlsym(handle, name.as_ptr()) };
    (!sym.is_null()).then_some(sym)
}

fn load() -> Option<Lib> {
    let glib = open(&[c"libglib-2.0.so.0", c"libglib-2.0.so"]);
    if glib.is_null() {
        return None;
    }
    let secret = open(&[c"libsecret-1.so.0"]);
    if secret.is_null() {
        return None;
    }
    // SAFETY: each symbol is cast to the signature libsecret and GLib document for it.
    unsafe {
        Some(Lib {
            store: std::mem::transmute::<*mut c_void, Store>(symbol(secret, c"secret_password_store_sync")?),
            lookup: std::mem::transmute::<*mut c_void, Lookup>(symbol(secret, c"secret_password_lookup_sync")?),
            clear: std::mem::transmute::<*mut c_void, Clear>(symbol(secret, c"secret_password_clear_sync")?),
            free_password: std::mem::transmute::<*mut c_void, unsafe extern "C" fn(*mut c_char)>(symbol(secret, c"secret_password_free")?),
            free_error: std::mem::transmute::<*mut c_void, unsafe extern "C" fn(*mut GError)>(symbol(glib, c"g_error_free")?),
        })
    }
}

fn lib() -> Result<&'static Lib, String> {
    static LIB: OnceLock<Option<Lib>> = OnceLock::new();
    LIB.get_or_init(load).as_ref().ok_or_else(|| "libsecret not available".to_string())
}

fn cstring(s: &str) -> Result<CString, String> {
    CString::new(s).map_err(|_| "secrets cannot contain NUL bytes".to_string())
}

/// Takes ownership of a GError, returning its message.
fn take_error(lib: &Lib, err: *mut GError) -> Option<String> {
    if err.is_null() {
        return None;
    }
    // SAFETY: a GError libsecret filled in; freed exactly once.
    unsafe {
        let msg = if (*err).message.is_null() { "libsecret error".to_string() } else { CStr::from_ptr((*err).message).to_string_lossy().into_owned() };
        (lib.free_error)(err);
        Some(msg)
    }
}

pub fn store(service: &str, name: &str, value: &str) -> Result<(), String> {
    let lib = lib()?;
    let (service, account, secret) = (cstring(service)?, cstring(name)?, cstring(value)?);
    let label = cstring(&format!("{}/{}", service.to_string_lossy(), account.to_string_lossy()))?;
    let mut err: *mut GError = ptr::null_mut();
    // SAFETY: the attribute list is name/value pairs ended by NULL, as the variadic call requires.
    let ok = unsafe {
        (lib.store)(
            &SCHEMA,
            ptr::null(),
            label.as_ptr(),
            secret.as_ptr(),
            ptr::null_mut(),
            &mut err,
            c"service".as_ptr(),
            service.as_ptr(),
            c"account".as_ptr(),
            account.as_ptr(),
            ptr::null::<c_char>(),
        )
    };
    match take_error(lib, err) {
        Some(msg) => Err(msg),
        None if ok == 0 => Err("failed to store the secret".into()),
        None => Ok(()),
    }
}

pub fn lookup(service: &str, name: &str) -> Result<Option<String>, String> {
    let lib = lib()?;
    let (service, account) = (cstring(service)?, cstring(name)?);
    let mut err: *mut GError = ptr::null_mut();
    // SAFETY: as in `store`.
    let found = unsafe {
        (lib.lookup)(&SCHEMA, ptr::null_mut(), &mut err, c"service".as_ptr(), service.as_ptr(), c"account".as_ptr(), account.as_ptr(), ptr::null::<c_char>())
    };
    if let Some(msg) = take_error(lib, err) {
        if !found.is_null() {
            // SAFETY: returned by libsecret, freed once.
            unsafe { (lib.free_password)(found) };
        }
        return Err(msg);
    }
    if found.is_null() {
        return Ok(None);
    }
    // SAFETY: a NUL-terminated string owned by libsecret until freed here.
    let value = unsafe { CStr::from_ptr(found).to_string_lossy().into_owned() };
    unsafe { (lib.free_password)(found) };
    Ok(Some(value))
}

/// Removes the item; one that is not there is not an error.
pub fn clear(service: &str, name: &str) -> Result<(), String> {
    let lib = lib()?;
    let (service, account) = (cstring(service)?, cstring(name)?);
    let mut err: *mut GError = ptr::null_mut();
    // SAFETY: as in `store`.
    unsafe {
        (lib.clear)(&SCHEMA, ptr::null_mut(), &mut err, c"service".as_ptr(), service.as_ptr(), c"account".as_ptr(), account.as_ptr(), ptr::null::<c_char>());
    }
    match take_error(lib, err) {
        Some(msg) => Err(msg),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_schema_matches_buns() {
        // SAFETY: the statics point at literals.
        unsafe {
            assert_eq!(CStr::from_ptr(SCHEMA.name).to_str().unwrap(), "com.oven-sh.bun.Secret");
            assert_eq!(CStr::from_ptr(SCHEMA.attributes[0].name).to_str().unwrap(), "service");
            assert_eq!(CStr::from_ptr(SCHEMA.attributes[1].name).to_str().unwrap(), "account");
        }
        assert!(SCHEMA.attributes[2].name.is_null());
        assert_eq!(SCHEMA.flags, 0);
    }

    #[test]
    fn values_with_nul_bytes_are_refused_before_any_call() {
        assert!(cstring("a\0b").is_err());
    }
}

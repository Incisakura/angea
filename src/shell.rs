use std::env;
use std::ffi::{CStr, CString};
use std::mem;
use std::os::raw::c_void;
use std::os::unix::io::RawFd;
use std::ptr;
use std::process::{Command, Stdio};
use std::result::Result;

use libdbus_sys::*;

use crate::ptyfwd::PTYForward;

pub fn enter() {
    if is_inside() {
        println!("Systemd is already running in current PID namespace.");
        return;
    }

    let owned_fd = match unsafe { get_master() } {
        Ok(fd) => fd,
        Err(e) => unsafe { panic!("{}", CStr::from_ptr(e.message).to_str().unwrap()) },
    };
    let mut f = PTYForward::new(owned_fd);
    f.wait();
}

fn is_inside() -> bool {
    Command::new("/usr/bin/systemctl")
        .arg("is-system-running")
        .stdout(Stdio::null())
        .status()
        .map_or(false, |s| s.success())
}

unsafe fn get_master() -> Result<RawFd, DBusError> {
    unsafe fn append_string(iter: *mut DBusMessageIter, value: &str) {
        let value = CString::new(value).unwrap();
        dbus_message_iter_append_basic(
            iter,
            DBUS_TYPE_STRING,
            &value.as_ptr() as *const _ as *const c_void,
        );
    }
    
    unsafe fn append_array_string(
        m: *mut DBusMessage,
        iter: *mut DBusMessageIter,
        values: Vec<String>,
    ) {
        let signature = CString::new("s").unwrap();
        let mut i = mem::zeroed();
        dbus_message_iter_init_append(m, &mut i);
        dbus_message_iter_open_container(iter, DBUS_TYPE_ARRAY, signature.as_ptr(), &mut i);
        for value in values {
            append_string(&mut i, value.as_str());
        }
        dbus_message_iter_close_container(iter, &mut i);
    }

    let args: Vec<String> = Vec::new();
    let envs: Vec<String> = env::vars()
        .filter_map(|(k, v)| {
            if k == "PATH" || k == "TERM" || k == "LANG" {
                return Some(format!("{}={}", k, v));
            }
            None
        })
        .collect();

    // Init connection
    let mut e = DBusError {
        name: ptr::null(),
        message: ptr::null(),
        dummy: 0,
        padding1: ptr::null(),
    };
    dbus_error_init(&mut e);
    let conn = dbus_bus_get_private(DBusBusType::System, &mut e);
    if conn.is_null() {
        return Err(e);
    }

    // new method call
    let dest = CString::new("org.freedesktop.machine1").unwrap();
    let path = CString::new("/org/freedesktop/machine1").unwrap();
    let iface = CString::new("org.freedesktop.machine1.Manager").unwrap();
    let method = CString::new("OpenMachineShell").unwrap();
    let m = dbus_message_new_method_call(
        dest.as_ptr(),
        path.as_ptr(),
        iface.as_ptr(),
        method.as_ptr(),
    );

    // Append args
    let mut iter: DBusMessageIter = mem::zeroed();
    dbus_message_iter_init_append(m, &mut iter);
    append_string(&mut iter, ".host");
    append_string(&mut iter, "");
    append_string(&mut iter, "");
    append_array_string(m as *mut _, &mut iter, args);
    append_array_string(m as *mut _, &mut iter, envs);

    // Send message and recive reply
    let m = dbus_connection_send_with_reply_and_block(conn as *mut _, m as *mut _, 3000, &mut e);
    if m.is_null() {
        return Err(e);
    }

    // Get result
    let mut i: DBusMessageIter = mem::zeroed();
    dbus_message_iter_init(m, &mut i);
    let mut fd: RawFd = mem::zeroed();
    dbus_message_iter_get_basic(&mut i, &mut fd as *mut _ as *mut c_void);
    Ok(fd)
}

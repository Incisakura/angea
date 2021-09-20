use std::env;
use std::ffi::CStr;
use std::mem;
use std::os::unix::io::RawFd;
use std::process::{Command, Stdio};
use std::result::Result;

use libdbus_sys::*;

use crate::ptyfwd::PTYForward;

pub fn enter(user: &str) {
    if is_inside() {
        panic!("Systemd is already running in current PID namespace.");
    }

    let owned_fd = unsafe {
        get_master(user).unwrap_or_else(|e| {
            panic!("{}", CStr::from_ptr(e.message).to_str().unwrap());
        })
    };
    let mut f = PTYForward::new(owned_fd).unwrap_or_else(|e| panic!("{}", e.desc()));
    f.wait().unwrap_or_else(|e| panic!("{}", e.desc()));
}

fn is_inside() -> bool {
    Command::new("/usr/bin/systemctl")
        .arg("is-system-running")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_or(false, |s| s.success())
}

unsafe fn get_master(user: &str) -> Result<RawFd, DBusError> {
    unsafe fn append_string(iter: *mut DBusMessageIter, value: &str) {
        dbus_message_iter_append_basic(
            iter,
            DBUS_TYPE_STRING,
            &value.as_ptr() as *const _ as *const _,
        );
    }

    unsafe fn append_array_string(
        m: *mut DBusMessage,
        iter: *mut DBusMessageIter,
        values: Vec<String>,
    ) {
        let mut i: DBusMessageIter = mem::zeroed();
        dbus_message_iter_init_append(m, &mut i);
        dbus_message_iter_open_container(iter, DBUS_TYPE_ARRAY, "s\0".as_ptr() as *const _, &mut i);
        for value in values {
            append_string(&mut i, value.as_str());
        }
        dbus_message_iter_close_container(iter, &mut i);
    }

    // Prepare arguments
    let args: Vec<String> = Vec::new();
    let envs: Vec<String> = env::vars()
        .filter_map(|(k, v)| {
            if k == "PATH" || k == "TERM" || k == "LANG" {
                return Some(format!("{}={}\0", k, v));
            }
            None
        })
        .collect();
    let mut user = String::from(user);
    user.push('\0');

    // Init connection
    let mut e: DBusError = mem::zeroed();
    dbus_error_init(&mut e);
    let conn: *mut DBusConnection = dbus_bus_get_private(DBusBusType::System, &mut e);
    if conn.is_null() {
        return Err(e);
    }

    // new method call
    let m: *mut DBusMessage = dbus_message_new_method_call(
        "org.freedesktop.machine1\0".as_ptr() as *const _,
        "/org/freedesktop/machine1\0".as_ptr() as *const _,
        "org.freedesktop.machine1.Manager\0".as_ptr() as *const _,
        "OpenMachineShell\0".as_ptr() as *const _,
    );

    // Append args
    let mut iter: DBusMessageIter = mem::zeroed();
    dbus_message_iter_init_append(m, &mut iter);
    append_string(&mut iter, ".host\0");
    append_string(&mut iter, user.as_str());
    append_string(&mut iter, "\0");
    append_array_string(m, &mut iter, args);
    append_array_string(m, &mut iter, envs);

    // Send message and recive reply then close conn
    let msg = dbus_connection_send_with_reply_and_block(conn, m, 3000, &mut e);
    if m.is_null() {
        return Err(e);
    }

    // Get result
    let mut i: DBusMessageIter = mem::zeroed();
    dbus_message_iter_init(msg, &mut i);
    let mut fd: RawFd = mem::zeroed();
    dbus_message_iter_get_basic(&mut i, &mut fd as *mut _ as *mut _);

    // Release all pointer resources
    dbus_connection_close(conn);
    dbus_connection_unref(conn);
    dbus_message_unref(m);
    dbus_message_unref(msg);

    Ok(fd)
}

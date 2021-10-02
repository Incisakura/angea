use std::ffi::CStr;
use std::mem;
use std::os::raw::{c_char, c_int};
use std::ptr;

use libdbus_sys::*;

pub struct DBus {
    conn: *mut DBusConnection,
    error: DBusError,
    message: *mut DBusMessage,
    reply: *mut DBusMessage,
}

#[rustfmt::skip]
impl DBus {
    /// New DBus message & connection instance
    pub unsafe fn new() -> Result<DBus, String> {
        let message: *mut DBusMessage = dbus_message_new_method_call(
            "org.freedesktop.systemd1\0".as_ptr() as *const _,
            "/org/freedesktop/systemd1\0".as_ptr() as *const _,
            "org.freedesktop.systemd1.Manager\0".as_ptr() as *const _,
            "StartTransientUnit\0".as_ptr() as *const _,
        );
        let mut error: DBusError = mem::zeroed();
        let conn: *mut DBusConnection = dbus_bus_get_private(DBusBusType::System, &mut error);

        let dbus = DBus { conn, message, error, reply: ptr::null_mut() };
        if dbus.conn.is_null() {
            return Err(dbus.get_error());
        }
        Ok(dbus)
    }

    /// Append args to dbus message
    pub unsafe fn append(&mut self, user: &str, slave: &str, pts_id: &str, envs: Vec<String>) {
        let service = format!("container-shell@{}.service\0", pts_id);
        let user = format!("{}\0", user);
        let slave = format!("{}\0", slave);
        let mut i0: DBusMessageIter = mem::zeroed();
        dbus_message_iter_init_append(self.message, &mut i0);
        dbus_message_iter_append_basic(&mut i0, DBUS_TYPE_STRING, &service.as_ptr() as *const _ as *const _);
        dbus_message_iter_append_basic(&mut i0, DBUS_TYPE_STRING, &"fail\0".as_ptr() as *const _ as *const _);
        {
            let mut c = Container::new(self.message, &mut i0, DBUS_TYPE_ARRAY, "(sv)\0".as_ptr() as *const _);
            self.append_struct_ss(&mut c.sub, "User\0", user.as_str());
            self.append_struct_ss(&mut c.sub, "WorkingDirectory\0", "-~\0");
            self.append_struct_ss(&mut c.sub, "StandardInput\0", "tty\0");
            self.append_struct_ss(&mut c.sub, "StandardOutput\0", "tty\0");
            self.append_struct_ss(&mut c.sub, "StandardError\0", "tty\0");
            self.append_struct_ss(&mut c.sub, "TTYPath\0", slave.as_str());
            self.append_struct_exec(&mut c.sub);
            self.append_struct_envs(&mut c.sub, envs);
        }
        Container::new(self.message, &mut i0, DBUS_TYPE_ARRAY, "(sa(sv))\0".as_ptr() as *const _);
    }

    /// Send dbus message
    pub unsafe fn send(&mut self) -> Result<(), String> {
        self.reply = dbus_connection_send_with_reply_and_block(self.conn, self.message, 3000, &mut self.error);
        if self.reply.is_null() {
            return Err(self.get_error());
        }
        Ok(())
    }

    /// Get error message
    unsafe fn get_error(&self) -> String {
        match CStr::from_ptr(self.error.name).to_str() {
            Ok(s) => String::from(s),
            Err(e) => e.to_string(),
        }
    }

    /// Append struct ss: `String - String`
    unsafe fn append_struct_ss(&self, i: *mut DBusMessageIter, s: &str, v: &str) {
        let mut c = Container::new(self.message, i, DBUS_TYPE_STRUCT, ptr::null());
        dbus_message_iter_append_basic(&mut c.sub, DBUS_TYPE_STRING, &s.as_ptr() as *const _ as *const _);
        {
            let mut c = Container::new(self.message, &mut c.sub, DBUS_TYPE_VARIANT, "s\0".as_ptr() as *const _);
            dbus_message_iter_append_basic(&mut c.sub, DBUS_TYPE_STRING, &v.as_ptr() as *const _ as *const _);
        }
    }

    //// Append struct environment
    unsafe fn append_struct_envs(&self, i: *mut DBusMessageIter, envs: Vec<String>) {
        let mut c = Container::new(self.message, i, DBUS_TYPE_STRUCT, ptr::null());
        dbus_message_iter_append_basic(&mut c.sub, DBUS_TYPE_STRING, &"Environment\0".as_ptr() as *const _ as *const _);
        {
            let mut c = Container::new(self.message, &mut c.sub, DBUS_TYPE_VARIANT, "as\0".as_ptr() as *const _);
            {
                let mut c = Container::new(self.message, &mut c.sub, DBUS_TYPE_ARRAY, "s\0".as_ptr() as *const _);
                for env in envs {
                    dbus_message_iter_append_basic(&mut c.sub, DBUS_TYPE_STRING, &env.as_ptr() as *const _ as *const _);
                }
            }
        }
    }

    /// Append struct exec path and args
    unsafe fn append_struct_exec(&self, i: *mut DBusMessageIter) {
        let args = ["/bin/bash\0", "-l\0"];
        let mut c = Container::new(self.message, i, DBUS_TYPE_STRUCT, ptr::null());
        dbus_message_iter_append_basic(&mut c.sub, DBUS_TYPE_STRING, &"ExecStart\0".as_ptr() as *const _ as *const _);
        {
            let mut c = Container::new(self.message, &mut c.sub, DBUS_TYPE_VARIANT, "a(sasb)\0".as_ptr() as *const _);
            {
                let mut c = Container::new(self.message, &mut c.sub, DBUS_TYPE_ARRAY, "(sasb)\0".as_ptr() as *const _);
                {
                    let mut c = Container::new(self.message, &mut c.sub, DBUS_TYPE_STRUCT, ptr::null());
                    dbus_message_iter_append_basic(&mut c.sub, DBUS_TYPE_STRING, &args[0].as_ptr() as *const _ as *const _);
                    {
                        let mut c = Container::new(self.message, &mut c.sub, DBUS_TYPE_ARRAY, "s\0".as_ptr() as *const _);
                        for arg in args {
                            dbus_message_iter_append_basic(&mut c.sub, DBUS_TYPE_STRING, &arg.as_ptr() as *const _ as *const _);
                        }
                    }
                    dbus_message_iter_append_basic(&mut c.sub, DBUS_TYPE_BOOLEAN, &1 as *const _ as *const _);
                }
            }
        }
    }
}

// Drop pointer resources
impl Drop for DBus {
    fn drop(&mut self) {
        unsafe {
            dbus_connection_close(self.conn);
            dbus_connection_unref(self.conn);
            dbus_message_unref(self.message);
            dbus_message_unref(self.reply);
        }
    }
}

/// RAII dbus message iter container
struct Container {
    iter: *mut DBusMessageIter,
    pub sub: DBusMessageIter,
}

impl Container {
    pub unsafe fn new(
        msg: *mut DBusMessage,
        iter: *mut DBusMessageIter,
        _type: c_int,
        contained_signature: *const c_char,
    ) -> Container {
        let mut sub: DBusMessageIter = mem::zeroed();
        dbus_message_iter_init_append(msg, &mut sub);
        dbus_message_iter_open_container(iter, _type, contained_signature, &mut sub);
        Container { iter, sub }
    }
}

impl Drop for Container {
    fn drop(&mut self) {
        unsafe { dbus_message_iter_close_container(self.iter, &mut self.sub) };
    }
}

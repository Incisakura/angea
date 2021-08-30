use std::env;
use std::process::{Command, Stdio};

use dbus::arg::OwnedFd;
use dbus::ffidisp::{BusType, Connection};
use dbus::message::Message;
use dbus::Error;

use crate::ptyfwd::PTYForward;

pub fn enter() {
    if is_inside() {
        println!("Systemd is already running in current PID namespace.");
        return
    }

    let owned_fd = get_master().expect("");
    let mut f = PTYForward::new(owned_fd.into_fd());
    f.wait();
}

fn is_inside() -> bool {
    let status = Command::new("/usr/bin/systemctl")
        .arg("is-system-running")
        .stdout(Stdio::null())
        .status();
    match status {
        Ok(status) => status.success(),
        Err(_) => false
    }
}

fn get_master() -> Result<OwnedFd, Error> {
    let args: Vec<String> = Vec::new();
    let envs: Vec<String> = env::vars()
        .filter_map(|(k, v)| {
            if k == "PATH" || k == "TERM" || k == "LANG" {
                return Some(format!("{}={}", k, v));
            }
            None
        })
        .collect();
    let c = Connection::get_private(BusType::System)?;
    let m = Message::new_method_call(
        "org.freedesktop.machine1",
        "/org/freedesktop/machine1",
        "org.freedesktop.machine1.Manager",
        "OpenMachineShell",
    )
    .unwrap()
    // host | user | path
    .append3(".host", "", "")
    // args | envs
    .append2(args, envs);
    let r = c.send_with_reply_and_block(m, 3000)?;
    let (owned_fd, _): (OwnedFd, String) = r.read2()?;
    // return ownedfd to keep ownership
    Ok(owned_fd)
}

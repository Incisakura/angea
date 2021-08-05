use std::fs::File;

use unshare::{Command, Namespace, Stdio};

use crate::systemd::Systemd;

pub fn enter() {
    if is_inside() {
        println!("Systemd is already running in current PID namespace.");
        return
    }

    let systemd = Systemd::new();
    let pid_file = File::open(format!("/proc/{}/ns/pid", &systemd.pid)).unwrap();
    let mut child = Command::new("/bin/bash")
        .set_namespace(&pid_file, Namespace::Pid)
        .unwrap()
        .spawn()
        .unwrap();
    child.wait().unwrap();
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

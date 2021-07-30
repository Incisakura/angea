use std::fs::File;

use unshare::{Command, Namespace};

use crate::systemd::Systemd;

pub fn enter() {
    let systemd = Systemd::new();
    let mut child = Command::new("/bin/bash")
        .set_namespace(&mut File::open(format!("/proc/{}/ns/pid", &systemd.pid)).unwrap(), Namespace::Pid)
        .unwrap()
        .spawn()
        .unwrap();
    child.wait().unwrap();
}

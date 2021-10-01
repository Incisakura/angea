use std::env;
use std::fs;
use std::os::unix::io::RawFd;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::result::Result;
use std::thread;
use std::time::Duration;

use nix::errno::Errno;
use nix::pty::openpty;
use nix::unistd::{close, read};

use crate::dbus::DBus;
use crate::ptyfwd::PTYForward;

pub fn enter(user: &str) {
    if is_inside() {
        panic!("Systemd is already running in current PID namespace.");
    }

    let master = unsafe { get_pty(user).unwrap_or_else(|e| panic!("{}", e)) };
    let mut f = PTYForward::new(master).unwrap_or_else(|e| panic!("{}", e.desc()));
    f.wait().unwrap_or_else(|e| eprintln!("{}", e.desc()));
    f.disconnect().unwrap(); // should no error here
}

fn is_inside() -> bool {
    Command::new("/usr/bin/systemctl")
        .arg("is-system-running")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_or(false, |s| s.success())
}

unsafe fn get_pty(user: &str) -> Result<RawFd, String> {
    // collect environment variables
    let envs: Vec<String> = env::vars()
        .filter_map(|(k, v)| {
            if k == "PATH" || k == "TERM" || k == "LANG" {
                return Some(format!("{}={}\0", k, v));
            }
            None
        })
        .collect();

    // init pty peer
    let pty = openpty(None, None).map_err(|e| e.desc())?;
    let path = PathBuf::from(format!("/proc/self/fd/{}", pty.slave));
    let path = fs::read_link(path).map_err(|e| e.to_string())?;
    let pts_id = path.to_str().unwrap().trim_start_matches("/dev/pts/");

    // dbus message
    let mut dbus = DBus::new()?;
    dbus.append(user, path.to_str().unwrap(), pts_id, envs);
    dbus.send()?;

    // no longer used, close
    close(pty.slave).map_err(|e| e.desc())?;
    wait_master(pty.master)?;

    Ok(pty.master)
}

/// Wait master for readable
fn wait_master(master: RawFd) -> Result<(), String> {
    let mut buff = [0; 128];
    for _ in 1..10 {
        thread::sleep(Duration::from_millis(100));
        match read(master, &mut buff) {
            Ok(_) => return Ok(()),
            Err(Errno::EIO) => continue,
            Err(e) => return Err(e.desc().to_string()),
        }
    }
    Err("Waiting master readable timeout".to_string())
}

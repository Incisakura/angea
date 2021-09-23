use std::ffi::CString;
use std::fs;
use std::io;
use std::str::FromStr;

use nix::mount::{mount, MsFlags};
use nix::sched::{clone, CloneFlags};
use nix::sys::signal::{kill, Signal};
use nix::unistd::{execve, Pid};
use nix::Result;

pub struct Systemd(Pid);

impl Systemd {
    /// Try to fetch running systemd
    pub fn fetch() -> Option<Systemd> {
        let raw_pid = Self::from_proc().expect("Unable to read /proc");
        if raw_pid == 0 {
            return None;
        }
        Some(Self(Pid::from_raw(raw_pid)))
    }

    /// Try to fetch systemd or create a new one
    pub fn fetch_or_create() -> Systemd {
        match Self::fetch() {
            Some(s) => s,
            None => Self::create().expect("Unable to spawn systemd"),
        }
    }

    /// Try to fetch systemd from /proc
    fn from_proc() -> io::Result<i32> {
        for entry in fs::read_dir("/proc")? {
            let mut path = entry?.path();
            if path.is_dir() {
                let pid = match path.file_name().and_then(|f| f.to_str()).map(i32::from_str) {
                    Some(Ok(p)) => p,
                    _ => continue,
                };
                path.push("comm");
                let comm = match fs::read_to_string(path) {
                    Ok(str) => str,
                    _ => continue,
                };
                if comm == "systemd\n" {
                    return Ok(pid);
                }
            }
        }
        return Ok(0);
    }

    pub fn shutdown(self) {
        if self.0.as_raw() != 0 {
            kill(self.0, Signal::SIGKILL).unwrap_or_else(|_| {
                panic!(
                    "Unable to kill systemd PID: {}. Are you in container?",
                    self.0.as_raw()
                );
            });
        }
    }

    fn create() -> Result<Systemd> {
        // Spawn child process
        let mut stack = [0; 4096];
        let pid = clone(
            Box::new(|| -> isize {
                let args: Vec<CString> = vec![CString::new("/lib/systemd/systemd").unwrap()];
                let environ: Vec<CString> = Vec::new();
                mount(
                    Some("proc"),
                    "/proc",
                    Some("proc"),
                    MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC | MsFlags::MS_NODEV,
                    None::<&str>,
                )
                .unwrap();
                execve(args[0].as_c_str(), &args, &environ).unwrap();
                0
            }),
            &mut stack,
            CloneFlags::CLONE_NEWPID | CloneFlags::CLONE_NEWNS,
            None,
        )?;

        return Ok(Systemd(pid));
    }
}

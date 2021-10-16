use std::ffi::{CString, OsStr};
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
    /// Try to fetch a running systemd process
    pub fn fetch() -> Option<Systemd> {
        Self::from_proc().expect("Failed to read procfs")
    }

    /// Create a new systemd process if necessary
    pub fn fetch_or_create() {
        if Self::fetch().is_none() {
            Self::create().expect("Failed to spawn systemd process");
        }
    }

    /// Kill running systemd process
    pub fn shutdown(self) {
        kill(self.0, Signal::SIGKILL).expect("Failed to kill systemd. Are you in container?");
    }

    /// Start a new systemd process
    fn create() -> Result<Systemd> {
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
                unreachable!();
            }),
            &mut stack,
            CloneFlags::CLONE_NEWPID | CloneFlags::CLONE_NEWNS,
            None,
        )?;
        Ok(Systemd(pid))
    }

    /// Try to fetch `Systemd` from procfs
    fn from_proc() -> io::Result<Option<Systemd>> {
        for entry in fs::read_dir("/proc")? {
            let mut path = entry?.path();
            if path.is_dir() {
                let pid = match path.file_name().and_then(OsStr::to_str).map(i32::from_str) {
                    Some(Ok(p)) => p,
                    _ => continue,
                };
                path.push("comm"); // /proc/PID/comm
                if fs::read_to_string(path)? == "systemd\n" {
                    return Ok(Some(Systemd(Pid::from_raw(pid))));
                }
            }
        }
        Ok(None)
    }
}

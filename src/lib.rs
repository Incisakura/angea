mod shell;

mod systemd;

use nix::Result;
use shell::{get_pty, PTYForward};
use std::env;

pub fn cmd() {
    let args: Vec<String> = env::args().collect();
    let r = match args.get(1) {
        Some(c) => match c.as_str() {
            "boot" => cmd_boot(),
            "shutdown" => cmd_shutdown(),
            "shell" => cmd_shell(&args),
            _ => cmd_help(),
        },
        None => cmd_help(),
    };
    if let Err(e) = r {
        eprintln!("{}", e);
    }
}

fn cmd_shell(args: &[String]) -> Result<()> {
    cmd_boot()?;

    let user = args.get(2).map_or("root", |s| s.as_str());
    let master = get_pty(user)?;
    let mut f = PTYForward::new(master)?;
    f.wait()?;
    f.disconnect()?;
    Ok(())
}

fn cmd_boot() -> Result<()> {
    if systemd::from_proc()?.is_none() {
        systemd::start()?;
    }
    Ok(())
}

fn cmd_shutdown() -> Result<()> {
    if let Some(p) = systemd::from_proc()? {
        systemd::shutdown(p);
    }
    Ok(())
}

fn cmd_help() -> Result<()> {
    print!(concat!(
        "Angea version ",
        env!("CARGO_PKG_VERSION"),
        "
Usage: angea <command> [more]
Command:
    boot            Start systemd
    shell [user]    Open a shell in systemd. [Default: root]
    shutdown        Kill running systemd
    help            This message
"
    ));
    Ok(())
}

use std::env;

use angea::shell;
use angea::systemd::Systemd;

fn main() {
    let args: Vec<String> = env::args().collect();
    match args.get(1) {
        Some(c) => match c.as_str() {
            "boot" => {
                Systemd::fetch_or_create();
            }
            "shell" => {
                Systemd::fetch_or_create();
                shell::enter();
            }
            "shutdown" => {
                Systemd::fetch().map(|s| s.shutdown());
            }
            _ => help(),
        },
        None => help(),
    };
}

fn help() {
    print!(
        "Angea v0.0.4
Usage: angea <command>
Command:
    boot        Boot systemd as daemon
    shell       Open a shell in container
    shutdown    Kill running systemd
    help        This message
"
    );
}

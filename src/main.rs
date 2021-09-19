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
                let user = args.get(2).map_or("root", |s| s.as_str());
                Systemd::fetch_or_create();
                shell::enter(user);
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
Usage: angea <command> [more]
Command:
    boot            Boot systemd as daemon
    shell [user]    Open a shell in container. Default: root
    shutdown        Kill running systemd
    help            This message
"
    );
}

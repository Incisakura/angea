# Angea

---
> Naming from hydrangea(アジサイ)

A command-line tool to make systemd work in WSL2(Windows Subsystem for Linux 2)

## How it works

Systemd needs to work on PID 1, so Angea creates a new PID namespace for systemd.

## How to use

See `angea help`

>Usage: angea `command`
>
>Command:
>
> - **boot**: Boot systemd as daemon
> - **shell**: Init bash shell in container
> - **shutdown**: Kill runing systemd

If you want to use shell completely inside systemd, please install `systemd-container`, then run `machinectl shell {username}@` after `angea boot`.

## Credit

Incisakura &lt;incisakura@icloud.com>

## Licence

MIT

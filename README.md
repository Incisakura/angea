# Angea

---
> Naming from hydrangea(アジサイ)

A command line tool to make systemd works in WSL2(Windows Subsystem for Linux 2).

## How it works

Systemd needs to work on PID 1, so Angea creates a new PID namaspace for systemd.

## How to use

See `angea help`

>Usage: angea `command`
>command:
>
> - **boot**: Boot systemd as daemon
> - **shell**: Init bash shell in container
> - **shutdown**: Kill runing systemd

## Credit

Incisakura &lt;incisakura@icloud.com>

## Licence

MIT

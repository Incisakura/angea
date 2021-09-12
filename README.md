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
> - **shell**: Open a shell in container
> - **shutdown**: Kill running systemd

## Requirement

`systemd-container`

In some distributions, it isn't built-in with systemd or preinstalled.

So you may need install it manually before running `angea shell`.

## Credit

Incisakura &lt;incisakura@icloud.com>

## Licence

MIT

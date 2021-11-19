# Angea

---
> Naming from hydrangea(アジサイ)

A lite tool to make systemd work in any container(Windows Subsystem for Linux 2, Docker, Podman, etc.)

**WSL1 is not supported.**

## Usage

See `angea help`

## Advanced Usage

### Custom Shell

Run `angea shell` with envivonment variable `ARGS`. (default args in example below)

``` bash
ARGS="/usr/bin/bash -l" angea shell
```

### Custom Envivonment Variable

Notice: Wroung environment variable passed may trigger an error.

``` bash
// Set Envivonment Variable
ENVS="TERM=xterm-256color,WSL=1" angea shell

// Inherit Envivonment Variable
ENV_INHERIT="TERM,WT_SESSION" angea shell

// Both
ENVS="TERM=xterm-256color" ENV_INHERIT="WT_SESSION" angea shell
```

## Requirement

No! But you should install `systemd` as least.

## Credit

Incisakura &lt;incisakura@icloud.com>

## Licence

MIT

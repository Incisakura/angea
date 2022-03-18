# Angea

---
> Naming from hydrangea(アジサイ)

A lite tool to make systemd work in Windows Subsystem for Linux 2

**WSL1 is not supported.**

## Usage

See `angea help`

## Advanced Usage

### Custom Shell Program

Run `angea shell` with envivonment variable `ARGS`. (default args in example below)

**Note**: The first argument must be a absolute path.

``` bash
ANGEA_ARGS="/usr/bin/bash -l" angea shell
```

### Custom Envivonment Variable

Notice: Wroung environment variable passed may trigger an error.

``` bash
// Set Envivonment Variable
ANGEA_ENVS="TERM=xterm-256color,WSL=1" angea shell

// Inherit Envivonment Variable
// If `ANGEA_ENV_INHERIT` is not set, angea would inherit `TERM` by default
ANGEA_ENV_INHERIT="TERM,WT_SESSION" angea shell

// Both
ANGEA_ENVS="TERM=xterm-256color" ENV_INHERIT="WT_SESSION" angea shell
```

## Requirement

Nothing! But you should install `systemd` as least.

## Credit

Incisakura &lt;incisakura@icloud.com>

## Licence

MIT

<h1 align="center">Crier</h1>

<p align="center">
  <a href="Cargo.toml"><img src="https://img.shields.io/badge/version-0.3.0-f9a477?style=flat-square" alt="Version 0.3.0"></a>
  <a href="https://aur.archlinux.org/packages/crier"><img src="https://img.shields.io/aur/version/crier?style=flat-square&amp;logo=archlinux&amp;logoColor=white&amp;color=1793d1" alt="AUR package version"></a>
  <a href="https://github.com/skorotkiewicz/crier/actions/workflows/release.yml"><img src="https://github.com/skorotkiewicz/crier/actions/workflows/release.yml/badge.svg" alt="Build and release status"></a>
  <a href="Cargo.toml"><img src="https://img.shields.io/badge/Rust-2021-f9a477?style=flat-square&amp;logo=rust&amp;logoColor=white" alt="Rust edition 2021"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-64dabb?style=flat-square" alt="MIT license"></a>
</p>

<p align="center">
  Send notifications over TCP or MQTT and run a local command for each message.
</p>

## Install

```sh
cargo build --release
# Binary: ./target/release/crier
```

## TCP

Start a listener and send a notification:

```sh
crier listen 0.0.0.0:5555 -m 'notify-send "Build" "{}"' --auth example-token
crier send localhost:5555 -m 'Build complete!' --auth example-token
```

On Windows, a listener can use `-m 'msg * "{}"'` in PowerShell.
TCP requires an explicit address and port. Messages must fit on one line and
contain at most 65536 UTF-8 bytes. The sender reports success after the listener
runs the command successfully and replies `OK`.

## MQTT

```sh
crier listen --relay broker.emqx.io -t ci/example -m 'notify-send "Build" "{}"'
crier send --relay broker.emqx.io -t ci/example -m 'Build complete!'
```

The broker port defaults to 1883. Override it with `--port`.
Messages can contain newlines and at most 65536 UTF-8 bytes.
The sender waits up to five seconds for the broker to acknowledge publication.
This confirms broker receipt. The listener command can still fail or the listener
can be offline. MQTT QoS 1 can deliver a message more than once, so handlers
should tolerate duplicates.

Listeners reconnect after broker errors and restore their subscriptions.

## Command templates

Crier splits the template into an executable and arguments, then replaces `{}`
in each argument with the received text. Quotes group arguments. Message text
is passed literally, including quotes, dollar signs, and shell operators.
The executable must be fixed and cannot contain `{}`.

```sh
crier listen localhost:5555 -m 'notify-send "Build status" "{}"'
crier listen localhost:5555 -m './on-message.sh "{}"'
```

Crier also sets `CRIER_MESSAGE` in the command's environment.
Commands run directly. Shell operators such as `|` and `>>`, variable expansion,
and shell builtins need an explicit script or shell. Existing templates that
use those operations must be updated.

For example, save this as an executable `on-message.sh` and select it with
`-m './on-message.sh'`:

```sh
#!/bin/sh
printf '[%s] %s\n' "$(date)" "$CRIER_MESSAGE" >> "$HOME/crier.log"
```

Keep message data out of script source. When explicitly invoking an interpreter,
pass the message as an argument or read `CRIER_MESSAGE` inside the script.
Commands run synchronously. A long-running command delays other notifications
and can cause the sender's five-second response deadline to expire. Use a handler
that returns promptly.

## Presets

The default config file is `crier.yml` in the platform config directory, usually
`~/.config/crier.yml` on Linux. Use `-c FILE` to select another file.
See [crier.example.yml](crier.example.yml) for a complete example.

```yaml
default_preset: local

local:
  addr: "localhost:5555"
  auth: example-token
  message: 'notify-send "Build" "{}"'

mybuilds:
  relay: broker.emqx.io
  port: 1883
  topic: ci/example
  message: 'notify-send "Build" "{}"'
```

```sh
crier listen -p mybuilds
crier send -p mybuilds -m 'Build passed!'
crier -c ./project.yml listen -p local
```

CLI options override the selected preset, including an explicit `--port 1883`.
An explicit TCP address overrides a preset's relay, and `--relay` overrides a
preset's TCP address. A preset must otherwise select exactly one transport.

Without `-p`, explicit addresses or `--relay` work independently of presets.
If neither transport is supplied, Crier uses `default_preset`, then a preset
named `local` if present. A missing default config file is allowed. Malformed
or unreadable config files, unknown preset fields, missing named presets, and
missing explicitly selected config files produce errors.

## Authentication

Use the same `--auth TOKEN` on the sender and listener, or set `auth` in a preset.
Tokens must contain 1 to 1024 bytes and cannot contain colons, newlines, or NUL.

The token is part of the message protocol. TCP and MQTT connections currently
use plaintext, and the token does not provide encryption or replay protection.
Public MQTT subscribers can see payloads and tokens. Use a trusted broker and
network for private notifications.

## Options

```text
USAGE:
  crier listen [OPTIONS] [ADDR]
  crier send [OPTIONS] [ADDR]

SUBCOMMANDS:
  listen                    Listen for messages and run a command
  send                      Send a message

GLOBAL OPTIONS:
  -c, --config <FILE>       Use a different config file
  -e, --examples            Show usage examples
  -h, --help                Print help
  -V, --version             Print version

COMMON OPTIONS:
  -p, --preset <NAME>       Use a named preset from the config file
  -m, --message <TEXT>      Command to run (listen) or text to send (send)
  -a, --auth <TOKEN>        Authentication token, shared by sender and listener

MQTT MODE:
  --relay <BROKER>          MQTT broker hostname (e.g., broker.emqx.io)
  --port <PORT>             MQTT broker port (default: 1883)
  -t, --topic <TOPIC>       Topic to listen on or send to

TCP MODE:
  <ADDR>                    Address and port to listen on or send to
                            Example: localhost:5555
```

The default config file is `crier.yml` in the platform config directory,
usually `~/.config/crier.yml` on Linux.

With `default_preset: local` in your config, you can simply run:

```sh
crier listen
crier send -m 'Build passed!'
```

## Development

```sh
just check
just test
```

The tests use local TCP sockets and a simulated MQTT broker. They do not require
an external broker or notification service. Release validation includes Linux
and Windows tests. The release workflow runs only for tags.

## License

MIT

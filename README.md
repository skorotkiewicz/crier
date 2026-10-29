# Crier

Send notifications over TCP or MQTT and run a local command for each message.

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
crier listen --relay test.mosquitto.org -t ci/example -m 'notify-send "Build" "{}"'
crier send --relay test.mosquitto.org -t ci/example -m 'Build complete!'
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
  relay: test.mosquitto.org
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

| Option | Purpose |
| --- | --- |
| `listen` | Receive messages and run a command |
| `send` | Send a notification |
| `-c, --config FILE` | Select a config file |
| `-p, --preset NAME` | Select a named preset |
| `-m, --message TEXT` | Listener command or sender message |
| `-a, --auth TOKEN` | Shared authentication token |
| `--relay BROKER` | MQTT broker hostname |
| `--port PORT` | MQTT broker port, default 1883 |
| `-t, --topic TOPIC` | MQTT topic or listener topic filter |
| `ADDR` | TCP bind address or destination |
| `-e, --examples` | Print examples |
| `-h, --help` | Print help |
| `-V, --version` | Print version |

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

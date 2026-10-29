use crate::args::{Args, Commands, EndpointArgs};
use crate::command_template::CommandTemplate;
use crate::config::Config;
use crate::{direct, relay, wire};
use clap::Parser;
use std::io;
use std::path::Path;
use std::process::ExitCode;

/// Run the notification CLI and report operational failures with a nonzero exit code.
pub fn main() -> ExitCode {
    match run(Args::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Args) -> io::Result<()> {
    if args.examples {
        println!("TCP:\n  crier listen 0.0.0.0:5555 -m 'notify-send \"Status\" \"{{}}\"'\n  crier send localhost:5555 -m 'Build done!'\n\nMQTT:\n  crier listen --relay broker.emqx.io -t mybuilds -m 'notify-send \"Status\" \"{{}}\"'\n  crier send --relay broker.emqx.io -t mybuilds -m 'Build done!'\n\nPresets:\n  crier -c ./project.yml listen -p build\n  crier -c ./project.yml send -p build -m 'Build done!'");
        return Ok(());
    }
    let command = args.command.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "A subcommand is required. Try: crier --help",
        )
    })?;
    match command {
        Commands::Listen(endpoint) => execute(endpoint, args.config.as_deref(), Mode::Listen),
        Commands::Send(endpoint) => execute(endpoint, args.config.as_deref(), Mode::Send),
    }
}

enum Mode {
    Listen,
    Send,
}

fn execute(args: EndpointArgs, config_path: Option<&Path>, mode: Mode) -> io::Result<()> {
    let preset = Config::load(config_path)?.resolve(args)?;
    wire::validate_auth(preset.auth.as_deref())?;
    let message = preset
        .message
        .as_deref()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "--message is required"))?;
    let auth = preset.auth.as_deref();
    match (preset.addr.as_deref(), preset.relay.as_deref()) {
        (Some(addr), None) if !addr.is_empty() => match mode {
            Mode::Listen => direct::listen(addr, &CommandTemplate::parse(message)?, auth),
            Mode::Send => direct::send(addr, message, auth),
        },
        (None, Some(broker)) if !broker.is_empty() => {
            let port = preset.port.unwrap_or(1883);
            if port == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "MQTT port must be greater than zero",
                ));
            }
            let topic = preset
                .topic
                .as_deref()
                .filter(|topic| {
                    !topic.is_empty() && topic.len() <= u16::MAX as usize && !topic.contains('\0')
                })
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "A valid --topic is required with --relay",
                    )
                })?;
            match mode {
                Mode::Listen => {
                    relay::listen(broker, port, topic, &CommandTemplate::parse(message)?, auth)
                }
                Mode::Send => relay::send(broker, port, topic, message, auth),
            }
        }
        (Some(_), Some(_)) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Choose either a TCP address or --relay",
        )),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Provide an address, --relay, or --preset",
        )),
    }
}

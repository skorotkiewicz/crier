use clap::{Args as ClapArgs, Parser, Subcommand};
use std::path::PathBuf;

/// Send notifications over TCP or MQTT and run local notification commands.
#[derive(Parser)]
#[command(name = "crier", version, about)]
pub(crate) struct Args {
    /// Config file path, defaulting to the platform config directory/crier.yml
    #[arg(long, short = 'c', value_name = "FILE", global = true)]
    pub(crate) config: Option<PathBuf>,

    /// Show usage examples
    #[arg(long, short = 'e', global = true)]
    pub(crate) examples: bool,

    #[command(subcommand)]
    pub(crate) command: Option<Commands>,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
    /// Listen for messages and run a command with literal message arguments
    Listen(EndpointArgs),
    /// Send a message
    Send(EndpointArgs),
}

#[derive(ClapArgs, Default)]
pub(crate) struct EndpointArgs {
    /// Use a named preset from the config file
    #[arg(long, short = 'p', value_name = "NAME")]
    pub(crate) preset: Option<String>,

    /// TCP bind address for listen, or target address for send
    #[arg(value_name = "ADDR", conflicts_with = "relay")]
    pub(crate) addr: Option<String>,

    /// MQTT broker hostname
    #[arg(long, value_name = "BROKER")]
    pub(crate) relay: Option<String>,

    /// MQTT broker port [default: 1883]
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..))]
    pub(crate) port: Option<u16>,

    /// MQTT topic
    #[arg(long, short = 't', value_name = "TOPIC")]
    pub(crate) topic: Option<String>,

    /// Command template for listen, or message text for send
    #[arg(long, short)]
    pub(crate) message: Option<String>,

    /// Shared authentication token
    #[arg(long, short)]
    pub(crate) auth: Option<String>,
}

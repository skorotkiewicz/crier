use clap::{Parser, Subcommand};
use rumqttc::{Client, Event, MqttOptions, Packet, QoS};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

/// Crier - Simple push notification tool
#[derive(Parser, Debug)]
#[command(name = "crier", version, about)]
struct Args {
    /// Config file path (default: ~/.config/crier.yml)
    #[arg(long, short = 'c', value_name = "FILE", global = true)]
    config: Option<PathBuf>,

    /// Show usage examples
    #[arg(long, short = 'e', global = true)]
    examples: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Listen for messages
    Listen {
        /// Use preset from config file
        #[arg(long, short = 'p', value_name = "NAME")]
        preset: Option<String>,

        /// Direct mode: bind address (e.g., 0.0.0.0:5555)
        #[arg(value_name = "ADDR")]
        addr: Option<String>,

        /// Relay mode: MQTT broker (e.g., test.mosquitto.org)
        #[arg(long, value_name = "BROKER")]
        relay: Option<String>,

        /// MQTT broker port (default: 1883)
        #[arg(long, default_value = "1883")]
        port: u16,

        /// Topic for relay mode
        #[arg(long, short = 't', value_name = "TOPIC")]
        topic: Option<String>,

        /// Command to run (use {} as message placeholder)
        #[arg(long, short)]
        message: Option<String>,

        /// Authentication token
        #[arg(long, short)]
        auth: Option<String>,
    },

    /// Send a message
    Send {
        /// Use preset from config file
        #[arg(long, short = 'p', value_name = "NAME")]
        preset: Option<String>,

        /// Direct mode: target address (e.g., 192.168.1.10:5555)
        #[arg(value_name = "ADDR")]
        addr: Option<String>,

        /// Relay mode: MQTT broker (e.g., test.mosquitto.org)
        #[arg(long, value_name = "BROKER")]
        relay: Option<String>,

        /// MQTT broker port (default: 1883)
        #[arg(long, default_value = "1883")]
        port: u16,

        /// Topic for relay mode
        #[arg(long, short = 't', value_name = "TOPIC")]
        topic: Option<String>,

        /// Message to send
        #[arg(long, short)]
        message: Option<String>,

        /// Authentication token
        #[arg(long, short)]
        auth: Option<String>,
    },
}

// ============= CONFIG =============

#[derive(Debug, Deserialize, Default, Clone)]
struct Preset {
    addr: Option<String>,
    relay: Option<String>,
    port: Option<u16>,
    topic: Option<String>,
    message: Option<String>,
    auth: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct Config {
    #[serde(flatten)]
    presets: HashMap<String, Preset>,
    default_preset: Option<String>,
}

fn config_path(custom: Option<&PathBuf>) -> PathBuf {
    custom.cloned().unwrap_or_else(|| {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("crier.yml")
    })
}

fn load_config(custom_path: Option<&PathBuf>) -> Config {
    let path = config_path(custom_path);
    if path.exists() {
        match fs::read_to_string(&path) {
            Ok(content) => serde_yaml::from_str(&content).unwrap_or_default(),
            Err(_) => Config::default(),
        }
    } else {
        Config::default()
    }
}

fn get_preset(name: &str, custom_path: Option<&PathBuf>) -> Preset {
    let config = load_config(custom_path);
    let path = config_path(custom_path);

    let preset_name = if name.is_empty() {
        config.default_preset.as_deref().unwrap_or("local")
    } else {
        name
    };

    config.presets.get(preset_name).cloned().unwrap_or_else(|| {
        eprintln!("Error: Preset '{}' not found in {:?}", preset_name, path);
        eprintln!(
            "Available presets: {:?}",
            config.presets.keys().collect::<Vec<_>>()
        );
        std::process::exit(1);
    })
}

fn print_examples() {
    println!("EXAMPLES:");
    println!();
    println!("  # TCP mode");
    println!("  crier listen 0.0.0.0:5555 -m 'notify-send \"Status\" \"{{}}\"'");
    println!("  crier send 192.168.1.10:5555 -m 'Build done!'");
    println!();
    println!("  # MQTT mode");
    println!("  crier listen --relay test.mosquitto.org -t mybuilds -m 'notify-send \"Status\" \"{{}}\"'");
    println!("  crier send --relay test.mosquitto.org -t mybuilds -m 'Build done!'");
    println!();
    println!("  # Using presets from ~/.config/crier.yml");
    println!("  crier listen -p mypreset");
    println!("  crier send -p mypreset -m 'Build done!'");
    println!();
    println!("  # Custom config file");
    println!("  crier -c ./project.yml listen -p build");
}

// ============= MAIN =============

fn main() {
    let args = Args::parse();

    // Show examples if requested
    if args.examples {
        print_examples();
        return;
    }

    let config_path = args.config.as_ref();

    let command = args.command.unwrap_or_else(|| {
        eprintln!("Error: A subcommand is required (listen or send)");
        eprintln!("Try: crier --help");
        std::process::exit(1);
    });

    match command {
        Commands::Listen {
            preset,
            addr,
            relay,
            port,
            topic,
            message,
            auth,
        } => {
            // Load preset if specified
            let p = preset
                .as_ref()
                .map(|n| get_preset(n, config_path))
                .unwrap_or_else(|| get_preset("", config_path));

            // CLI overrides preset
            let addr = addr.or(p.addr);
            let relay = relay.or(p.relay);
            let port = if port != 1883 {
                port
            } else {
                p.port.unwrap_or(1883)
            };
            let topic = topic.or(p.topic);
            let message = message.or(p.message);
            let auth = auth.or(p.auth);

            let message = message.unwrap_or_else(|| {
                eprintln!("Error: --message is required");
                std::process::exit(1);
            });

            if let Some(broker) = relay {
                let topic = topic.unwrap_or_else(|| {
                    eprintln!("Error: --topic is required with --relay");
                    std::process::exit(1);
                });
                relay_listen(&broker, port, &topic, &message, auth.as_deref());
            } else if let Some(addr) = addr {
                direct_listen(&addr, &message, auth.as_deref());
            } else {
                eprintln!("Error: Provide address, --relay, or --preset");
                std::process::exit(1);
            }
        }
        Commands::Send {
            preset,
            addr,
            relay,
            port,
            topic,
            message,
            auth,
        } => {
            // Load preset if specified
            let p = preset
                .as_ref()
                .map(|n| get_preset(n, config_path))
                .unwrap_or_else(|| get_preset("", config_path));

            // CLI overrides preset
            let addr = addr.or(p.addr);
            let relay = relay.or(p.relay);
            let port = if port != 1883 {
                port
            } else {
                p.port.unwrap_or(1883)
            };
            let topic = topic.or(p.topic);
            let message = message.or(p.message);
            let auth = auth.or(p.auth);

            let message = message.unwrap_or_else(|| {
                eprintln!("Error: --message is required");
                std::process::exit(1);
            });

            if let Some(broker) = relay {
                let topic = topic.unwrap_or_else(|| {
                    eprintln!("Error: --topic is required with --relay");
                    std::process::exit(1);
                });
                relay_send(&broker, port, &topic, &message, auth.as_deref());
            } else if let Some(addr) = addr {
                direct_send(&addr, &message, auth.as_deref());
            } else {
                eprintln!("Error: Provide address, --relay, or --preset");
                std::process::exit(1);
            }
        }
    }
}

// ============= RELAY MODE (MQTT) =============

fn relay_listen(broker: &str, port: u16, topic: &str, cmd_template: &str, auth: Option<&str>) {
    let mut opts = MqttOptions::new("crier-listener", broker, port);
    opts.set_keep_alive(Duration::from_secs(60));

    let (client, mut connection) = Client::new(opts, 10);
    client.subscribe(topic, QoS::AtLeastOnce).unwrap();

    println!("Connected to: {}", broker);
    println!("Topic: {}", topic);
    println!("Command: {}", cmd_template);
    if auth.is_some() {
        println!("Auth: enabled");
    }
    println!("Waiting for messages...\n");

    for event in connection.iter().flatten() {
        if let Event::Incoming(Packet::Publish(msg)) = event {
            let payload = String::from_utf8_lossy(&msg.payload);

            // Check auth if required
            let message = if let Some(expected) = auth {
                if let Some(stripped) = payload.strip_prefix(&format!("AUTH:{}:", expected)) {
                    stripped.to_string()
                } else {
                    eprintln!("Auth failed, ignoring message");
                    continue;
                }
            } else {
                payload.to_string()
            };

            println!("Received: {}", message);
            let cmd = cmd_template.replace("{}", &message);
            run_command(&cmd);
        }
    }
}

fn relay_send(broker: &str, port: u16, topic: &str, message: &str, auth: Option<&str>) {
    let mut opts = MqttOptions::new("crier-sender", broker, port);
    opts.set_keep_alive(Duration::from_secs(5));

    let (client, mut connection) = Client::new(opts, 10);

    // Prepend auth to message if provided
    let payload = match auth {
        Some(a) => format!("AUTH:{}:{}", a, message),
        None => message.to_string(),
    };

    client
        .publish(topic, QoS::AtMostOnce, false, payload.as_bytes())
        .unwrap();

    // Poll connection briefly to actually send the message
    let start = std::time::Instant::now();
    let timeout = Duration::from_secs(5);

    for event in connection.iter() {
        if start.elapsed() > timeout {
            eprintln!("Timeout waiting for broker");
            std::process::exit(1);
        }
        match event {
            Ok(Event::Outgoing(rumqttc::Outgoing::Publish(_))) => {
                println!("Sent via {}: {}", broker, message);
                return;
            }
            Ok(Event::Incoming(Packet::ConnAck(_))) => {
                // Connected, continue polling
            }
            Err(e) => {
                eprintln!("Error: {:?}", e);
                std::process::exit(1);
            }
            _ => {}
        }
    }
}

// ============= DIRECT MODE (TCP) =============

fn direct_listen(addr: &str, cmd_template: &str, auth: Option<&str>) {
    let listener = TcpListener::bind(addr).unwrap_or_else(|e| {
        eprintln!("Failed to bind {}: {}", addr, e);
        std::process::exit(1);
    });

    println!("Listening on {}", addr);
    println!("Command: {}", cmd_template);
    if auth.is_some() {
        println!("Auth: enabled");
    }
    println!();

    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                let peer = stream
                    .peer_addr()
                    .map(|a| a.to_string())
                    .unwrap_or_default();

                let reader = BufReader::new(&stream);
                let mut lines = reader.lines();

                if let Some(expected_auth) = auth {
                    match lines.next() {
                        Some(Ok(line)) if line == format!("AUTH:{}", expected_auth) => {}
                        _ => {
                            eprintln!("[{}] Auth failed", peer);
                            let _ = stream.write_all(b"ERR:AUTH\n");
                            continue;
                        }
                    }
                }

                if let Some(Ok(message)) = lines.next() {
                    println!("[{}] {}", peer, message);
                    let cmd = cmd_template.replace("{}", &message);
                    run_command(&cmd);
                    let _ = stream.write_all(b"OK\n");
                }
            }
            Err(e) => eprintln!("Connection error: {}", e),
        }
    }
}

fn direct_send(addr: &str, message: &str, auth: Option<&str>) {
    let mut stream = TcpStream::connect(addr).unwrap_or_else(|e| {
        eprintln!("Failed to connect to {}: {}", addr, e);
        std::process::exit(1);
    });

    if let Some(auth_token) = auth {
        writeln!(stream, "AUTH:{}", auth_token).unwrap();
    }

    writeln!(stream, "{}", message).unwrap();

    let mut reader = BufReader::new(&stream);
    let mut response = String::new();
    if reader.read_line(&mut response).is_ok() {
        if response.trim() == "OK" {
            println!("Sent: {}", message);
        } else {
            eprintln!("Error: {}", response.trim());
            std::process::exit(1);
        }
    }
}

fn run_command(cmd: &str) {
    println!("Running: {}", cmd);

    // Use appropriate shell based on OS
    #[cfg(target_os = "windows")]
    let status = Command::new("cmd").arg("/C").arg(cmd).status();

    #[cfg(not(target_os = "windows"))]
    let status = Command::new("sh").arg("-c").arg(cmd).status();

    match status {
        Ok(s) if !s.success() => eprintln!("Command failed: {}", s),
        Err(e) => eprintln!("Failed to run: {}", e),
        _ => {}
    }
}

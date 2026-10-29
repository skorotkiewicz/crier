use crate::args::EndpointArgs;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Preset {
    pub(crate) addr: Option<String>,
    pub(crate) relay: Option<String>,
    pub(crate) port: Option<u16>,
    pub(crate) topic: Option<String>,
    pub(crate) message: Option<String>,
    pub(crate) auth: Option<String>,
}

#[derive(Default, Deserialize)]
pub(crate) struct Config {
    #[serde(flatten)]
    presets: BTreeMap<String, Preset>,
    default_preset: Option<String>,
}

pub(crate) fn config_path(custom: Option<&Path>) -> PathBuf {
    custom.map(Path::to_path_buf).unwrap_or_else(|| {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("crier.yml")
    })
}

impl Config {
    pub(crate) fn load(custom: Option<&Path>) -> io::Result<Self> {
        let path = config_path(custom);
        match fs::read_to_string(&path) {
            Ok(content) => serde_yaml::from_str(&content).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Invalid config {}: {error}", path.display()),
                )
            }),
            Err(error) if custom.is_none() && error.kind() == io::ErrorKind::NotFound => {
                Ok(Self::default())
            }
            Err(error) => Err(io::Error::new(
                error.kind(),
                format!("Cannot read config {}: {error}", path.display()),
            )),
        }
    }

    pub(crate) fn resolve(mut self, args: EndpointArgs) -> io::Result<Preset> {
        // Explicit transport options work independently of the default preset.
        let name = args.preset.as_deref().or_else(|| {
            if args.addr.is_some() || args.relay.is_some() {
                None
            } else {
                self.default_preset
                    .as_deref()
                    .or_else(|| self.presets.contains_key("local").then_some("local"))
            }
        });
        let mut preset = match name {
            Some(name) => self.presets.remove(name).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "Preset '{name}' not found. Available presets: {}",
                        self.presets
                            .keys()
                            .map(String::as_str)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                )
            })?,
            None => Preset::default(),
        };

        if args.addr.is_some() {
            preset.relay = None;
        } else if args.relay.is_some() {
            preset.addr = None;
        }
        preset.addr = args.addr.or(preset.addr);
        preset.relay = args.relay.or(preset.relay);
        preset.port = args.port.or(preset.port);
        preset.topic = args.topic.or(preset.topic);
        preset.message = args.message.or(preset.message);
        preset.auth = args.auth.or(preset.auth);
        Ok(preset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        serde_yaml::from_str(
            "default_preset: remote\nremote:\n  relay: localhost\n  port: 2883\n  topic: builds\n",
        )
        .expect("valid config fixture")
    }

    #[test]
    fn explicit_transport_needs_no_preset() {
        let resolved = Config::default().resolve(EndpointArgs {
            addr: Some("localhost:5555".into()),
            ..EndpointArgs::default()
        });
        assert!(
            resolved.is_ok(),
            "explicit transport must not require a local preset"
        );
    }

    #[test]
    fn explicit_default_port_overrides_preset() {
        let resolved = config()
            .resolve(EndpointArgs {
                preset: Some("remote".into()),
                port: Some(1883),
                ..EndpointArgs::default()
            })
            .expect("preset exists");
        assert_eq!(
            resolved.port,
            Some(1883),
            "an explicit port must always win"
        );
    }

    #[test]
    fn explicit_address_overrides_relay_preset() {
        let resolved = config()
            .resolve(EndpointArgs {
                preset: Some("remote".into()),
                addr: Some("localhost:5555".into()),
                ..EndpointArgs::default()
            })
            .expect("preset exists");
        assert!(
            resolved.relay.is_none(),
            "TCP override must disable the preset relay"
        );
    }

    #[test]
    fn explicit_transport_skips_default_preset() {
        let resolved = config()
            .resolve(EndpointArgs {
                addr: Some("localhost:5555".into()),
                ..EndpointArgs::default()
            })
            .expect("explicit address is valid");
        assert!(
            resolved.auth.is_none() && resolved.relay.is_none(),
            "defaults must not leak into explicit transport options"
        );
    }

    #[test]
    fn default_preset_is_used_without_transport() {
        let resolved = config()
            .resolve(EndpointArgs::default())
            .expect("default preset exists");
        assert_eq!(
            resolved.port,
            Some(2883),
            "configured default preset must be selected"
        );
    }

    #[test]
    fn malformed_config_and_misspelled_options_are_rejected() {
        for source in ["local: [", "local:\n  adress: localhost:5555"] {
            assert!(
                serde_yaml::from_str::<Config>(source).is_err(),
                "invalid configuration must be reported"
            );
        }
    }
}

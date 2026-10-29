//! TCP and MQTT notification command-line application.
#![warn(missing_docs)]

mod application;
mod args;
mod command_template;
mod config;
mod direct;
mod relay;
mod wire;

pub use application::main;

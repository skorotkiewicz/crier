use crate::command_template::CommandTemplate;
use crate::wire::{validate_message, MAX_MESSAGE_BYTES, NETWORK_TIMEOUT};
use rumqttc::{Client, Event, MqttOptions, Packet, QoS, RecvTimeoutError, SubscribeReasonCode};
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn options(broker: &str, port: u16) -> MqttOptions {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let id = format!(
        "crier-{}-{time}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    );
    let mut options = MqttOptions::new(id, broker, port);
    options.set_keep_alive(Duration::from_secs(30));
    // Include space for the auth envelope and MQTT packet headers/topic.
    options.set_max_packet_size(
        MAX_MESSAGE_BYTES + 128 * 1024,
        MAX_MESSAGE_BYTES + 128 * 1024,
    );
    options
}

pub(crate) fn listen(
    broker: &str,
    port: u16,
    topic: &str,
    command: &CommandTemplate,
    auth: Option<&str>,
) -> io::Result<()> {
    let (client, mut connection) = Client::new(options(broker, port), 10);
    for event in connection.iter() {
        match event {
            Ok(Event::Incoming(Packet::ConnAck(_))) => {
                // Clean MQTT sessions lose subscriptions after reconnecting.
                client
                    .subscribe(topic, QoS::AtLeastOnce)
                    .map_err(io::Error::other)?;
            }
            Ok(Event::Incoming(Packet::SubAck(ack))) => {
                if ack.return_codes.contains(&SubscribeReasonCode::Failure) {
                    return Err(io::Error::other("Broker rejected subscription"));
                }
                println!("Listening via {broker}:{port} on {topic}");
            }
            Ok(Event::Incoming(Packet::Publish(publish))) => {
                let result =
                    decode_message(&publish.payload, auth).and_then(|message| command.run(message));
                if let Err(error) = result {
                    eprintln!("Message rejected: {error}");
                }
            }
            Err(error) => {
                eprintln!(
                    "MQTT connection error for {broker}:{port}: {error}; reconnecting in 1 second"
                );
                std::thread::sleep(Duration::from_secs(1));
            }
            _ => {}
        }
    }
    Err(io::Error::other("MQTT connection closed"))
}

fn decode_message<'a>(payload: &'a [u8], auth: Option<&str>) -> io::Result<&'a str> {
    let payload = std::str::from_utf8(payload)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let message = match auth {
        Some(expected) => payload
            .strip_prefix(&format!("AUTH:{expected}:"))
            .ok_or_else(|| io::Error::new(io::ErrorKind::PermissionDenied, "Auth failed"))?,
        None => payload,
    };
    validate_message(message)?;
    Ok(message)
}

pub(crate) fn send(
    broker: &str,
    port: u16,
    topic: &str,
    message: &str,
    auth: Option<&str>,
) -> io::Result<()> {
    validate_message(message)?;
    let payload = match auth {
        Some(token) => format!("AUTH:{token}:{message}"),
        None => message.to_owned(),
    };
    let (client, mut connection) = Client::new(options(broker, port), 10);
    // QoS 1 allows success to mean the broker acknowledged the publication.
    client
        .publish(topic, QoS::AtLeastOnce, false, payload)
        .map_err(io::Error::other)?;
    let deadline = Instant::now() + NETWORK_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Timeout waiting for broker acknowledgement",
            ));
        }
        match connection.recv_timeout(remaining) {
            Ok(Ok(Event::Incoming(Packet::PubAck(_)))) => {
                println!("Sent via {broker}:{port}: {message}");
                return Ok(());
            }
            Ok(Ok(_)) => {}
            Ok(Err(error)) => {
                return Err(io::Error::other(format!(
                    "MQTT connection to {broker}:{port} failed: {error}"
                )))
            }
            Err(RecvTimeoutError::Timeout) => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Timeout waiting for broker acknowledgement",
                ))
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(io::Error::other(
                    "MQTT connection closed before acknowledgement",
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authenticated_messages_preserve_the_entire_body() {
        assert_eq!(
            decode_message(b"AUTH:secret:a:b\nnext", Some("secret")).expect("correct auth"),
            "a:b\nnext",
            "auth parsing must leave message content intact"
        );
        assert!(
            decode_message(b"AUTH:secret-extra:hello", Some("secret")).is_err(),
            "token prefixes must not authenticate"
        );
        assert!(
            decode_message(b"AUTH:secret:\xff", Some("secret")).is_err(),
            "invalid UTF-8 must not be replaced silently"
        );
    }

    #[test]
    fn clients_do_not_share_broker_ids() {
        assert_ne!(
            options("localhost", 1883).client_id(),
            options("localhost", 1883).client_id(),
            "separate clients must not disconnect one another"
        );
    }
}

use crate::command_template::CommandTemplate;
use crate::wire::{read_line, validate_message, MAX_MESSAGE_BYTES, NETWORK_TIMEOUT};
use std::io::{self, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::time::Instant;

// An absolute deadline prevents a peer from holding the listener by slowly
// sending bytes just before each individual socket timeout would expire.
struct DeadlineReader<'a> {
    stream: &'a TcpStream,
    deadline: Instant,
}

impl Read for DeadlineReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "TCP read timed out",
            ));
        }
        self.stream.set_read_timeout(Some(remaining))?;
        self.stream.read(buffer)
    }
}

pub(crate) fn listen(addr: &str, command: &CommandTemplate, auth: Option<&str>) -> io::Result<()> {
    let listener = TcpListener::bind(addr)?;
    println!("Listening on {}", listener.local_addr()?);
    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                if let Err(error) = handle(&mut stream, command, auth) {
                    eprintln!("Connection error: {error}");
                }
            }
            Err(error) => eprintln!("Connection error: {error}"),
        }
    }
    Ok(())
}

fn handle(stream: &mut TcpStream, command: &CommandTemplate, auth: Option<&str>) -> io::Result<()> {
    stream.set_write_timeout(Some(NETWORK_TIMEOUT))?;
    let mut reader = BufReader::new(DeadlineReader {
        stream,
        deadline: Instant::now() + NETWORK_TIMEOUT,
    });
    if let Some(expected) = auth {
        let line = read_line(&mut reader, 1029)?;
        if line.strip_prefix("AUTH:") != Some(expected) {
            stream.write_all(b"ERR:AUTH\n")?;
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Auth failed",
            ));
        }
    }
    let message = read_line(&mut reader, MAX_MESSAGE_BYTES)?;
    validate_message(&message)?;
    match command.run(&message) {
        Ok(()) => stream.write_all(b"OK\n"),
        Err(error) => {
            stream.write_all(b"ERR:COMMAND\n")?;
            Err(error)
        }
    }
}

pub(crate) fn send(addr: &str, message: &str, auth: Option<&str>) -> io::Result<()> {
    validate_message(message)?;
    if message.contains(['\r', '\n']) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "TCP messages must not contain newlines",
        ));
    }
    let deadline = Instant::now() + NETWORK_TIMEOUT;
    let mut last_error = io::Error::new(
        io::ErrorKind::InvalidInput,
        "Address resolved to no targets",
    );
    let mut connected = None;
    for target in addr.to_socket_addrs()? {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "TCP connect timed out",
            ));
        }
        match TcpStream::connect_timeout(&target, remaining) {
            Ok(stream) => {
                connected = Some(stream);
                break;
            }
            Err(error) => last_error = error,
        }
    }
    let mut stream = connected.ok_or(last_error)?;
    stream.set_write_timeout(Some(NETWORK_TIMEOUT))?;
    if let Some(token) = auth {
        writeln!(stream, "AUTH:{token}")?;
    }
    writeln!(stream, "{message}")?;
    let response = read_line(
        &mut BufReader::new(DeadlineReader {
            stream: &stream,
            deadline: Instant::now() + NETWORK_TIMEOUT,
        }),
        128,
    )?;
    if response != "OK" {
        return Err(io::Error::other(format!(
            "Listener rejected message: {response}"
        )));
    }
    println!("Sent: {message}");
    Ok(())
}

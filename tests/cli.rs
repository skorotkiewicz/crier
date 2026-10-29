use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(10);

struct Fixture {
    directory: PathBuf,
}

impl Fixture {
    fn new(config: &str) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "crier-test-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).expect("create isolated fixture directory");
        fs::write(directory.join("config.yml"), config).expect("write config fixture");
        Self { directory }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_crier"));
        command
            .arg("--config")
            .arg(self.directory.join("config.yml"));
        command
    }

    fn spawn(&self, args: &[&str]) -> Process {
        Process(
            self.command()
                .args(args)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("spawn CLI"),
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

struct Process(Child);

impl Process {
    fn finish(&mut self) -> Output {
        let deadline = Instant::now() + TIMEOUT;
        let status = loop {
            if let Some(status) = self.0.try_wait().expect("poll CLI") {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "CLI must exit within the test deadline"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        self.0
            .stdout
            .take()
            .expect("captured stdout")
            .read_to_end(&mut stdout)
            .expect("read stdout");
        self.0
            .stderr
            .take()
            .expect("captured stderr")
            .read_to_end(&mut stderr)
            .expect("read stderr");
        Output {
            status,
            stdout,
            stderr,
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn peer(listener: &TcpListener) -> TcpStream {
    listener
        .set_nonblocking(true)
        .expect("use bounded accept loop");
    let deadline = Instant::now() + TIMEOUT;
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream
                    .set_read_timeout(Some(TIMEOUT))
                    .expect("set test peer timeout");
                stream
                    .set_write_timeout(Some(TIMEOUT))
                    .expect("set test write timeout");
                return stream;
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                assert!(
                    Instant::now() < deadline,
                    "CLI must connect within the test deadline"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("test accept failed: {error}"),
        }
    }
}

#[test]
fn config_errors_are_reported_and_explicit_options_need_no_preset() {
    let fixture = Fixture::new("{}");
    let output = fixture
        .spawn(&["send", "invalid-address", "-m", "hello"])
        .finish();
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "invalid TCP address must fail");
    assert!(
        !error.contains("Preset"),
        "explicit options must not require a preset: {error}"
    );

    fs::write(fixture.directory.join("config.yml"), "local: [").expect("write malformed config");
    let output = fixture
        .spawn(&["send", "-p", "local", "-m", "hello"])
        .finish();
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Invalid config"),
        "parse errors must be reported"
    );

    fs::remove_file(fixture.directory.join("config.yml")).expect("remove config fixture");
    let output = fixture
        .spawn(&["send", "invalid-address", "-m", "hello"])
        .finish();
    assert!(
        !output.status.success(),
        "an explicitly selected missing config must fail"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Cannot read config"),
        "missing custom config must be reported"
    );
}

#[test]
fn tcp_sender_requires_a_complete_positive_acknowledgement() {
    for response in [b"OK\n".as_slice(), b"ERR:AUTH\n", b"", b"OK"] {
        let fixture = Fixture::new("{}");
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test receiver");
        let addr = listener.local_addr().expect("receiver address").to_string();
        let mut sender = fixture.spawn(&["send", &addr, "-m", "hello", "--auth", "secret"]);
        let mut stream = peer(&listener);
        let mut reader = BufReader::new(&stream);
        for expected in ["AUTH:secret\n", "hello\n"] {
            let mut line = String::new();
            reader.read_line(&mut line).expect("read sender frame");
            assert_eq!(
                line, expected,
                "sender must preserve auth and message frames"
            );
        }
        stream.write_all(response).expect("send acknowledgement");
        drop(stream);
        let output = sender.finish();
        assert_eq!(
            output.status.success(),
            response == b"OK\n",
            "only a complete OK response may succeed: {output:?}"
        );
    }
}

#[test]
fn tcp_sender_rejects_multiline_messages_before_connecting() {
    let fixture = Fixture::new("{}");
    let output = fixture
        .spawn(&["send", "127.0.0.1:1", "-m", "one\ntwo"])
        .finish();
    assert!(
        !output.status.success(),
        "TCP cannot silently truncate multiline messages"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("must not contain newlines"),
        "validation must happen before connecting"
    );
}

// The listener invokes this test executable as a portable notification handler.
#[test]
fn notification_handler() {
    if let Some(path) = std::env::var_os("CRIER_TEST_OUTPUT") {
        if std::env::var("CRIER_MESSAGE").as_deref() == Ok("__crier_test_fail__") {
            std::process::exit(7);
        }
        fs::write(
            path,
            std::env::var("CRIER_MESSAGE").expect("message environment variable"),
        )
        .expect("record handler message");
    }
}

#[test]
fn tcp_listener_authenticates_and_runs_handlers() {
    let fixture = Fixture::new("{}");
    let output_path = fixture.directory.join("message.txt");
    let handler = shell_words::join([
        std::env::current_exe()
            .expect("test executable")
            .to_str()
            .expect("UTF-8 test executable"),
        "--exact",
        "notification_handler",
        "--nocapture",
    ]);
    let mut listener = Process(
        fixture
            .command()
            .args(["listen", "127.0.0.1:0", "--auth", "secret", "-m", &handler])
            .env("CRIER_TEST_OUTPUT", &output_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn listener"),
    );
    let mut line = String::new();
    let mut listener_output = BufReader::new(listener.0.stdout.take().expect("listener output"));
    listener_output
        .read_line(&mut line)
        .expect("listener startup");
    let addr = line
        .trim()
        .strip_prefix("Listening on ")
        .expect("listener reports its address");
    let mut rejected = fixture.spawn(&["send", addr, "--auth", "wrong", "-m", "ignored"]);
    assert!(
        !rejected.finish().status.success(),
        "wrong authentication must fail"
    );
    assert!(
        !output_path.exists(),
        "unauthenticated messages must not invoke handlers"
    );
    let mut stalled = TcpStream::connect(addr).expect("connect stalled peer");
    stalled
        .set_read_timeout(Some(Duration::from_secs(7)))
        .expect("bound stalled peer test");
    stalled
        .write_all(b"AUTH:")
        .expect("send incomplete auth frame");
    let started = Instant::now();
    let mut byte = [0];
    assert_eq!(
        stalled
            .read(&mut byte)
            .expect("listener must close stalled peer"),
        0,
        "incomplete frames must time out"
    );
    assert!(
        started.elapsed() < Duration::from_secs(7),
        "stalled peers must not hold the listener forever"
    );
    let failed = fixture
        .spawn(&[
            "send",
            addr,
            "--auth",
            "secret",
            "-m",
            "__crier_test_fail__",
        ])
        .finish();
    assert!(
        !failed.status.success(),
        "failed handler must not receive a positive acknowledgement"
    );
    assert!(
        String::from_utf8_lossy(&failed.stderr).contains("ERR:COMMAND"),
        "sender must receive the command failure"
    );
    let message = "quotes \" ' $(touch marker) ; & %PATH%";
    let output = fixture
        .spawn(&["send", addr, "--auth", "secret", "-m", message])
        .finish();
    assert!(
        output.status.success(),
        "authenticated message must succeed: {output:?}"
    );
    assert_eq!(
        fs::read_to_string(output_path).expect("handler output"),
        message,
        "handler must receive the original text"
    );
}

fn packet(stream: &mut TcpStream) -> (u8, Vec<u8>) {
    let mut header = [0];
    stream.read_exact(&mut header).expect("MQTT fixed header");
    let mut length = 0;
    let mut multiplier = 1;
    loop {
        let mut byte = [0];
        stream.read_exact(&mut byte).expect("MQTT packet length");
        length += usize::from(byte[0] & 127) * multiplier;
        if byte[0] & 128 == 0 {
            break;
        }
        multiplier *= 128;
        assert!(
            multiplier <= 128 * 128 * 128,
            "MQTT length must fit four bytes"
        );
    }
    assert!(length < 256 * 1024, "test MQTT packet must remain bounded");
    let mut body = vec![0; length];
    stream.read_exact(&mut body).expect("MQTT packet body");
    (header[0], body)
}

#[test]
fn mqtt_sender_waits_for_the_broker_acknowledgement() {
    for acknowledge in [true, false] {
        let fixture = Fixture::new("{}");
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake broker");
        let port = listener
            .local_addr()
            .expect("broker address")
            .port()
            .to_string();
        let mut sender = fixture.spawn(&[
            "send",
            "--relay",
            "127.0.0.1",
            "--port",
            &port,
            "-t",
            "builds",
            "-m",
            "hello",
            "--auth",
            "secret",
        ]);
        let mut stream = peer(&listener);
        assert_eq!(packet(&mut stream).0, 0x10, "client must send CONNECT");
        stream.write_all(&[0x20, 2, 0, 0]).expect("send CONNACK");
        let (header, publish) = packet(&mut stream);
        assert_eq!(header, 0x32, "publication must use QoS 1");
        let topic_len = usize::from(u16::from_be_bytes([publish[0], publish[1]]));
        let id_offset = 2 + topic_len;
        assert_eq!(&publish[2..id_offset], b"builds", "publish correct topic");
        assert_eq!(
            &publish[id_offset + 2..],
            b"AUTH:secret:hello",
            "publish authenticated message intact"
        );
        if acknowledge {
            stream
                .write_all(&[0x40, 2, publish[id_offset], publish[id_offset + 1]])
                .expect("send PUBACK");
        }
        let started = Instant::now();
        let output = sender.finish();
        assert_eq!(
            output.status.success(),
            acknowledge,
            "success requires a broker acknowledgement: {output:?}"
        );
        if !acknowledge {
            assert!(
                started.elapsed() < Duration::from_secs(8),
                "silent broker must hit the actual five second deadline"
            );
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("Timeout"),
                "silent broker must report timeout"
            );
        }
    }
}

#[test]
fn mqtt_listener_resubscribes_after_reconnecting() {
    let fixture = Fixture::new("{}");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake broker");
    let port = listener
        .local_addr()
        .expect("broker address")
        .port()
        .to_string();
    let _process = fixture.spawn(&[
        "listen",
        "--relay",
        "127.0.0.1",
        "--port",
        &port,
        "-t",
        "builds",
        "-m",
        "unused-handler",
    ]);
    for _ in 0..2 {
        let mut stream = peer(&listener);
        assert_eq!(
            packet(&mut stream).0,
            0x10,
            "listener must CONNECT on each session"
        );
        stream.write_all(&[0x20, 2, 0, 0]).expect("send CONNACK");
        let (header, subscribe) = packet(&mut stream);
        assert_eq!(
            header, 0x82,
            "listener must SUBSCRIBE on every fresh session"
        );
        assert_eq!(
            &subscribe[4..subscribe.len() - 1],
            b"builds",
            "listener must restore the topic subscription"
        );
        stream
            .write_all(&[0x90, 3, subscribe[0], subscribe[1], 1])
            .expect("send SUBACK");
    }
}

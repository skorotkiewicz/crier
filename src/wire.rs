use std::io::{self, BufRead};

pub(crate) const MAX_MESSAGE_BYTES: usize = 64 * 1024;
pub(crate) const NETWORK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

pub(crate) fn validate_auth(auth: Option<&str>) -> io::Result<()> {
    if let Some(token) = auth {
        if token.is_empty() || token.contains([':', '\r', '\n', '\0']) || token.len() > 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Auth token must be 1 to 1024 bytes and contain no colon, newline, or NUL",
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_message(message: &str) -> io::Result<()> {
    if message.len() > MAX_MESSAGE_BYTES || message.contains('\0') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Message must be at most 65536 bytes and contain no NUL",
        ));
    }
    Ok(())
}

pub(crate) fn read_line(reader: &mut impl BufRead, limit: usize) -> io::Result<String> {
    let mut bytes = Vec::new();
    std::io::Read::take(reader, (limit + 2) as u64).read_until(b'\n', &mut bytes)?;
    if bytes.last() != Some(&b'\n') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Missing newline or oversized frame",
        ));
    }
    bytes.pop();
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
    if bytes.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Frame exceeds size limit",
        ));
    }
    String::from_utf8(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn frames_are_bounded_and_require_terminators() {
        for bytes in [b"unterminated".as_slice(), b"12345\n", b"\xff\n"] {
            assert!(
                read_line(&mut Cursor::new(bytes), 4).is_err(),
                "oversized, incomplete, or non-UTF-8 frames must fail"
            );
        }
        assert_eq!(
            read_line(&mut Cursor::new(b"1234\r\n"), 4).expect("valid CRLF frame"),
            "1234",
            "CRLF must be accepted at the size limit"
        );
        assert_eq!(
            read_line(&mut Cursor::new(b"\n"), 4).expect("empty frame is valid"),
            "",
            "empty notifications must be preserved"
        );
    }

    #[test]
    fn auth_cannot_inject_protocol_fields() {
        for token in ["", "a:b", "a\nb", "a\rb", "a\0b"] {
            assert!(
                validate_auth(Some(token)).is_err(),
                "ambiguous auth tokens must be rejected"
            );
        }
    }
}

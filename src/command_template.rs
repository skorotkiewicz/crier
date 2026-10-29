use std::io;
use std::process::Command;

pub(crate) struct CommandTemplate {
    program: String,
    arguments: Vec<String>,
}

impl CommandTemplate {
    pub(crate) fn parse(template: &str) -> io::Result<Self> {
        let mut words = shell_words::split(template)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?
            .into_iter();
        let program = words
            .next()
            .filter(|word| !word.is_empty())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "Command must not be empty")
            })?;
        if program.contains("{}") || template.contains('\0') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Command must use a fixed executable and contain no NUL bytes",
            ));
        }
        Ok(Self {
            program,
            arguments: words.collect(),
        })
    }

    fn command(&self, message: &str) -> Command {
        let mut command = Command::new(&self.program);
        command.args(
            self.arguments
                .iter()
                .map(|argument| argument.replace("{}", message)),
        );
        command.env("CRIER_MESSAGE", message);
        command
    }

    pub(crate) fn run(&self, message: &str) -> io::Result<()> {
        let status = self.command(message).status()?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!("Command exited with {status}")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders_preserve_message_as_one_literal_argument() {
        let template = CommandTemplate::parse("notify-send 'Build status' \"{}\" 'prefix:{}'")
            .expect("valid command");
        let message = "\"; touch marker; $(touch marker) `touch marker` & %PATH%\nnext";
        let command = template.command(message);
        let arguments: Vec<_> = command.get_args().collect();
        assert_eq!(
            arguments,
            ["Build status", message, &format!("prefix:{message}")],
            "message content must never change argument boundaries"
        );
    }

    #[cfg(unix)]
    #[test]
    fn message_text_cannot_execute_shell_commands() {
        let marker = std::env::temp_dir().join(format!("crier-injection-{}", std::process::id()));
        let message = format!("$(touch {})", marker.display());
        let output = CommandTemplate::parse("printf '%s' \"{}\"")
            .expect("valid command")
            .command(&message)
            .output()
            .expect("printf is available");
        assert!(output.status.success(), "printf must complete");
        assert_eq!(
            output.stdout,
            message.as_bytes(),
            "shell expressions must be printed literally"
        );
        assert!(
            !marker.exists(),
            "message text must not execute shell commands"
        );
    }

    #[test]
    fn invalid_templates_are_rejected_before_listening() {
        for template in ["", "''", "echo 'unfinished", "{} argument", "echo \0"] {
            assert!(
                CommandTemplate::parse(template).is_err(),
                "invalid command must be rejected"
            );
        }
    }
}

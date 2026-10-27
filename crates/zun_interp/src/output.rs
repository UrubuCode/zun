use super::Output;

/// An [`Output`] that collects lines in a vector, for tests.
#[derive(Debug, Default)]
pub struct CaptureOutput {
    /// Lines written to the standard stream so far (`println` and raw `write`).
    pub lines: Vec<String>,
    /// Lines written to the error stream (`console.error`/`warn`) so far.
    pub errors: Vec<String>,
    /// Queued stdin lines handed out by `read_line`, oldest first.
    pub input: std::collections::VecDeque<String>,
    /// The last exit code requested (`exitProcess`), if any.
    pub exit_code: Option<i32>,
}

impl Output for CaptureOutput {
    fn println(&mut self, line: &str) {
        self.lines.push(line.to_string());
    }

    fn eprintln(&mut self, line: &str) {
        self.errors.push(line.to_string());
    }

    fn read_line(&mut self) -> Option<String> {
        self.input.pop_front()
    }

    fn write_out(&mut self, text: &str) {
        self.lines.push(text.to_string());
    }

    fn write_err(&mut self, text: &str) {
        self.errors.push(text.to_string());
    }

    fn exit(&mut self, code: i32) {
        self.exit_code = Some(code);
    }
}

/// An [`Output`] that writes to the real stdout / stderr streams and the real
/// process host (stdin, exit).
#[derive(Debug, Default)]
pub struct StdoutOutput;

impl Output for StdoutOutput {
    fn println(&mut self, line: &str) {
        println!("{line}");
    }

    fn eprintln(&mut self, line: &str) {
        eprintln!("{line}");
    }

    fn read_line(&mut self) -> Option<String> {
        use std::io::BufRead;
        let mut line = String::new();
        match std::io::stdin().lock().read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => {
                if line.ends_with('\n') {
                    line.pop();
                    if line.ends_with('\r') {
                        line.pop();
                    }
                }
                Some(line)
            }
        }
    }

    fn write_out(&mut self, text: &str) {
        use std::io::Write;
        let mut out = std::io::stdout();
        let _ = write!(out, "{text}");
        let _ = out.flush();
    }

    fn write_err(&mut self, text: &str) {
        use std::io::Write;
        let mut err = std::io::stderr();
        let _ = write!(err, "{text}");
        let _ = err.flush();
    }

    fn exit(&mut self, code: i32) {
        std::process::exit(code);
    }
}

use super::Output;

/// An [`Output`] that collects lines in a vector, for tests.
#[derive(Debug, Default)]
pub struct CaptureOutput {
    /// Lines written to the standard stream so far.
    pub lines: Vec<String>,
    /// Lines written to the error stream (`console.error`/`warn`) so far.
    pub errors: Vec<String>,
}

impl Output for CaptureOutput {
    fn println(&mut self, line: &str) {
        self.lines.push(line.to_string());
    }

    fn eprintln(&mut self, line: &str) {
        self.errors.push(line.to_string());
    }
}

/// An [`Output`] that writes to the real stdout / stderr streams.
#[derive(Debug, Default)]
pub struct StdoutOutput;

impl Output for StdoutOutput {
    fn println(&mut self, line: &str) {
        println!("{line}");
    }

    fn eprintln(&mut self, line: &str) {
        eprintln!("{line}");
    }
}

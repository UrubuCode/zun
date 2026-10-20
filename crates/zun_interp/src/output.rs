use super::Output;

/// An [`Output`] that collects lines in a vector, for tests.
#[derive(Debug, Default)]
pub struct CaptureOutput {
    /// Lines written so far.
    pub lines: Vec<String>,
}

impl Output for CaptureOutput {
    fn println(&mut self, line: &str) {
        self.lines.push(line.to_string());
    }
}

/// An [`Output`] that writes to real stdout.
#[derive(Debug, Default)]
pub struct StdoutOutput;

impl Output for StdoutOutput {
    fn println(&mut self, line: &str) {
        println!("{line}");
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorValue {
    name: String,
    message: String,
}

impl ErrorValue {
    pub fn new(name: impl Into<String>, message: impl Into<String>) -> Self {
        Self { name: name.into(), message: message.into() }
    }
    pub fn name(&self) -> &str { &self.name }
    pub fn message(&self) -> &str { &self.message }
    pub fn set_name(&mut self, name: impl Into<String>) { self.name = name.into(); }
    pub fn set_message(&mut self, message: impl Into<String>) { self.message = message.into(); }
}

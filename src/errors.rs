use std::fmt;

pub const EXIT_SUCCESS: u8 = 0;
pub const EXIT_GENERAL: u8 = 1;
pub const EXIT_CONFIG: u8 = 2;
pub const EXIT_SOURCE: u8 = 3;
pub const EXIT_MOUNT: u8 = 4;
pub const EXIT_UNHEALTHY: u8 = 5;

#[derive(Debug)]
pub struct AppError {
    pub code: u8,
    pub message: String,
}

impl AppError {
    pub fn new(code: u8, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn general(message: impl Into<String>) -> Self {
        Self::new(EXIT_GENERAL, message)
    }

    pub fn config(message: impl Into<String>) -> Self {
        Self::new(EXIT_CONFIG, message)
    }

    pub fn source(message: impl Into<String>) -> Self {
        Self::new(EXIT_SOURCE, message)
    }

    pub fn mount(message: impl Into<String>) -> Self {
        Self::new(EXIT_MOUNT, message)
    }

    pub fn unhealthy(message: impl Into<String>) -> Self {
        Self::new(EXIT_UNHEALTHY, message)
    }

    pub fn io(code: u8, context: &str, error: std::io::Error) -> Self {
        Self::new(code, format!("{context}: {error}"))
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AppError {}

pub type Result<T> = std::result::Result<T, AppError>;

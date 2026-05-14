#[derive(Debug)]
pub enum AppError {
    Nokhawa(nokhwa::NokhwaError),
    Other(String),
}

impl From<nokhwa::NokhwaError> for AppError {
    fn from(value: nokhwa::NokhwaError) -> Self {
        AppError::Nokhawa(value)
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::Nokhawa(e) => write!(f, "Camera Error: {}", e),
            AppError::Other(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for AppError {}

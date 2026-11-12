#[derive(Debug)]
pub enum EvalError {
    /// The source could not be turned into a V8 string.
    InvalidSource,
    /// Compilation or execution threw; contains the exception text.
    Exception(String),
    /// A binding or value conversion failed.
    Binding(String),
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvalError::InvalidSource => write!(fmt, "invalid script source"),
            EvalError::Exception(msg) | EvalError::Binding(msg) => write!(fmt, "{msg}"),
        }
    }
}

impl std::error::Error for EvalError {}

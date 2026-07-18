use crate::types::AppRef;

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Errors surfaced to the UI.
///
/// Variants are chosen by *what the user can do about it*, not by where they
/// came from — the UI switches on these to decide between a retry button, an
/// auth prompt, and a plain error toast.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The backend isn't usable on this system (daemon missing, not installed).
    /// The UI should hide the backend rather than show an error.
    #[error("{backend} is unavailable: {reason}")]
    BackendUnavailable { backend: &'static str, reason: String },

    #[error("no such app: {0}")]
    NotFound(AppRef),

    /// Needs elevated privileges and the user declined or the prompt failed.
    #[error("authorization was declined")]
    NotAuthorized,

    #[error("network error: {0}")]
    Network(String),

    /// Not enough disk space. Both figures in bytes, for a useful message.
    #[error("need {needed} bytes but only {available} are free")]
    OutOfSpace { needed: u64, available: u64 },

    /// The transaction was cancelled by the user. Not worth showing an error.
    #[error("cancelled")]
    Cancelled,

    /// Backend refused the operation for its own reasons — the string is
    /// already user-facing and comes straight from the package system.
    #[error("{0}")]
    Backend(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl Error {
    /// Whether offering a retry button makes sense.
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Network(_) | Self::Io(_))
    }

    /// Cancellation is a normal outcome; don't surface it as a failure.
    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }
}

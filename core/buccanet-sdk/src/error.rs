use buccanet_protocol::api::codes;

#[derive(Debug, thiserror::Error)]
pub enum SdkError {
    #[error("relay answered {status} {code}: {message}")]
    Relay { status: u16, code: String, message: String },
    #[error("the relay could not be reached: {0}")]
    Network(String),
    #[error("MLS failure: {0}")]
    Mls(String),
    #[error("unknown room {0}")]
    UnknownRoom(String),
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("stored state is unreadable: {0}")]
    State(String),
    /// An agent's message holds something shaped like a key, by the gitleaks rule named.
    #[error("an agent cannot send what looks like a secret ({0}); secrets never go into a room")]
    SecretShaped(String),
}

impl SdkError {
    pub fn is_stale_epoch(&self) -> bool {
        self.relay_code() == Some(codes::STALE_EPOCH)
    }

    /// The stable code of a relay refusal, such as `room_full` or `agent_burst`.
    pub fn relay_code(&self) -> Option<&str> {
        match self {
            SdkError::Relay { code, .. } => Some(code),
            _ => None,
        }
    }

    pub fn mls(error: impl std::fmt::Debug) -> Self {
        SdkError::Mls(format!("{error:?}"))
    }
}

impl From<reqwest::Error> for SdkError {
    fn from(error: reqwest::Error) -> Self {
        SdkError::Network(error.to_string())
    }
}

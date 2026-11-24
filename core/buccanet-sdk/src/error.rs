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
}

impl SdkError {
    pub fn is_stale_epoch(&self) -> bool {
        matches!(self, SdkError::Relay { code, .. } if code == codes::STALE_EPOCH)
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

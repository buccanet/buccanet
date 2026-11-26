//! Keeps secrets out of what an agent sends: a message is checked before it is encrypted,
//! against the secret values the agent's host knows and the key formats gitleaks knows. The SDK
//! runs the key format check itself on every text an agent device sends.

mod values;

pub use buccanet_sdk::secrets::key_shaped;
pub use values::contains_any;

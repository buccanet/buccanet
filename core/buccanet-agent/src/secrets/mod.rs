//! Keeps secrets out of what an agent sends: a message is checked before it is encrypted,
//! against the secret values the agent's host knows and the key formats gitleaks knows.

mod patterns;
mod values;

pub use patterns::find as key_shaped;
pub use values::contains_any;

//! Buccanet protocol v1.
//!
//! Everything a client and a relay must agree on: the HTTP bodies, the
//! inbox events a device receives, the plaintext envelope carried inside MLS
//! application messages, and the MLS framing fields a relay reads without keys.

pub mod api;
pub mod b64;
pub mod envelope;
pub mod events;
pub mod framing;

/// The only MLS ciphersuite Buccanet v1 accepts.
pub const CIPHERSUITE: openmls::prelude::Ciphersuite =
    openmls::prelude::Ciphersuite::MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519;

/// Path prefix of every relay route.
pub const API_PREFIX: &str = "/api/v1";

//! The MLS fields a relay reads without any key: group, epoch and content type.

use openmls::prelude::{ContentType, MlsMessageBodyIn, MlsMessageIn, ProtocolMessage};
use tls_codec::Deserialize;

use crate::events::MessageKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Framing {
    pub group_id: Vec<u8>,
    pub epoch: u64,
    pub kind: MessageKind,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FramingError {
    #[error("bytes are not an MLS message")]
    Malformed,
    #[error("only private or public handshake and application messages are accepted here")]
    NotProtocolMessage,
    #[error("proposals by reference are not accepted; send commits with inline proposals")]
    Proposal,
}

pub fn read_framing(bytes: &[u8]) -> Result<Framing, FramingError> {
    let message = MlsMessageIn::tls_deserialize_exact(bytes).map_err(|_| FramingError::Malformed)?;
    let protocol: ProtocolMessage = match message.extract() {
        MlsMessageBodyIn::PrivateMessage(private) => private.into(),
        MlsMessageBodyIn::PublicMessage(public) => public.into(),
        _ => return Err(FramingError::NotProtocolMessage),
    };
    let kind = match protocol.content_type() {
        ContentType::Application => MessageKind::Application,
        ContentType::Commit => MessageKind::Commit,
        ContentType::Proposal => return Err(FramingError::Proposal),
    };
    Ok(Framing { group_id: protocol.group_id().as_slice().to_vec(), epoch: protocol.epoch().as_u64(), kind })
}

/// True when the bytes are an MLS Welcome.
pub fn is_welcome(bytes: &[u8]) -> bool {
    MlsMessageIn::tls_deserialize_exact(bytes)
        .map(|message| matches!(message.extract(), MlsMessageBodyIn::Welcome(_)))
        .unwrap_or(false)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPackageOwner {
    /// The MLS credential identity: the Buccanet device id, as UTF-8.
    pub identity: String,
    pub signature_key: Vec<u8>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum KeyPackageError {
    #[error("bytes are not a valid key package")]
    Invalid,
    #[error("the key package uses another ciphersuite")]
    Ciphersuite,
    #[error("the key package credential is not a basic UTF-8 identity")]
    Credential,
}

/// Validates a key package signature and returns who it belongs to.
pub fn key_package_owner(bytes: &[u8]) -> Result<KeyPackageOwner, KeyPackageError> {
    use openmls::prelude::{BasicCredential, KeyPackageIn, ProtocolVersion};
    use openmls_rust_crypto::RustCrypto;

    let package_in = KeyPackageIn::tls_deserialize_exact(bytes).map_err(|_| KeyPackageError::Invalid)?;
    let package = package_in
        .validate(&RustCrypto::default(), ProtocolVersion::Mls10)
        .map_err(|_| KeyPackageError::Invalid)?;
    if package.ciphersuite() != crate::CIPHERSUITE {
        return Err(KeyPackageError::Ciphersuite);
    }
    let leaf = package.leaf_node();
    let credential = BasicCredential::try_from(leaf.credential().clone()).map_err(|_| KeyPackageError::Credential)?;
    let identity = String::from_utf8(credential.identity().to_vec()).map_err(|_| KeyPackageError::Credential)?;
    Ok(KeyPackageOwner { identity, signature_key: leaf.signature_key().as_slice().to_vec() })
}

#[cfg(test)]
mod tests {
    use openmls::prelude::*;
    use openmls_basic_credential::SignatureKeyPair;
    use openmls_rust_crypto::OpenMlsRustCrypto;
    use openmls_traits::OpenMlsProvider;
    use tls_codec::Serialize;

    use super::*;
    use crate::CIPHERSUITE;

    fn member(name: &str, provider: &OpenMlsRustCrypto) -> (CredentialWithKey, SignatureKeyPair) {
        let signer = SignatureKeyPair::new(CIPHERSUITE.signature_algorithm()).unwrap();
        signer.store(provider.storage()).unwrap();
        let credential = BasicCredential::new(name.as_bytes().to_vec());
        (CredentialWithKey { credential: credential.into(), signature_key: signer.public().into() }, signer)
    }

    #[test]
    fn relay_reads_owner_epoch_and_kind_without_keys() {
        let (alice_p, bob_p) = (OpenMlsRustCrypto::default(), OpenMlsRustCrypto::default());
        let (alice, alice_signer) = member("device-a", &alice_p);
        let (bob, bob_signer) = member("device-b", &bob_p);
        let package = KeyPackage::builder().build(CIPHERSUITE, &bob_p, &bob_signer, bob).unwrap();
        let package_bytes = package.key_package().tls_serialize_detached().unwrap();
        let owner = key_package_owner(&package_bytes).unwrap();
        assert_eq!(owner.identity, "device-b");
        assert_eq!(owner.signature_key, bob_signer.public());

        let config = MlsGroupCreateConfig::builder().ciphersuite(CIPHERSUITE).use_ratchet_tree_extension(true).build();
        let mut group = MlsGroup::new(&alice_p, &alice_signer, &config, alice).unwrap();
        let (commit, welcome, _) = group.add_members(&alice_p, &alice_signer, &[package.key_package().clone()]).unwrap();
        let commit = commit.tls_serialize_detached().unwrap();
        let framing = read_framing(&commit).unwrap();
        assert_eq!((framing.epoch, framing.kind), (0, MessageKind::Commit));
        assert_eq!(framing.group_id, group.group_id().as_slice());
        assert!(is_welcome(&welcome.tls_serialize_detached().unwrap()));
        assert!(!is_welcome(&commit));

        group.merge_pending_commit(&alice_p).unwrap();
        let text = group.create_message(&alice_p, &alice_signer, b"sealed").unwrap().tls_serialize_detached().unwrap();
        assert_eq!(read_framing(&text).unwrap().kind, MessageKind::Application);
        assert_eq!(read_framing(&text).unwrap().epoch, 1);
        assert_eq!(read_framing(b"not mls"), Err(FramingError::Malformed));
    }
}

//! MLS through OpenMLS: one engine per device, its storage exportable as one blob.

use std::collections::HashMap;

use buccanet_protocol::CIPHERSUITE;
use openmls::prelude::tls_codec::{Deserialize as _, Serialize as _};
use openmls::prelude::*;
use openmls_basic_credential::SignatureKeyPair;
use openmls_rust_crypto::OpenMlsRustCrypto;
use openmls_traits::OpenMlsProvider;

use crate::error::SdkError;

pub struct MlsEngine {
    provider: OpenMlsRustCrypto,
    signer: SignatureKeyPair,
    credential: CredentialWithKey,
}

/// A member of a group as the MLS tree knows it: the device id from its credential and the
/// signing key that authenticates everything it sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Leaf {
    pub device_id: String,
    pub signature_key: Vec<u8>,
}

/// A commit checked but not yet applied: the caller decides from its committer and its
/// additions, then merges it with [`MlsEngine::merge_commit`].
pub struct ReviewedCommit {
    pub committer: String,
    pub added: Vec<Leaf>,
    pub self_removed: bool,
    staged: Box<StagedCommit>,
}

pub enum Incoming {
    Application { sender: String, plaintext: Vec<u8> },
    Commit(ReviewedCommit),
    Ignored,
}

pub struct Joined {
    pub group_id: Vec<u8>,
    /// The member who sent the Welcome: the admin who approved this device.
    pub welcomer: String,
    pub members: Vec<Leaf>,
}

/// What an admin expects of a key package before adding it.
pub struct ExpectedJoiner<'a> {
    pub device_id: &'a str,
    /// The signing key this device was first seen with, if it was seen before.
    pub pinned_key: Option<&'a [u8]>,
}

pub type StorageSnapshot = Vec<(Vec<u8>, Vec<u8>)>;

fn credential_for(identity: &str, signer: &SignatureKeyPair) -> CredentialWithKey {
    let credential = BasicCredential::new(identity.as_bytes().to_vec());
    CredentialWithKey { credential: credential.into(), signature_key: signer.public().into() }
}

fn identity_of(credential: &Credential) -> String {
    String::from_utf8_lossy(credential.serialized_content()).into_owned()
}

fn leaf_of(node: &LeafNode) -> Leaf {
    Leaf { device_id: identity_of(node.credential()), signature_key: node.signature_key().as_slice().to_vec() }
}

fn decode_message(bytes: &[u8]) -> Result<MlsMessageIn, SdkError> {
    MlsMessageIn::tls_deserialize_exact(bytes).map_err(SdkError::mls)
}

fn serialize(message: &MlsMessageOut) -> Result<Vec<u8>, SdkError> {
    message.tls_serialize_detached().map_err(SdkError::mls)
}

impl MlsEngine {
    /// A fresh signing key; the identity is attached once the relay assigned a device id.
    pub fn new_signer() -> Result<SignatureKeyPair, SdkError> {
        SignatureKeyPair::new(CIPHERSUITE.signature_algorithm()).map_err(SdkError::mls)
    }

    pub fn new(identity: &str, signer: SignatureKeyPair) -> Result<Self, SdkError> {
        let provider = OpenMlsRustCrypto::default();
        signer.store(provider.storage()).map_err(SdkError::mls)?;
        let credential = credential_for(identity, &signer);
        Ok(Self { provider, signer, credential })
    }

    pub fn restore(identity: &str, public_key: &[u8], snapshot: StorageSnapshot) -> Result<Self, SdkError> {
        let provider = OpenMlsRustCrypto::default();
        provider.storage().values.write().map_err(SdkError::mls)?.extend(snapshot);
        let signer = SignatureKeyPair::read(provider.storage(), public_key, CIPHERSUITE.signature_algorithm())
            .ok_or_else(|| SdkError::State("signing key missing from storage".into()))?;
        let credential = credential_for(identity, &signer);
        Ok(Self { provider, signer, credential })
    }

    pub fn snapshot(&self) -> Result<StorageSnapshot, SdkError> {
        let values = self.provider.storage().values.read().map_err(SdkError::mls)?;
        let map: &HashMap<Vec<u8>, Vec<u8>> = &values;
        Ok(map.iter().map(|(key, value)| (key.clone(), value.clone())).collect())
    }

    pub fn public_key(&self) -> Vec<u8> {
        self.signer.public().to_vec()
    }

    fn load(&self, group_id: &[u8]) -> Result<MlsGroup, SdkError> {
        MlsGroup::load(self.provider.storage(), &GroupId::from_slice(group_id))
            .map_err(SdkError::mls)?
            .ok_or_else(|| SdkError::Mls("group missing from storage".into()))
    }

    pub fn create_group(&self, group_id: &[u8]) -> Result<(), SdkError> {
        let config = MlsGroupCreateConfig::builder().ciphersuite(CIPHERSUITE).use_ratchet_tree_extension(true).build();
        MlsGroup::new_with_group_id(&self.provider, &self.signer, &config, GroupId::from_slice(group_id), self.credential.clone())
            .map_err(SdkError::mls)?;
        Ok(())
    }

    pub fn key_package(&self) -> Result<Vec<u8>, SdkError> {
        let bundle =
            KeyPackage::builder().build(CIPHERSUITE, &self.provider, &self.signer, self.credential.clone()).map_err(SdkError::mls)?;
        bundle.key_package().tls_serialize_detached().map_err(SdkError::mls)
    }

    /// The members of a group, in leaf order.
    pub fn members(&self, group_id: &[u8]) -> Result<Vec<Leaf>, SdkError> {
        let group = self.load(group_id)?;
        Ok(group.members().map(|member| Leaf { device_id: identity_of(&member.credential), signature_key: member.signature_key }).collect())
    }

    /// Stages the Add of a key package; returns the commit and the Welcome to send. The
    /// package must belong to the expected device, with its pinned key, and the device must
    /// not be in the group already.
    pub fn add_member(&self, group_id: &[u8], key_package: &[u8], expected: ExpectedJoiner) -> Result<(Vec<u8>, Vec<u8>), SdkError> {
        let mut group = self.load(group_id)?;
        let package = KeyPackageIn::tls_deserialize_exact(key_package)
            .map_err(SdkError::mls)?
            .validate(self.provider.crypto(), ProtocolVersion::Mls10)
            .map_err(SdkError::mls)?;
        let joiner = leaf_of(package.leaf_node());
        if joiner.device_id != expected.device_id {
            return Err(SdkError::Invalid(format!("the key package belongs to {}, not to {}", joiner.device_id, expected.device_id)));
        }
        if expected.pinned_key.is_some_and(|pinned| pinned != joiner.signature_key) {
            return Err(SdkError::Invalid(format!("{} presents a signing key that differs from the one first seen", joiner.device_id)));
        }
        if group.members().any(|member| identity_of(&member.credential) == joiner.device_id) {
            return Err(SdkError::Invalid(format!("{} is already in the group", joiner.device_id)));
        }
        let (commit, welcome, _) = group.add_members(&self.provider, &self.signer, &[package]).map_err(SdkError::mls)?;
        Ok((serialize(&commit)?, serialize(&welcome)?))
    }

    /// Removes every leaf of a device, so no copy of it stays in the group.
    pub fn remove_member(&self, group_id: &[u8], device_id: &str) -> Result<Vec<u8>, SdkError> {
        let mut group = self.load(group_id)?;
        let leaves: Vec<LeafNodeIndex> =
            group.members().filter(|member| identity_of(&member.credential) == device_id).map(|member| member.index).collect();
        if leaves.is_empty() {
            return Err(SdkError::Invalid(format!("{device_id} is not in the group")));
        }
        let (commit, _, _) = group.remove_members(&self.provider, &self.signer, &leaves).map_err(SdkError::mls)?;
        serialize(&commit)
    }

    pub fn merge_pending(&self, group_id: &[u8]) -> Result<(), SdkError> {
        self.load(group_id)?.merge_pending_commit(&self.provider).map_err(SdkError::mls)
    }

    pub fn clear_pending(&self, group_id: &[u8]) -> Result<(), SdkError> {
        self.load(group_id)?.clear_pending_commit(self.provider.storage()).map_err(SdkError::mls)
    }

    /// Joins from a Welcome; returns the group, who sent the Welcome and the members.
    pub fn join(&self, welcome: &[u8]) -> Result<Joined, SdkError> {
        let MlsMessageBodyIn::Welcome(welcome) = decode_message(welcome)?.extract() else {
            return Err(SdkError::Invalid("not a Welcome".into()));
        };
        let config = MlsGroupJoinConfig::builder().use_ratchet_tree_extension(true).build();
        let staged = StagedWelcome::new_from_welcome(&self.provider, &config, welcome, None).map_err(SdkError::mls)?;
        let welcomer = identity_of(staged.welcome_sender().map_err(SdkError::mls)?.credential());
        let group = staged.into_group(&self.provider).map_err(SdkError::mls)?;
        let group_id = group.group_id().as_slice().to_vec();
        Ok(Joined { members: self.members(&group_id)?, group_id, welcomer })
    }

    /// Forgets a group and its secrets, once this device left it.
    pub fn delete_group(&self, group_id: &[u8]) -> Result<(), SdkError> {
        self.load(group_id)?.delete(self.provider.storage()).map_err(SdkError::mls)
    }

    pub fn encrypt(&self, group_id: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, SdkError> {
        let mut group = self.load(group_id)?;
        let message = group.create_message(&self.provider, &self.signer, plaintext).map_err(SdkError::mls)?;
        serialize(&message)
    }

    /// Decrypts an application message, or checks a commit without applying it.
    pub fn process(&self, group_id: &[u8], bytes: &[u8]) -> Result<Incoming, SdkError> {
        let mut group = self.load(group_id)?;
        let protocol = decode_message(bytes)?.try_into_protocol_message().map_err(SdkError::mls)?;
        let processed = group.process_message(&self.provider, protocol).map_err(SdkError::mls)?;
        let sender = identity_of(processed.credential());
        match processed.into_content() {
            ProcessedMessageContent::ApplicationMessage(app) => Ok(Incoming::Application { sender, plaintext: app.into_bytes() }),
            ProcessedMessageContent::StagedCommitMessage(staged) => {
                let added = staged.add_proposals().map(|add| leaf_of(add.add_proposal().key_package().leaf_node())).collect();
                Ok(Incoming::Commit(ReviewedCommit { committer: sender, added, self_removed: staged.self_removed(), staged }))
            }
            _ => Ok(Incoming::Ignored),
        }
    }

    pub fn merge_commit(&self, group_id: &[u8], commit: ReviewedCommit) -> Result<(), SdkError> {
        self.load(group_id)?.merge_staged_commit(&self.provider, *commit.staged).map_err(SdkError::mls)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine(id: &str) -> MlsEngine {
        MlsEngine::new(id, MlsEngine::new_signer().unwrap()).unwrap()
    }

    #[test]
    fn adds_only_the_expected_device_with_its_pinned_key() {
        let (admin, bob) = (engine("admin"), engine("bob"));
        admin.create_group(b"room").unwrap();
        let package = bob.key_package().unwrap();
        let other_key = vec![7u8; 32];
        let refused = |device_id: &str, pinned_key: Option<&[u8]>| {
            admin.add_member(b"room", &package, ExpectedJoiner { device_id, pinned_key }).is_err()
        };
        assert!(refused("eve", None));
        assert!(refused("bob", Some(&other_key)));
        assert!(refused("admin", None));
        let expected = ExpectedJoiner { device_id: "bob", pinned_key: Some(&bob.public_key()) };
        let (_, welcome) = admin.add_member(b"room", &package, expected).unwrap();
        admin.merge_pending(b"room").unwrap();
        let joined = bob.join(&welcome).unwrap();
        assert_eq!(joined.welcomer, "admin");
        assert_eq!(joined.members.iter().map(|leaf| leaf.device_id.as_str()).collect::<Vec<_>>(), vec!["admin", "bob"]);
    }
}

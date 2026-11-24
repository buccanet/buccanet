//! MLS through OpenMLS: one engine per device, its storage exportable as one blob.

use std::collections::HashMap;

use buccanet_protocol::{ADMINS_EXTENSION, CIPHERSUITE};
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
    pub removed: Vec<String>,
    /// The admin list the commit sets, when it changes the group's admin extension.
    pub admins: Option<Vec<String>>,
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

/// What every leaf this device creates supports beyond the defaults: the admin list.
fn capabilities() -> Capabilities {
    Capabilities::builder().extensions(vec![ExtensionType::Unknown(ADMINS_EXTENSION)]).build()
}

fn admins_in(extensions: &Extensions<GroupContext>) -> Result<Option<Vec<String>>, SdkError> {
    extensions
        .unknown(ADMINS_EXTENSION)
        .map(|extension| serde_json::from_slice(&extension.0).map_err(|error| SdkError::Mls(format!("admin list: {error}"))))
        .transpose()
}

/// The group's extensions with `admins` as its admin list, required of every member.
fn with_admins(group: &MlsGroup, admins: &[String]) -> Result<Extensions<GroupContext>, SdkError> {
    let mut extensions = group.extensions().clone();
    let required = RequiredCapabilitiesExtension::new(&[ExtensionType::Unknown(ADMINS_EXTENSION)], &[], &[]);
    let list = serde_json::to_vec(admins).map_err(|error| SdkError::Mls(error.to_string()))?;
    extensions.add_or_replace(Extension::RequiredCapabilities(required)).map_err(SdkError::mls)?;
    extensions.add_or_replace(Extension::Unknown(ADMINS_EXTENSION, UnknownExtension(list))).map_err(SdkError::mls)?;
    Ok(extensions)
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
        let config =
            MlsGroupCreateConfig::builder().ciphersuite(CIPHERSUITE).capabilities(capabilities()).use_ratchet_tree_extension(true).build();
        MlsGroup::new_with_group_id(&self.provider, &self.signer, &config, GroupId::from_slice(group_id), self.credential.clone())
            .map_err(SdkError::mls)?;
        Ok(())
    }

    pub fn key_package(&self) -> Result<Vec<u8>, SdkError> {
        let bundle = KeyPackage::builder()
            .leaf_node_capabilities(capabilities())
            .build(CIPHERSUITE, &self.provider, &self.signer, self.credential.clone())
            .map_err(SdkError::mls)?;
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

    /// Removes every leaf of a device, so no copy of it stays in the group. `admins` is the new
    /// admin list when the device was an admin.
    pub fn remove_member(&self, group_id: &[u8], device_id: &str, admins: Option<&[String]>) -> Result<Vec<u8>, SdkError> {
        let mut group = self.load(group_id)?;
        let leaves: Vec<LeafNodeIndex> =
            group.members().filter(|member| identity_of(&member.credential) == device_id).map(|member| member.index).collect();
        if leaves.is_empty() {
            return Err(SdkError::Invalid(format!("{device_id} is not in the group")));
        }
        let extensions = admins.map(|admins| with_admins(&group, admins)).transpose()?;
        let mut builder = group.commit_builder().propose_removals(leaves);
        if let Some(extensions) = extensions {
            builder = builder.propose_group_context_extensions(extensions).map_err(SdkError::mls)?;
        }
        let bundle = builder
            .load_psks(self.provider.storage())
            .map_err(SdkError::mls)?
            .build(self.provider.rand(), self.provider.crypto(), &self.signer, |_| true)
            .map_err(SdkError::mls)?
            .stage_commit(&self.provider)
            .map_err(SdkError::mls)?;
        serialize(bundle.commit())
    }

    /// The group's admin list, or None while its creator is its only admin.
    pub fn admins(&self, group_id: &[u8]) -> Result<Option<Vec<String>>, SdkError> {
        admins_in(self.load(group_id)?.extensions())
    }

    /// Members whose leaf cannot hold an admin list: devices on a Buccanet older than admin lists.
    pub fn without_admin_support(&self, group_id: &[u8]) -> Result<Vec<String>, SdkError> {
        let group = self.load(group_id)?;
        let extension = ExtensionType::Unknown(ADMINS_EXTENSION);
        Ok(group
            .public_group()
            .members()
            .filter(|member| {
                !group.public_group().leaf(member.index).is_some_and(|leaf| leaf.capabilities().extensions().contains(&extension))
            })
            .map(|member| identity_of(&member.credential))
            .collect())
    }

    /// Stages a commit that replaces the admin list; returns it to send.
    pub fn set_admins(&self, group_id: &[u8], admins: &[String]) -> Result<Vec<u8>, SdkError> {
        let mut group = self.load(group_id)?;
        let extensions = with_admins(&group, admins)?;
        let (commit, _, _) = group.update_group_context_extensions(&self.provider, extensions, &self.signer).map_err(SdkError::mls)?;
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
            ProcessedMessageContent::StagedCommitMessage(staged) => Ok(Incoming::Commit(review(&group, sender, staged)?)),
            _ => Ok(Incoming::Ignored),
        }
    }

    pub fn merge_commit(&self, group_id: &[u8], commit: ReviewedCommit) -> Result<(), SdkError> {
        self.load(group_id)?.merge_staged_commit(&self.provider, *commit.staged).map_err(SdkError::mls)
    }
}

/// Reads what a staged commit changes, so the caller can decide before merging it.
fn review(group: &MlsGroup, committer: String, staged: Box<StagedCommit>) -> Result<ReviewedCommit, SdkError> {
    let added = staged.add_proposals().map(|add| leaf_of(add.add_proposal().key_package().leaf_node())).collect();
    let removed =
        staged.remove_proposals().filter_map(|remove| group.member(remove.remove_proposal().removed())).map(identity_of).collect();
    let changes_admins = staged.queued_proposals().any(|queued| matches!(queued.proposal(), Proposal::GroupContextExtensions(_)));
    let admins = if changes_admins { admins_in(staged.group_context().extensions())? } else { None };
    Ok(ReviewedCommit { committer, added, removed, admins, self_removed: staged.self_removed(), staged })
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

    fn committed(engine: &MlsEngine, bytes: &[u8]) -> ReviewedCommit {
        match engine.process(b"room", bytes).unwrap() {
            Incoming::Commit(commit) => commit,
            _ => panic!("expected a commit"),
        }
    }

    #[test]
    fn a_second_admin_comes_from_a_commit_and_leaves_with_its_removal() {
        let (ada, bob, eve) = (engine("ada"), engine("bob"), engine("eve"));
        ada.create_group(b"room").unwrap();
        let add = |engine: &MlsEngine, id: &str| {
            let (commit, welcome) =
                ada.add_member(b"room", &engine.key_package().unwrap(), ExpectedJoiner { device_id: id, pinned_key: None }).unwrap();
            ada.merge_pending(b"room").unwrap();
            (commit, welcome)
        };
        bob.join(&add(&bob, "bob").1).unwrap();
        assert_eq!(ada.admins(b"room").unwrap(), None);
        let (add_eve, welcome) = add(&eve, "eve");
        let commit = committed(&bob, &add_eve);
        bob.merge_commit(b"room", commit).unwrap();
        eve.join(&welcome).unwrap();

        let admins = vec!["ada".to_string(), "bob".to_string()];
        assert!(ada.without_admin_support(b"room").unwrap().is_empty());
        let promote = ada.set_admins(b"room", &admins).unwrap();
        ada.merge_pending(b"room").unwrap();
        for member in [&bob, &eve] {
            let commit = committed(member, &promote);
            assert_eq!(commit.admins.as_deref(), Some(admins.as_slice()));
            member.merge_commit(b"room", commit).unwrap();
            assert_eq!(member.admins(b"room").unwrap().as_deref(), Some(admins.as_slice()));
        }

        let remove_ada = bob.remove_member(b"room", "ada", Some(&["bob".to_string()])).unwrap();
        bob.merge_pending(b"room").unwrap();
        let commit = committed(&eve, &remove_ada);
        assert_eq!((commit.committer.as_str(), commit.removed.clone()), ("bob", vec!["ada".to_string()]));
        assert_eq!(commit.admins, Some(vec!["bob".to_string()]));
        eve.merge_commit(b"room", commit).unwrap();
        assert_eq!(eve.admins(b"room").unwrap(), Some(vec!["bob".to_string()]));
    }
}

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

pub enum Incoming {
    Application { sender: String, plaintext: Vec<u8> },
    Commit { self_removed: bool },
    Ignored,
}

pub type StorageSnapshot = Vec<(Vec<u8>, Vec<u8>)>;

fn credential_for(identity: &str, signer: &SignatureKeyPair) -> CredentialWithKey {
    let credential = BasicCredential::new(identity.as_bytes().to_vec());
    CredentialWithKey { credential: credential.into(), signature_key: signer.public().into() }
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

    /// Stages the Add of a key package; returns the commit and the Welcome to send.
    pub fn add_member(&self, group_id: &[u8], key_package: &[u8]) -> Result<(Vec<u8>, Vec<u8>), SdkError> {
        let mut group = self.load(group_id)?;
        let package = KeyPackageIn::tls_deserialize_exact(key_package)
            .map_err(SdkError::mls)?
            .validate(self.provider.crypto(), ProtocolVersion::Mls10)
            .map_err(SdkError::mls)?;
        let (commit, welcome, _) = group.add_members(&self.provider, &self.signer, &[package]).map_err(SdkError::mls)?;
        Ok((serialize(&commit)?, serialize(&welcome)?))
    }

    pub fn remove_member(&self, group_id: &[u8], device_id: &str) -> Result<Vec<u8>, SdkError> {
        let mut group = self.load(group_id)?;
        let leaf = group
            .members()
            .find(|member| member.credential.serialized_content() == device_id.as_bytes())
            .map(|member| member.index)
            .ok_or_else(|| SdkError::Invalid(format!("{device_id} is not in the group")))?;
        let (commit, _, _) = group.remove_members(&self.provider, &self.signer, &[leaf]).map_err(SdkError::mls)?;
        serialize(&commit)
    }

    pub fn merge_pending(&self, group_id: &[u8]) -> Result<(), SdkError> {
        self.load(group_id)?.merge_pending_commit(&self.provider).map_err(SdkError::mls)
    }

    pub fn clear_pending(&self, group_id: &[u8]) -> Result<(), SdkError> {
        self.load(group_id)?.clear_pending_commit(self.provider.storage()).map_err(SdkError::mls)
    }

    /// Joins from a Welcome and returns the group id.
    pub fn join(&self, welcome: &[u8]) -> Result<Vec<u8>, SdkError> {
        let MlsMessageBodyIn::Welcome(welcome) = decode_message(welcome)?.extract() else {
            return Err(SdkError::Invalid("not a Welcome".into()));
        };
        let config = MlsGroupJoinConfig::builder().use_ratchet_tree_extension(true).build();
        let group = StagedWelcome::new_from_welcome(&self.provider, &config, welcome, None)
            .map_err(SdkError::mls)?
            .into_group(&self.provider)
            .map_err(SdkError::mls)?;
        Ok(group.group_id().as_slice().to_vec())
    }

    pub fn encrypt(&self, group_id: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, SdkError> {
        let mut group = self.load(group_id)?;
        let message = group.create_message(&self.provider, &self.signer, plaintext).map_err(SdkError::mls)?;
        serialize(&message)
    }

    pub fn process(&self, group_id: &[u8], bytes: &[u8]) -> Result<Incoming, SdkError> {
        let mut group = self.load(group_id)?;
        let protocol = decode_message(bytes)?.try_into_protocol_message().map_err(SdkError::mls)?;
        let processed = group.process_message(&self.provider, protocol).map_err(SdkError::mls)?;
        let sender = String::from_utf8_lossy(processed.credential().serialized_content()).into_owned();
        match processed.into_content() {
            ProcessedMessageContent::ApplicationMessage(app) => Ok(Incoming::Application { sender, plaintext: app.into_bytes() }),
            ProcessedMessageContent::StagedCommitMessage(staged) => {
                let self_removed = staged.self_removed();
                group.merge_staged_commit(&self.provider, *staged).map_err(SdkError::mls)?;
                Ok(Incoming::Commit { self_removed })
            }
            _ => Ok(Incoming::Ignored),
        }
    }
}

use orchard::{
    builder::BundleMetadata,
    bundle::{Authorization, Flags},
    note::{ExtractedNoteCommitment, Nullifier, TransmittedNoteCiphertext},
    Anchor,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Block {
    pub transactions: Vec<Transaction>,
}

impl Block {
    /// These must be added to the nullifier set when a block is connected.
    pub fn nullifiers(&self) -> Vec<Nullifier> {
        let mut nullifiers = vec![];
        for transaction in &self.transactions {
            let transaction_nullifiers = transaction.nullifiers();
            nullifiers.extend(transaction_nullifiers);
        }
        nullifiers
    }

    /// These must be appended to the incremental note commitment merkle tree when a block is
    /// connected.
    pub fn extracted_note_commitments(&self) -> Vec<ExtractedNoteCommitment> {
        let mut extracted_note_commitments = vec![];
        for transaction in &self.transactions {
            let transaction_commitments = transaction.extracted_note_commitments();
            extracted_note_commitments.extend(transaction_commitments);
        }
        extracted_note_commitments
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Output {
    pub value: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Transaction {
    pub inputs: Vec<u32>,
    pub outputs: Vec<Output>,
    pub actions: Vec<Action>,
    pub value_balance_orchard: i64,
}

impl Transaction {
    pub fn to_bundle(
        &self,
        anchor: Anchor,
    ) -> Option<orchard::bundle::Bundle<orchard::bundle::testing::Unauthorized, i64>> {
        let actions: Vec<orchard::Action<()>> = self
            .actions
            .iter()
            .cloned()
            .map(|action| action.into())
            .collect();
        let actions = match nonempty::NonEmpty::from_vec(actions) {
            Some(actions) => actions,
            None => return None,
        };
        let flags = Flags::ENABLED;
        let value_balance_orchard = self.value_balance_orchard;
        let authorization = orchard::bundle::testing::Unauthorized;
        Some(orchard::Bundle::from_parts(
            actions,
            flags,
            value_balance_orchard,
            anchor,
            authorization,
        ))
    }

    pub fn from_bundle<T: Authorization>(
        inputs: Vec<u32>,
        outputs: Vec<Output>,
        bundle: &Option<(orchard::bundle::Bundle<T, i64>, BundleMetadata)>,
    ) -> Self {
        match bundle {
            Some((bundle, _bundle_metadata)) => {
                let mut actions = vec![];
                for action in bundle.actions() {
                    let action = Action::from(action);
                    actions.push(action);
                }
                Self {
                    inputs,
                    outputs,
                    actions,
                    value_balance_orchard: *bundle.value_balance(),
                }
            }
            None => Self {
                inputs,
                outputs,
                actions: vec![],
                value_balance_orchard: 0,
            },
        }
    }

    /// These must be added to the nullifier set when a block is connected.
    pub fn nullifiers(&self) -> Vec<Nullifier> {
        let mut nullifiers = vec![];
        for action in &self.actions {
            let action = orchard::Action::from(action);
            let nullifier = action.nullifier();
            nullifiers.push(*nullifier);
        }
        nullifiers
    }

    /// These must be appended to the incremental note commitment merkle tree when a block is
    /// connected.
    pub fn extracted_note_commitments(&self) -> Vec<ExtractedNoteCommitment> {
        let mut extracted_note_commitments = vec![];
        for action in &self.actions {
            let action = orchard::Action::from(action);
            let extracted_note_commitment = action.cmx();
            extracted_note_commitments.push(*extracted_note_commitment);
        }
        extracted_note_commitments
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Action {
    pub nf: [u8; 32],
    pub rk: [u8; 32],
    pub cmx: [u8; 32],
    pub epk_bytes: [u8; 32],
    pub enc_ciphertext: Vec<u8>, // Must be 580 bytes.
    pub out_ciphertext: Vec<u8>, // Must be 80 bytes.
    pub cv_net: [u8; 32],
}

impl<T> From<&orchard::Action<T>> for Action {
    fn from(value: &orchard::Action<T>) -> Self {
        let nf = value.nullifier().to_bytes();
        let rk = value.rk().into();
        let cmx = value.cmx().to_bytes();
        let TransmittedNoteCiphertext {
            epk_bytes,
            enc_ciphertext,
            out_ciphertext,
        } = value.encrypted_note();
        let cv_net = value.cv_net().to_bytes();
        Action {
            nf,
            rk,
            cmx,
            epk_bytes: *epk_bytes,
            enc_ciphertext: enc_ciphertext.to_vec(),
            out_ciphertext: out_ciphertext.to_vec(),
            cv_net,
        }
    }
}

impl From<Action> for orchard::Action<()> {
    fn from(value: Action) -> Self {
        let nf = orchard::note::Nullifier::from_bytes(&value.nf).unwrap();
        let rk = orchard::primitives::redpallas::VerificationKey::try_from(value.rk).unwrap();
        let cmx = orchard::note::ExtractedNoteCommitment::from_bytes(&value.cmx).unwrap();
        let encrypted_note = orchard::note::TransmittedNoteCiphertext {
            epk_bytes: value.epk_bytes,
            enc_ciphertext: value.enc_ciphertext.try_into().unwrap(),
            out_ciphertext: value.out_ciphertext.try_into().unwrap(),
        };
        let cv_net = orchard::value::ValueCommitment::from_bytes(&value.cv_net).unwrap();
        orchard::Action::from_parts(nf, rk, cmx, encrypted_note, cv_net, ())
    }
}

impl From<&Action> for orchard::Action<()> {
    fn from(value: &Action) -> Self {
        let nf = orchard::note::Nullifier::from_bytes(&value.nf).unwrap();
        let rk = orchard::primitives::redpallas::VerificationKey::try_from(value.rk).unwrap();
        let cmx = orchard::note::ExtractedNoteCommitment::from_bytes(&value.cmx).unwrap();
        let encrypted_note = orchard::note::TransmittedNoteCiphertext {
            epk_bytes: value.epk_bytes,
            enc_ciphertext: value.enc_ciphertext.clone().try_into().unwrap(),
            out_ciphertext: value.out_ciphertext.clone().try_into().unwrap(),
        };
        let cv_net = orchard::value::ValueCommitment::from_bytes(&value.cv_net).unwrap();
        orchard::Action::from_parts(nf, rk, cmx, encrypted_note, cv_net, ())
    }
}

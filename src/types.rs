use orchard::{
    bundle::{Authorization, Flags},
    note::TransmittedNoteCiphertext,
    Anchor,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Block {
    pub anchor: [u8; 32],
    pub transactions: Vec<Transaction>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Transaction {
    pub actions: Vec<Action>,
    pub value_balance: i64,
    pub anchor: [u8; 32],
}

impl<T: Authorization> From<orchard::bundle::Bundle<T, i64>> for Transaction {
    fn from(value: orchard::bundle::Bundle<T, i64>) -> Self {
        let mut actions = vec![];
        for action in value.actions() {
            let action = Action::from(action);
            actions.push(action);
        }
        Transaction {
            actions,
            anchor: value.anchor().to_bytes(),
            value_balance: *value.value_balance(),
        }
    }
}

impl From<Transaction> for orchard::bundle::Bundle<orchard::bundle::testing::Unauthorized, i64> {
    fn from(value: Transaction) -> Self {
        let actions: Vec<orchard::Action<()>> = value
            .actions
            .iter()
            .cloned()
            .map(|action| action.into())
            .collect();
        let actions = nonempty::NonEmpty::from_vec(actions).unwrap();
        let flags = Flags::ENABLED;
        let value_balance = value.value_balance;
        let anchor = Anchor::from_bytes(value.anchor).unwrap();
        let authorization = orchard::bundle::testing::Unauthorized;
        orchard::Bundle::from_parts(actions, flags, value_balance, anchor, authorization)
    }
}

impl From<&Transaction> for orchard::bundle::Bundle<orchard::bundle::testing::Unauthorized, i64> {
    fn from(value: &Transaction) -> Self {
        let actions: Vec<orchard::Action<()>> = value
            .actions
            .iter()
            .cloned()
            .map(|action| action.into())
            .collect();
        let actions = nonempty::NonEmpty::from_vec(actions).unwrap();
        let flags = Flags::ENABLED;
        let value_balance = value.value_balance;
        let anchor = Anchor::from_bytes(value.anchor).unwrap();
        let authorization = orchard::bundle::testing::Unauthorized;
        orchard::Bundle::from_parts(actions, flags, value_balance, anchor, authorization)
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

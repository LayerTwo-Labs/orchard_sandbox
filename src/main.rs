use std::collections::HashSet;

use incrementalmerkletree::frontier::Frontier;
use miette::IntoDiagnostic;
use orchard::{
    builder::{BundleType, UnauthorizedBundle},
    bundle::Flags,
    circuit::{ProvingKey, VerifyingKey},
    keys::{Diversifier, FullViewingKey, IncomingViewingKey, OutgoingViewingKey, SpendingKey},
    note::{Note, Nullifier, RandomSeed, Rho},
    tree::{MerkleHashOrchard, MerklePath},
    value::NoteValue,
    Action, Anchor, Bundle,
};
use rand::SeedableRng;
use zip32::{arbitrary::SecretKey, AccountId, ChildIndex, DiversifierIndex};

fn main() -> miette::Result<()> {
    let seed = [0; 32];
    let sk = SpendingKey::from_zip32_seed(&seed, 0, AccountId::ZERO).unwrap();

    let full_viewing_key = FullViewingKey::from(&sk);

    let diversifier = Diversifier::from_bytes([0; 11]);

    let recipient = full_viewing_key.address(diversifier, orchard::keys::Scope::Internal);
    let value = NoteValue::from_raw(100);

    let mut nullifiers = HashSet::new();
    let mut frontier = Frontier::<MerkleHashOrchard, 32>::empty();

    dbg!(frontier.tree_size());
    dbg!(frontier.root());

    let anchor: Anchor = frontier.root().into();

    let mut builder = orchard::builder::Builder::new(
        BundleType::Transactional {
            flags: Flags::SPENDS_DISABLED,
            bundle_required: false,
        },
        anchor,
    );
    builder
        .add_output(None, recipient, value, None)
        .into_diagnostic()?;
    builder
        .add_output(None, recipient, value, None)
        .into_diagnostic()?;
    builder
        .add_output(None, recipient, value, None)
        .into_diagnostic()?;
    builder
        .add_output(None, recipient, value, None)
        .into_diagnostic()?;

    let rng = rand::rngs::StdRng::from_entropy();
    let (bundle, bundle_metadata) = builder.build::<i64>(rng).into_diagnostic()?.unwrap();

    println!("after bundle construction");

    dbg!(bundle.value_balance());

    let fvk = FullViewingKey::from(&sk);

    let notes = bundle.decrypt_outputs_with_keys(&[fvk.to_ivk(orchard::keys::Scope::Internal)]);
    // dbg!(notes);
    // dbg!(bundle.anchor());

    for action in bundle.actions() {
        let cmx = action.cmx();
        let mho = MerkleHashOrchard::from_cmx(cmx);
        frontier.append(mho);
        nullifiers.insert(action.nullifier().to_bytes());
    }

    dbg!(frontier.tree_size());
    dbg!(frontier.root());

    for nullifier in &nullifiers {
        dbg!(Nullifier::from_bytes(nullifier).unwrap());
    }

    let anchor: Anchor = frontier.root().into();

    let mut builder = orchard::builder::Builder::new(
        BundleType::Transactional {
            flags: Flags::ENABLED,
            bundle_required: false,
        },
        anchor,
    );
    builder
        .add_output(None, recipient, value, None)
        .into_diagnostic()?;
    builder
        .add_output(None, recipient, value, None)
        .into_diagnostic()?;
    builder
        .add_output(None, recipient, value, None)
        .into_diagnostic()?;
    builder
        .add_output(None, recipient, value, None)
        .into_diagnostic()?;

    let rng = rand::rngs::StdRng::from_entropy();
    let (bundle, bundle_metadata) = builder.build::<i64>(rng).into_diagnostic()?.unwrap();

    for action in bundle.actions() {
        let cmx = action.cmx();
        let mho = MerkleHashOrchard::from_cmx(cmx);
        frontier.append(mho);
        nullifiers.insert(action.nullifier().to_bytes());
    }

    dbg!(frontier.tree_size());
    dbg!(frontier.root());

    for nullifier in &nullifiers {
        dbg!(Nullifier::from_bytes(nullifier).unwrap());
    }

    dbg!(bundle.value_balance());

    Ok(())
}

#[derive(Debug)]
struct Wallet {
    notes: Vec<(Note, MerklePath)>,
    transparent_value: i64,
}

#[derive(Debug)]
struct Block {
    anchor: Anchor,
    bundles: Vec<UnauthorizedBundle<i64>>,
}

#[derive(Debug)]
struct Chain {
    blocks: Vec<Block>,
}

// there is a transparent

// conjure note
// spend note
// add note
//
// database for nullifier set
// incremental note commitment merkle tree

mod cli;
mod db;
mod types;

use clap::Parser as _;

use std::collections::HashSet;

use incrementalmerkletree::{
    frontier::{Frontier, NonEmptyFrontier},
    Address,
};
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
use zip32::AccountId;

const NOTE_COMMITMENT_TREE_DEPTH: u8 = 32;

fn main() -> miette::Result<()> {
    /*
    let mut rng = rand::thread_rng();
    let leaf = MerkleHashOrchard::random(&mut rng);
    let mut frontier = NonEmptyFrontier::<MerkleHashOrchard>::new(leaf);

    dbg!(&frontier);
    for _ in 0..100 {
        frontier.append(leaf);
        dbg!(&frontier.root(None));
    }

    let parts = frontier.into_parts(); // store this in DB.
    dbg!(parts);
    */

    let cli = cli::Cli::parse();

    let db = db::Db::new()?;

    // You can check for the existence of subcommands, and if found use their
    // matches just as you would the top level cmd
    match &cli.command {
        cli::Commands::Wallet => {
            let outputs = db.get_outputs()?;
            for (recipient, value) in outputs {
                let recipient = bs58::encode(recipient).with_check().into_string();
                println!("{recipient}: {value}");
            }
        }
        cli::Commands::CreateNote { value, recipient } => {
            db.create_note(recipient.clone(), *value)?;
        }
        cli::Commands::SpendNote { id } => {}
        cli::Commands::SubmitTxn => {
            db.submit_transaction()?;
        }
        cli::Commands::Mine => {
            db.mine()?;
        }
        cli::Commands::GetMnemonic => {
            let mnemonic = db.get_mnemonic()?;
            let phrase = mnemonic.phrase().to_string();
            println!("{phrase}");
        }
        cli::Commands::GetNewAddress => {
            let address = db.get_new_address()?;
            let address_bytes = address.to_raw_address_bytes();
            let address_string = bs58::encode(address_bytes).with_check().into_string();
            println!("{address_string}");
        }
    }
    Ok(())
}

fn example() -> miette::Result<()> {
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

// there is a transparent

// conjure note
// spend note
// add note
//
// database for nullifier set
// incremental note commitment merkle tree

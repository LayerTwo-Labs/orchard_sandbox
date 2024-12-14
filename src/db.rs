use bip39::{Mnemonic, Seed};
use incrementalmerkletree::{frontier::NonEmptyFrontier, Position};
use miette::{miette, IntoDiagnostic};
use orchard::{
    builder::BundleType,
    bundle::Flags,
    note::{Nullifier, RandomSeed, Rho},
    tree::{MerkleHashOrchard, MerklePath},
    value::NoteValue,
    Address, Anchor, Note,
};
use rand::SeedableRng;
use rusqlite::Connection;
use rusqlite_migration::{Migrations, M};
use zip32::AccountId;

use crate::types::{Block, Output};

pub struct Db {
    pub conn: Connection,
}

impl Db {
    pub fn new() -> miette::Result<Self> {
        // 1️⃣ Define migrations
        let migrations = Migrations::new(vec![
            M::up(
                "CREATE TABLE utxos(
                    id INTEGER PRIMARY KEY,
                    value INTEGER NOT NULL
            );",
            ),
            M::up(
                "CREATE TABLE notes(
                    id INTEGER PRIMARY KEY,
                    recipient BLOB NOT NULL,
                    value INTEGER NOT NULL,
                    rho BLOB NOT NULL,
                    rseed BLOB NOT NULL,
                    merkle_path BLOB NOT NULL

            );",
            ),
            M::up(
                "CREATE TABLE wallet_seed(
                    phrase TEXT NOT NULL
            );",
            ),
            M::up(
                "CREATE TABLE addresses(
                    id INTEGER PRIMARY KEY,
                    address TEXT
            );",
            ),
            M::up(
                "CREATE TABLE nullifiers(
                    id INTEGER PRIMARY KEY,
                    nullifier BLOB NOT NULL
            );",
            ),
            M::up(
                "CREATE TABLE frontier(
                    position INTEGER NOT NULL,
                    leaf BLOB NOT NULL,
                    ommers BLOB NOT NULL
            );",
            ),
            M::up(
                "CREATE TABLE transactions(
                    id INTEGER PRIMARY KEY,
                    tx BLOB NOT NULL
            );",
            ),
            M::up(
                "CREATE TABLE blocks(
                    id INTEGER PRIMARY KEY,
                    anchor BLOB NOT NULL,
                    block BLOB NOT NULL
            );",
            ),
            M::up(
                "CREATE TABLE inputs(
                    id INTEGER PRIMARY KEY,
                    utxo_id INTEGER NOT NULL,
                    FOREIGN KEY(utxo_id) REFERENCES utxos(id)
            );",
            ),
            M::up(
                "CREATE TABLE outputs(
                    id INTEGER PRIMARY KEY,
                    value INTEGER NOT NULL
            );",
            ),
            M::up(
                "CREATE TABLE shielded_inputs(
                    id INTEGER PRIMARY KEY,
                    note_id INTEGER NOT NULL,
                    FOREIGN KEY(note_id) REFERENCES notes(id)
            );",
            ),
            M::up(
                "CREATE TABLE shielded_outputs(
                    id INTEGER PRIMARY KEY,
                    recipient BLOB NOT NULL,
                    value INTEGER NOT NULL
            );",
            ),
        ]);

        let mut conn = Connection::open("./orchard.db3").into_diagnostic()?;

        conn.pragma_update_and_check(None, "journal_mode", &"WAL", |_| Ok(()))
            .into_diagnostic()?;

        // 2️⃣ Update the database schema, atomically
        migrations.to_latest(&mut conn).into_diagnostic()?;

        let mut db = Db { conn };

        let tx = db.conn.transaction().into_diagnostic()?;
        if !Db::get_mnemonic(&tx).is_ok() {
            Db::generate_seed(&tx)?;
        }
        tx.commit().into_diagnostic()?;

        Ok(db)
    }

    pub fn get_inputs(tx: &rusqlite::Transaction) -> miette::Result<Vec<u32>> {
        let mut statement = tx.prepare("SELECT utxo_id FROM inputs").into_diagnostic()?;
        let inputs: Vec<u32> = statement
            .query_map([], |row| Ok(row.get(0)?))
            .into_diagnostic()?
            .collect::<Result<Vec<_>, _>>()
            .into_diagnostic()?;
        Ok(inputs)
    }

    pub fn get_outputs(tx: &rusqlite::Transaction) -> miette::Result<Vec<Output>> {
        let mut statement = tx.prepare("SELECT value FROM outputs").into_diagnostic()?;
        let outputs: Vec<u64> = statement
            .query_map([], |row| Ok(row.get(0)?))
            .into_diagnostic()?
            .collect::<Result<Vec<_>, _>>()
            .into_diagnostic()?;
        let outputs: Vec<Output> = outputs.into_iter().map(|value| Output { value }).collect();
        Ok(outputs)
    }

    pub fn get_shielded_inputs(tx: &rusqlite::Transaction) -> miette::Result<Vec<u32>> {
        let mut statement = tx
            .prepare("SELECT note_id FROM shielded_inputs")
            .into_diagnostic()?;
        let outputs: Vec<u32> = statement
            .query_map([], |row| Ok(row.get(0)?))
            .into_diagnostic()?
            .collect::<Result<Vec<_>, _>>()
            .into_diagnostic()?;
        Ok(outputs)
    }

    pub fn get_shielded_outputs(tx: &rusqlite::Transaction) -> miette::Result<Vec<(Vec<u8>, u64)>> {
        let mut statement = tx
            .prepare("SELECT recipient, value FROM shielded_outputs")
            .into_diagnostic()?;
        let outputs: Vec<_> = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .into_diagnostic()?
            .collect::<Result<Vec<_>, _>>()
            .into_diagnostic()?;
        Ok(outputs)
    }

    pub fn create_utxo(&self, value: u64) -> miette::Result<()> {
        self.conn
            .execute("INSERT INTO outputs (value) VALUES (?1)", [value])
            .into_diagnostic()?;
        Ok(())
    }

    pub fn spend_utxo(&self, utxo_id: u32) -> miette::Result<()> {
        self.conn
            .execute("INSERT INTO inputs (utxo_id) VALUES (?1)", [utxo_id])
            .into_diagnostic()?;
        Ok(())
    }

    pub fn create_note(&mut self, recipient: Option<String>, value: u64) -> miette::Result<()> {
        let recipient = match recipient {
            Some(recipient) => {
                let recipient = bs58::decode(recipient).into_vec().into_diagnostic()?;
                let recipient: [u8; 43] = recipient
                    .try_into()
                    .map_err(|_err| miette!("wrong address length"))?;
                let _ = Address::from_raw_address_bytes(&recipient)
                    .expect("subtle error, failed to construct shielded address from raw bytes");
                recipient
            }
            None => {
                let recipient = self.get_new_address()?;
                recipient.to_raw_address_bytes()
            }
        };
        self.conn
            .execute(
                "INSERT INTO shielded_outputs (recipient, value) VALUES (?1, ?2)",
                (recipient, value),
            )
            .into_diagnostic()?;
        Ok(())
    }

    pub fn spend_note(&self, note_id: u32) -> miette::Result<()> {
        self.conn
            .execute(
                "INSERT INTO shielded_inputs (note_id) VALUES (?1)",
                [note_id],
            )
            .into_diagnostic()?;
        Ok(())
    }

    pub fn get_bundle_anchor(tx: &rusqlite::Transaction) -> miette::Result<Anchor> {
        // We need an anchor that is a few blocks old in order to construct an Orchard bundle.
        let anchor = match tx.query_row(
            "SELECT anchor FROM blocks ORDER BY id LIMIT 1 OFFSET 3",
            [],
            |row| row.get(0),
        ) {
            Ok(anchor) => Anchor::from_bytes(anchor)
                .expect("subtle error, failed to construct anchor from bytes"),
            Err(rusqlite::Error::QueryReturnedNoRows) => Anchor::empty_tree(),
            Err(err) => return Err(err).into_diagnostic(),
        };
        Ok(anchor)
    }

    pub fn get_note(
        tx: &rusqlite::Transaction,
        note_id: u32,
    ) -> miette::Result<(Note, MerklePath)> {
        let (recipient, value, rho, rseed, merkle_path) = tx
            .query_row(
                "SELECT recipient, value, rho, rseed, merkle_path FROM notes WHERE id = ?1",
                [note_id],
                |row| {
                    let recipient: Vec<u8> = row.get(0)?;
                    let value: u64 = row.get(1)?;
                    let rho: Vec<u8> = row.get(2)?;
                    let rseed: Vec<u8> = row.get(3)?;
                    let merkle_path: Vec<u8> = row.get(4)?;
                    Ok((recipient, value, rho, rseed, merkle_path))
                },
            )
            .into_diagnostic()?;
        let recipient: [u8; 43] = recipient
            .try_into()
            .expect("wrong recipient address length");
        let recipient = Address::from_raw_address_bytes(&recipient)
            .expect("subtle error, failed to construct address from bytes");
        let value = NoteValue::from_raw(value);
        let rho: [u8; 32] = rho.try_into().expect("wrong rho length");
        let rho = Rho::from_bytes(&rho).expect("subtle error, failed to construct rho from bytes");
        let rseed: [u8; 32] = rseed.try_into().expect("wrong rseed length");
        let rseed = RandomSeed::from_bytes(rseed, &rho)
            .expect("subtle error, failed to construct rseed from bytes");
        let (position, auth_path): (u32, [MerkleHashOrchard; 32]) =
            bincode::deserialize(&merkle_path).into_diagnostic()?;
        let merkle_path = MerklePath::from_parts(position, auth_path);
        let note = Note::from_parts(recipient, value, rho, rseed)
            .expect("subtle error, failed to construct note from parts");
        Ok((note, merkle_path))
    }

    pub fn submit_transaction(&mut self) -> miette::Result<()> {
        let tx = self.conn.transaction().into_diagnostic()?;
        let anchor: Anchor = Self::get_bundle_anchor(&tx)?;
        let mut builder = orchard::builder::Builder::new(
            BundleType::Transactional {
                flags: Flags::ENABLED,
                bundle_required: false,
            },
            anchor,
        );
        let shielded_inputs = Self::get_shielded_inputs(&tx)?;
        for note_id in shielded_inputs {
            let (note, merkle_path) = Self::get_note(&tx, note_id)?;
            let sk = Self::get_sk(&tx)?;
            let fvk = orchard::keys::FullViewingKey::from(&sk);
            builder
                .add_spend(fvk, note, merkle_path)
                .into_diagnostic()?;
        }
        let shielded_outputs = Self::get_shielded_outputs(&tx)?;
        for (recipient, value) in shielded_outputs {
            let recipient: [u8; 43] = recipient
                .try_into()
                .map_err(|_err| miette!("wrong address length"))?;
            let recipient = Address::from_raw_address_bytes(&recipient).unwrap();
            let value = NoteValue::from_raw(value);
            builder
                .add_output(None, recipient, value, None)
                .into_diagnostic()?;
        }

        let rng = rand::rngs::StdRng::from_entropy();
        let (bundle, _bundle_metadata) = builder.build::<i64>(rng).into_diagnostic()?.unwrap();

        let inputs = Self::get_inputs(&tx)?;
        let outputs = Self::get_outputs(&tx)?;
        let transaction = crate::types::Transaction::from_bundle(inputs, outputs, &bundle);

        let transaction_bytes = bincode::serialize(&transaction).into_diagnostic()?;

        tx.execute(
            "INSERT INTO transactions (tx) VALUES (?1)",
            (&transaction_bytes,),
        )
        .into_diagnostic()?;
        tx.execute("DELETE FROM inputs", []).into_diagnostic()?;
        tx.execute("DELETE FROM outputs", []).into_diagnostic()?;
        tx.execute("DELETE FROM shielded_inputs", [])
            .into_diagnostic()?;
        tx.execute("DELETE FROM shielded_outputs", [])
            .into_diagnostic()?;

        tx.commit().into_diagnostic()?;

        Ok(())
    }

    fn get_transactions(
        tx: &rusqlite::Transaction,
    ) -> miette::Result<Vec<crate::types::Transaction>> {
        let mut statement = tx
            .prepare("SELECT tx FROM transactions")
            .into_diagnostic()?;
        let transactions: Vec<Vec<u8>> = statement
            .query_map([], |row| Ok(row.get(0)?))
            .into_diagnostic()?
            .collect::<Result<Vec<_>, _>>()
            .into_diagnostic()?;
        let transactions: Vec<crate::types::Transaction> = transactions
            .iter()
            .map(|bytes| bincode::deserialize(bytes))
            .collect::<Result<_, _>>()
            .into_diagnostic()?;
        Ok(transactions)
    }

    fn get_frontier(
        tx: &rusqlite::Transaction,
    ) -> miette::Result<Option<NonEmptyFrontier<MerkleHashOrchard>>> {
        let (position, leaf, ommers): (u64, Vec<u8>, Vec<u8>) =
            match tx.query_row("SELECT position, leaf, ommers FROM frontier", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            }) {
                Ok((position, leaf, ommers)) => (position, leaf, ommers),
                Err(rusqlite::Error::QueryReturnedNoRows) => {
                    return Ok(None);
                }
                Err(err) => {
                    return Err(err).into_diagnostic();
                }
            };
        let position = Position::from(position);
        let leaf: [u8; 32] = leaf
            .try_into()
            .map_err(|_| miette!("wrong leaf length in SQLite db"))?;
        let leaf = MerkleHashOrchard::from_bytes(&leaf)
            .expect("subtle error while converting leaf from bytes");
        let ommers: Vec<[u8; 32]> = bincode::deserialize(&ommers).into_diagnostic()?;
        let ommers: Vec<MerkleHashOrchard> = ommers
            .iter()
            .map(|ommer| {
                MerkleHashOrchard::from_bytes(ommer)
                    .expect("subtle error while converting ommer from bytes")
            })
            .collect();
        let frontier = NonEmptyFrontier::from_parts(position, leaf, ommers)
            .expect("failed to reconstruct frontier");
        Ok(Some(frontier))
    }

    fn update_frontier(
        tx: &rusqlite::Transaction,
        frontier: NonEmptyFrontier<MerkleHashOrchard>,
    ) -> miette::Result<()> {
        tx.execute("DELETE FROM frontier", []).into_diagnostic()?;
        let (position, leaf, ommers) = frontier.into_parts();
        let position: u64 = position.into();
        let leaf: [u8; 32] = leaf.to_bytes();
        let ommers: Vec<[u8; 32]> = ommers.into_iter().map(|ommer| ommer.to_bytes()).collect();
        let ommers_bytes: Vec<u8> = bincode::serialize(&ommers).into_diagnostic()?;
        tx.execute(
            "INSERT INTO frontier (position, leaf, ommers) VALUES (?1, ?2, ?3)",
            (position, leaf, ommers_bytes),
        )
        .into_diagnostic()?;
        Ok(())
    }

    fn insert_nullifier(tx: &rusqlite::Transaction, nullifier: &Nullifier) -> miette::Result<()> {
        tx.execute(
            "INSERT INTO nullifiers (nullifier) VALUES (?1)",
            [nullifier.to_bytes()],
        )
        .into_diagnostic()?;
        Ok(())
    }

    fn nullifier_exists(tx: &rusqlite::Transaction, nullifier: &Nullifier) -> miette::Result<bool> {
        let nullifier = nullifier.to_bytes();
        let nullifier_exists = match tx.query_row(
            "SELECT nullifier FROM nullifiers WHERE nullifier = ?1",
            [nullifier],
            |row| {
                let nullifier: Vec<u8> = row.get(0)?;
                Ok(nullifier)
            },
        ) {
            Ok(_) => true,
            Err(rusqlite::Error::QueryReturnedNoRows) => false,
            Err(err) => {
                return Err(err).into_diagnostic();
            }
        };
        Ok(nullifier_exists)
    }

    fn store_block(
        tx: &rusqlite::Transaction,
        anchor: &Anchor,
        block: &Block,
    ) -> miette::Result<()> {
        let block_bytes = bincode::serialize(block).into_diagnostic()?;
        tx.execute(
            "INSERT INTO blocks (anchor, block) VALUES (?1, ?2)",
            (anchor.to_bytes(), block_bytes),
        )
        .into_diagnostic()?;
        Ok(())
    }

    fn clear_transactions(tx: &rusqlite::Transaction) -> miette::Result<()> {
        tx.execute("DELETE FROM transactions", [])
            .into_diagnostic()?;
        Ok(())
    }

    pub fn validate_transaction(
        tx: &rusqlite::Transaction,
        transaction: &crate::types::Transaction,
    ) -> miette::Result<u64> {
        let nullifiers = transaction.nullifiers();
        for nullifier in &nullifiers {
            if Self::nullifier_exists(&tx, nullifier)? {
                return Err(miette!("nullifier exists, note is already spent"));
            }
        }
        let bundle = {
            let anchor = Self::get_bundle_anchor(tx)?;
            transaction.to_bundle(anchor)
        };

        let mut value_in = 0;
        for input in &transaction.inputs {
            let value: i64 = tx
                .query_row("SELECT value FROM utxos WHERE id = ?1", [input], |row| {
                    row.get(0)
                })
                .into_diagnostic()?;
            value_in += value;
        }

        let mut value_out = 0;
        for output in &transaction.outputs {
            value_out += output.value as i64;
        }

        /*
        A positive Orchard balancing value takes value from the Orchard transaction value pool and
        adds it to the transparent transaction value pool. A negative Orchard balancing value does
        the reverse. As a result, positive value_balance_orchard is treated like an input to the
        transparent transaction value pool, whereas negative value_balance_orchard is treated like
        an output from that pool.
        */

        let value_balance_orchard = transaction.value_balance_orchard;

        let fee = value_in + value_balance_orchard - value_out;
        if fee < 0 {
            return Err(miette!("transaction fee is negative"));
        }

        Ok(fee as u64)
    }

    fn connect_block(tx: &rusqlite::Transaction, block: &Block) -> miette::Result<Anchor> {
        for transaction in &block.transactions {
            Self::validate_transaction(tx, transaction)?;
        }
        // TODO: Validate zkSNARK, authorizing signature, binding signature
        let nullifiers = block.nullifiers();
        for nullifier in &nullifiers {
            // If the same note is spent in the same block this will fail.
            if Self::nullifier_exists(&tx, nullifier)? {
                return Err(miette!("nullifier exists, note is already spent"));
            }
            Self::insert_nullifier(&tx, nullifier)?;
        }
        let commitments = block.extracted_note_commitments();
        let frontier = Self::get_frontier(&tx)?;
        let anchor: Anchor = match frontier {
            Some(mut frontier) => {
                if commitments.is_empty() {
                    Anchor::empty_tree()
                } else {
                    for cmx in &commitments {
                        let leaf = MerkleHashOrchard::from_cmx(cmx);
                        frontier.append(leaf);
                    }
                    let anchor = frontier.root(None);
                    Self::update_frontier(&tx, frontier)?;
                    anchor.into()
                }
            }
            None => {
                if commitments.is_empty() {
                    Anchor::empty_tree()
                } else {
                    let cmx = &commitments[0];
                    let leaf = MerkleHashOrchard::from_cmx(cmx);
                    let mut frontier = NonEmptyFrontier::new(leaf);
                    for cmx in &commitments[1..] {
                        let leaf = MerkleHashOrchard::from_cmx(cmx);
                        frontier.append(leaf);
                    }
                    let anchor = frontier.root(None);
                    Self::update_frontier(&tx, frontier)?;
                    anchor.into()
                }
            }
        };
        let anchor = Anchor::from(anchor);
        Ok(anchor)
    }

    pub fn mine(&mut self) -> miette::Result<()> {
        let tx = self.conn.transaction().into_diagnostic()?;
        let transactions = Self::get_transactions(&tx)?;
        if transactions.len() == 0 {
            return Ok(());
        }
        let block = Block { transactions };
        let anchor = Self::connect_block(&tx, &block)?;
        Self::store_block(&tx, &anchor, &block)?;
        Self::clear_transactions(&tx)?;
        tx.commit().into_diagnostic()?;
        Ok(())
    }

    fn generate_seed(tx: &rusqlite::Transaction) -> miette::Result<()> {
        let mnemonic = Mnemonic::new(bip39::MnemonicType::Words12, bip39::Language::English);
        let phrase = mnemonic.phrase().to_string();
        tx.execute("INSERT INTO wallet_seed (phrase) VALUES (?1)", [phrase])
            .into_diagnostic()?;
        Ok(())
    }

    pub fn get_mnemonic(tx: &rusqlite::Transaction) -> miette::Result<Mnemonic> {
        let phrase: String = tx
            .query_row("SELECT phrase FROM wallet_seed", [], |row| row.get(0))
            .into_diagnostic()?;
        let mnemonic =
            Mnemonic::from_phrase(&phrase, bip39::Language::English).into_diagnostic()?;
        Ok(mnemonic)
    }

    pub fn get_sk(tx: &rusqlite::Transaction) -> miette::Result<orchard::keys::SpendingKey> {
        let mnemonic = Self::get_mnemonic(tx)?;
        let seed = Seed::new(&mnemonic, "");
        let seed_bytes = seed.as_bytes();
        let sk = orchard::keys::SpendingKey::from_zip32_seed(seed_bytes, 0, AccountId::ZERO)
            .expect("couldn't derive spending key from seed");
        Ok(sk)
    }

    pub fn get_new_address(&mut self) -> miette::Result<Address> {
        let tx = self.conn.transaction().into_diagnostic()?;
        let sk = Self::get_sk(&tx)?;

        let index: u32 = match tx.query_row(
            "SELECT id FROM addresses ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        ) {
            Ok(index) => index,
            Err(rusqlite::Error::QueryReturnedNoRows) => 0,
            Err(err) => return Err(err).into_diagnostic(),
        };

        let fvk = orchard::keys::FullViewingKey::from(&sk);
        let address = fvk.address_at(index + 1, zip32::Scope::External);

        tx.execute(
            "INSERT INTO addresses (address) VALUES (?)",
            [address.to_raw_address_bytes()],
        )
        .into_diagnostic()?;
        tx.commit().into_diagnostic()?;

        Ok(address)
    }

    pub fn get_total_transparent_value(&self) -> miette::Result<u64> {
        let total_value: u64 =
            match self
                .conn
                .query_row("SELECT SUM(value) FROM utxos", [], |row| row.get(0))
            {
                Ok(total_value) => total_value,
                Err(rusqlite::Error::InvalidColumnType(..)) => 0,
                Err(err) => return Err(err).into_diagnostic(),
            };
        Ok(total_value)
    }

    pub fn get_total_shielded_value(&self) -> miette::Result<u64> {
        let total_value: u64 =
            match self
                .conn
                .query_row("SELECT SUM(value) FROM notes", [], |row| row.get(0))
            {
                Ok(total_value) => total_value,
                Err(rusqlite::Error::InvalidColumnType(..)) => 0,
                Err(err) => return Err(err).into_diagnostic(),
            };
        Ok(total_value)
    }

    pub fn conjure_utxo(&self, value: u64) -> miette::Result<()> {
        self.conn
            .execute("INSERT INTO utxos (value) VALUES (?1)", [value])
            .into_diagnostic()?;
        Ok(())
    }

    pub fn get_utxos(&self) -> miette::Result<Vec<(u32, u64)>> {
        let mut statement = self
            .conn
            .prepare("SELECT id, value FROM utxos")
            .into_diagnostic()?;
        let utxos: Vec<(u32, u64)> = statement
            .query_map([], |row| {
                let id = row.get(0)?;
                let value = row.get(1)?;
                Ok((id, value))
            })
            .into_diagnostic()?
            .collect::<Result<Vec<_>, _>>()
            .into_diagnostic()?;
        Ok(utxos)
    }
}

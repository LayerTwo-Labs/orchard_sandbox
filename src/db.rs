use crate::types::{Block, Output};
use bip39::{Mnemonic, Seed};
use incrementalmerkletree::{
    frontier::{CommitmentTree, Frontier, NonEmptyFrontier},
    witness::IncrementalWitness,
    Level, Position,
};
use miette::{miette, IntoDiagnostic};
use orchard::{
    builder::BundleType,
    bundle::Flags,
    note::{ExtractedNoteCommitment, Nullifier, RandomSeed, Rho},
    tree::MerkleHashOrchard,
    value::NoteValue,
    Address, Anchor, Note,
};
use rand::SeedableRng;
use rusqlite::Connection;
use rusqlite_migration::{Migrations, M};
use zip32::AccountId;

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
                    witness BLOB NOT NULL

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
                "CREATE TABLE transactions(
                    id INTEGER PRIMARY KEY,
                    tx BLOB NOT NULL
            );",
            ),
            M::up(
                "CREATE TABLE blocks(
                    id INTEGER PRIMARY KEY,
                    fee INTEGER NOT NULL,
                    frontier BLOB,
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
            "SELECT frontier FROM blocks ORDER BY id DESC LIMIT 1 OFFSET 3",
            [],
            |row| {
                let frontier_bytes: Option<Vec<u8>> = row.get(0)?;
                Ok(frontier_bytes)
            },
        ) {
            Ok(frontier_bytes) => {
                if let Some(frontier_bytes) = frontier_bytes {
                    let (position, leaf, ommers): (u64, MerkleHashOrchard, Vec<MerkleHashOrchard>) =
                        bincode::deserialize(&frontier_bytes).into_diagnostic()?;
                    let position = Position::from(position);
                    let frontier = NonEmptyFrontier::from_parts(position, leaf, ommers)
                        .expect("failed to construct frontier from parts");
                    let anchor: Anchor = frontier.root(Some(Level::from(32))).into();
                    anchor
                } else {
                    Anchor::empty_tree()
                }
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Anchor::empty_tree(),
            Err(err) => return Err(err).into_diagnostic(),
        };
        Ok(anchor)
    }

    pub fn get_note(
        tx: &rusqlite::Transaction,
        note_id: u32,
    ) -> miette::Result<(Note, orchard::tree::MerklePath)> {
        let (recipient, value, rho, rseed, witness) = tx
            .query_row(
                "SELECT recipient, value, rho, rseed, witness FROM notes WHERE id = ?1",
                [note_id],
                |row| {
                    let recipient: Vec<u8> = row.get(0)?;
                    let value: u64 = row.get(1)?;
                    let rho: Vec<u8> = row.get(2)?;
                    let rseed: Vec<u8> = row.get(3)?;
                    let witness: Vec<u8> = row.get(4)?;
                    Ok((recipient, value, rho, rseed, witness))
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
            bincode::deserialize(&witness).into_diagnostic()?;
        let witness = orchard::tree::MerklePath::from_parts(position, auth_path);
        let note = Note::from_parts(recipient, value, rho, rseed)
            .expect("subtle error, failed to construct note from parts");
        Ok((note, witness))
    }

    pub fn clear_transaction(&mut self) -> miette::Result<()> {
        let tx = self.conn.transaction().into_diagnostic()?;
        tx.execute("DELETE FROM inputs", []).into_diagnostic()?;
        tx.execute("DELETE FROM outputs", []).into_diagnostic()?;
        tx.execute("DELETE FROM shielded_inputs", [])
            .into_diagnostic()?;
        tx.execute("DELETE FROM shielded_outputs", [])
            .into_diagnostic()?;
        tx.commit().into_diagnostic()?;
        Ok(())
    }

    pub fn submit_transaction(&mut self) -> miette::Result<()> {
        let tx = self.conn.transaction().into_diagnostic()?;
        let anchor: Anchor = Self::get_bundle_anchor(&tx)?;
        dbg!(&anchor);
        let mut builder = orchard::builder::Builder::new(
            BundleType::Transactional {
                flags: Flags::ENABLED,
                bundle_required: false,
            },
            anchor,
        );
        let shielded_inputs = Self::get_shielded_inputs(&tx)?;
        let (_note, one_witness) = Self::get_note(&tx, shielded_inputs[0])?;
        for note_id in shielded_inputs {
            let (note, witness) = Self::get_note(&tx, note_id)?;
            dbg!(note_id, &witness.root(note.commitment().into()));
            dbg!(note_id, &one_witness.root(note.commitment().into()));
            let sk = Self::get_sk(&tx)?;
            let fvk = orchard::keys::FullViewingKey::from(&sk);
            println!("here");
            let err = builder.add_spend(fvk, note, witness);
            dbg!(&err);
            // err.into_diagnostic()?;
        }
        panic!();
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
        let bundle = builder.build::<i64>(rng).into_diagnostic()?;

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

    fn get_last_frontier(
        tx: &rusqlite::Transaction,
    ) -> miette::Result<Option<NonEmptyFrontier<MerkleHashOrchard>>> {
        let frontier: Vec<u8> = match tx.query_row(
            "SELECT frontier FROM blocks ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        ) {
            Ok(Some(frontier)) => frontier,
            Ok(None) => {
                return Ok(None);
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                return Ok(None);
            }
            Err(err) => {
                return Err(err).into_diagnostic();
            }
        };

        let (position, leaf, ommers): (u64, MerkleHashOrchard, Vec<MerkleHashOrchard>) =
            bincode::deserialize(&frontier).into_diagnostic()?;
        let position = Position::from(position);
        let frontier = NonEmptyFrontier::from_parts(position, leaf, ommers)
            .expect("failed to reconstruct frontier");
        Ok(Some(frontier))
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
        frontier: Option<NonEmptyFrontier<MerkleHashOrchard>>,
        fee: u64,
        block: &Block,
    ) -> miette::Result<()> {
        let frontier_bytes = match frontier {
            Some(frontier) => {
                let (position, leaf, ommers) = frontier.into_parts();
                let position: u64 = position.into();
                let frontier_bytes =
                    bincode::serialize(&(position, leaf, ommers)).into_diagnostic()?;
                Some(frontier_bytes)
            }
            None => None,
        };
        let block_bytes = bincode::serialize(block).into_diagnostic()?;
        tx.execute(
            "INSERT INTO blocks (fee, frontier, block) VALUES (?1, ?2, ?3)",
            (fee, frontier_bytes, block_bytes),
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
        let _bundle = {
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

    fn connect_block(
        tx: &rusqlite::Transaction,
        block: &Block,
    ) -> miette::Result<(Option<NonEmptyFrontier<MerkleHashOrchard>>, u64)> {
        // Updating transparent state.
        let mut total_fee = 0;
        for transaction in &block.transactions {
            let fee = Self::validate_transaction(tx, transaction)?;
            total_fee += fee;
            for input in &transaction.inputs {
                tx.execute("DELETE FROM utxos WHERE id = ?1", [input])
                    .into_diagnostic()?;
            }
            for output in &transaction.outputs {
                tx.execute("INSERT INTO utxos (value) VALUES (?1)", [output.value])
                    .into_diagnostic()?;
            }
        }

        // Storing notes and corresponding merkle proofs.
        {
            let anchor = Self::get_bundle_anchor(tx)?;
            let sk = Self::get_sk(tx)?;
            let fvk = orchard::keys::FullViewingKey::from(&sk);
            let ivk = fvk.to_ivk(zip32::Scope::External);
            let keys = [ivk];
            let mut notes = vec![];
            for transaction in &block.transactions {
                let bundle = transaction.to_bundle(anchor);
                if let Some(bundle) = bundle {
                    for (_action_index, _ivk, note, _address, _memo) in
                        bundle.decrypt_outputs_with_keys(&keys)
                    {
                        notes.push(note);
                    }
                }
            }
            let mut witnesses = vec![];
            if notes.len() > 0 {
                let mut frontier = {
                    let frontier = Self::get_last_frontier(tx)?;
                    match frontier {
                        Some(frontier) => frontier,
                        None => {
                            let note = notes[0];
                            notes.remove(0);
                            let cmx = ExtractedNoteCommitment::from(note.commitment());
                            let leaf = MerkleHashOrchard::from_cmx(&cmx);
                            let frontier = NonEmptyFrontier::new(leaf);

                            let witness = {
                                let frontier: Frontier<MerkleHashOrchard, 32> =
                                    Frontier::try_from(frontier.clone()).map_err(|_err| {
                                        miette!("failed to convert NonEmptyFrontier to Frontier")
                                    })?;
                                let tree: CommitmentTree<MerkleHashOrchard, 32> =
                                    CommitmentTree::from_frontier(&frontier);
                                IncrementalWitness::from_tree(tree)
                            };
                            witnesses.push((witness, note));
                            frontier
                        }
                    }
                };

                for note in notes {
                    let cmx = ExtractedNoteCommitment::from(note.commitment());
                    let leaf = MerkleHashOrchard::from_cmx(&cmx);
                    frontier.append(leaf);
                    for (witness, _note) in witnesses.iter_mut() {
                        witness.append(leaf).expect("tree is full");
                    }
                    let witness = {
                        let frontier: Frontier<MerkleHashOrchard, 32> =
                            Frontier::try_from(frontier.clone()).map_err(|_err| {
                                miette!("failed to convert NonEmptyFrontier to Frontier")
                            })?;
                        let tree: CommitmentTree<MerkleHashOrchard, 32> =
                            CommitmentTree::from_frontier(&frontier);
                        IncrementalWitness::from_tree(tree)
                    };
                    witnesses.push((witness, note));
                }
            }

            for (witness, note) in witnesses {
                Self::store_note(tx, &note, &witness)?;
            }
        }

        // Updating Orchard state.
        let frontier = {
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
            let last_frontier = Self::get_last_frontier(&tx)?;
            let frontier: Option<NonEmptyFrontier<MerkleHashOrchard>> = match last_frontier {
                Some(mut frontier) => {
                    if !commitments.is_empty() {
                        for cmx in &commitments {
                            let leaf = MerkleHashOrchard::from_cmx(cmx);
                            frontier.append(leaf);
                        }
                    }
                    Some(frontier)
                }
                None => {
                    if commitments.is_empty() {
                        None
                    } else {
                        let cmx = &commitments[0];
                        let leaf = MerkleHashOrchard::from_cmx(cmx);
                        let mut frontier = NonEmptyFrontier::new(leaf);
                        for cmx in &commitments[1..] {
                            let leaf = MerkleHashOrchard::from_cmx(cmx);
                            frontier.append(leaf);
                        }
                        Some(frontier)
                    }
                }
            };
            frontier
        };

        Ok((frontier, total_fee))
    }

    pub fn mine(&mut self) -> miette::Result<()> {
        let tx = self.conn.transaction().into_diagnostic()?;
        let transactions = Self::get_transactions(&tx)?;
        if transactions.len() == 0 {
            return Ok(());
        }
        let block = Block { transactions };
        let (frontier, total_fee) = Self::connect_block(&tx, &block)?;
        Self::store_block(&tx, frontier, total_fee, &block)?;
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

    pub fn get_wallet_notes(
        &self,
    ) -> miette::Result<Vec<(u32, Note, IncrementalWitness<MerkleHashOrchard, 32>)>> {
        let mut statement = self
            .conn
            .prepare("SELECT id, recipient, value, rho, rseed, witness FROM notes")
            .into_diagnostic()?;
        let notes: Vec<(u32, Note, IncrementalWitness<MerkleHashOrchard, 32>)> = statement
            .query_map([], |row| {
                let id = row.get(0)?;
                let note = {
                    let recipient: Vec<u8> = row.get(1)?;
                    let recipient: [u8; 43] =
                        recipient.try_into().expect("wrong shielded address length");
                    let recipient = Address::from_raw_address_bytes(&recipient)
                        .expect("subtle error, failed to convert bytes to shielded address");
                    let value = row.get(2)?;
                    let value = NoteValue::from_raw(value);
                    let rho: Vec<u8> = row.get(3)?;
                    let rho: [u8; 32] = rho.try_into().expect("wrong rho length");
                    let rho = Rho::from_bytes(&rho)
                        .expect("subtle error, failed to convert bytes to rho");
                    let rseed: Vec<u8> = row.get(4)?;
                    let rseed: [u8; 32] = rseed.try_into().expect("wrong rseed length");
                    let rseed = RandomSeed::from_bytes(rseed, &rho)
                        .expect("subtle error, failed to convert bytes to rseed");
                    Note::from_parts(recipient, value, rho, rseed)
                        .expect("subtle error, failed to construct note")
                };
                let witness: Vec<u8> = row.get(5)?;
                let witness = deserialize_incremental_witness(&witness)
                    .expect("failed to deserialize incremental witness");
                Ok((id, note, witness))
            })
            .into_diagnostic()?
            .collect::<Result<Vec<_>, _>>()
            .into_diagnostic()?;
        Ok(notes)
    }

    pub fn get_utxo_value(tx: &rusqlite::Transaction, id: u32) -> miette::Result<u64> {
        let value = tx
            .query_row("SELECT value FROM utxos WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .into_diagnostic()?;
        Ok(value)
    }

    pub fn store_note(
        tx: &rusqlite::Transaction,
        note: &Note,
        witness: &IncrementalWitness<MerkleHashOrchard, 32>,
    ) -> miette::Result<()> {
        let recipient = note.recipient().to_raw_address_bytes();
        let value = note.value().inner();
        let rho = note.rho().to_bytes();
        let rseed = note.rseed().as_bytes();
        let witness_bytes = serialize_incremental_witness(witness)?;
        tx.execute(
            "INSERT INTO notes (recipient, value, rho, rseed, witness) VALUES (?1, ?2, ?3, ?4, ?5)",
            (&recipient, &value, &rho, &rseed, &witness_bytes),
        )
        .into_diagnostic()?;
        Ok(())
    }

    pub fn get_notes(tx: &rusqlite::Transaction, block: &Block) -> miette::Result<Vec<Note>> {
        let anchor = Db::get_bundle_anchor(tx)?;
        let sk = Db::get_sk(tx)?;
        let fvk = orchard::keys::FullViewingKey::from(&sk);
        let ivk = fvk.to_ivk(zip32::Scope::External);
        let keys = [ivk];
        let mut decrypted_notes = vec![];
        for transaction in &block.transactions {
            if let Some(bundle) = transaction.to_bundle(anchor) {
                let notes = bundle.decrypt_outputs_with_keys(&keys);
                for (_action_index, _ivk, note, _address, _memo) in &notes {
                    decrypted_notes.push(*note);
                }
            }
        }
        Ok(decrypted_notes)
    }
}

fn deserialize_incremental_witness(
    bytes: &[u8],
) -> miette::Result<IncrementalWitness<MerkleHashOrchard, 32>> {
    let (tree, filled, cursor): (
        (
            Option<MerkleHashOrchard>,
            Option<MerkleHashOrchard>,
            Vec<Option<MerkleHashOrchard>>,
        ),
        Vec<MerkleHashOrchard>,
        Option<(
            Option<MerkleHashOrchard>,
            Option<MerkleHashOrchard>,
            Vec<Option<MerkleHashOrchard>>,
        )>,
    ) = bincode::deserialize(bytes).into_diagnostic()?;
    let tree: CommitmentTree<MerkleHashOrchard, 32> = {
        let (left, right, parents) = tree;
        CommitmentTree::from_parts(left, right, parents)
            .expect("failed to construct commitment tree from parts")
    };
    let cursor: Option<CommitmentTree<MerkleHashOrchard, 32>> =
        cursor.map(|(left, right, parents)| {
            CommitmentTree::from_parts(left, right, parents)
                .expect("failed to construct commitment tree from parts")
        });
    let witness: IncrementalWitness<MerkleHashOrchard, 32> =
        IncrementalWitness::from_parts(tree, filled, cursor);
    Ok(witness)
}

fn serialize_incremental_witness(
    witness: &IncrementalWitness<MerkleHashOrchard, 32>,
) -> miette::Result<Vec<u8>> {
    let tree = witness.tree();
    let tree = (tree.left(), tree.right(), tree.parents());
    let filled = witness.filled();
    let cursor = witness.cursor().clone();
    let cursor = cursor.map(|cursor| (*cursor.left(), *cursor.right(), cursor.parents().clone()));
    let parts = (tree, filled, cursor);
    let bytes = bincode::serialize(&parts).into_diagnostic()?;
    Ok(bytes)
}

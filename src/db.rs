use incrementalmerkletree::{
    frontier::{Frontier, NonEmptyFrontier},
    Position,
};
use miette::{miette, IntoDiagnostic};
use orchard::{
    builder::BundleType,
    bundle::Flags,
    keys::{Diversifier, FullViewingKey, SpendingKey},
    note::Nullifier,
    tree::MerkleHashOrchard,
    value::NoteValue,
    Action, Address, Anchor,
};
use rand::SeedableRng;
use rusqlite::Connection;
use rusqlite_migration::{Migrations, M};
use zip32::AccountId;

use crate::types::Block;

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn new() -> miette::Result<Self> {
        // 1️⃣ Define migrations
        let migrations = Migrations::new(vec![
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
                    note_id INTEGER NOT NULL,
                    FOREIGN KEY(note_id) REFERENCES notes(id)
            );",
            ),
            M::up(
                "CREATE TABLE outputs(
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

        Ok(Db { conn })
    }

    pub fn get_outputs(&self) -> miette::Result<Vec<(Vec<u8>, u64)>> {
        let mut statement = self
            .conn
            .prepare("SELECT recipient, value FROM outputs")
            .into_diagnostic()?;
        let outputs: Vec<_> = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .into_diagnostic()?
            .collect::<Result<Vec<_>, _>>()
            .into_diagnostic()?;
        Ok(outputs)
    }

    pub fn create_note(&self, recipient: Option<String>, value: i64) -> miette::Result<()> {
        let recipient = match recipient {
            Some(recipient) => {
                let recipient = bs58::decode(recipient).into_vec().into_diagnostic()?;
                let recipient: [u8; 43] = recipient
                    .try_into()
                    .map_err(|_err| miette!("wrong address length"))?;
                recipient
            }
            None => {
                let seed = [0; 32];
                let sk = SpendingKey::from_zip32_seed(&seed, 0, AccountId::ZERO).unwrap();
                let full_viewing_key = FullViewingKey::from(&sk);
                let diversifier = Diversifier::from_bytes([0; 11]);
                let recipient =
                    full_viewing_key.address(diversifier, orchard::keys::Scope::Internal);
                recipient.to_raw_address_bytes()
            }
        };
        self.conn
            .execute(
                "INSERT INTO outputs (recipient, value) VALUES (?1, ?2)",
                (recipient, value),
            )
            .into_diagnostic()?;
        Ok(())
    }

    pub fn submit_transaction(&self) -> miette::Result<()> {
        let outputs = self.get_outputs()?;
        // let anchor: Anchor = frontier.root().into();
        let anchor: Anchor = Anchor::empty_tree();
        let mut builder = orchard::builder::Builder::new(
            BundleType::Transactional {
                flags: Flags::ENABLED,
                bundle_required: false,
            },
            anchor,
        );
        for (recipient, value) in outputs {
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
        let (bundle, bundle_metadata) = builder.build::<i64>(rng).into_diagnostic()?.unwrap();
        println!("after bundle construction");

        dbg!(bundle.value_balance());

        let transaction = crate::types::Transaction::from_bundle(&bundle);

        let transaction_bytes = bincode::serialize(&transaction).into_diagnostic()?;

        self.conn
            .execute(
                "INSERT INTO transactions (tx) VALUES (?1)",
                (&transaction_bytes,),
            )
            .into_diagnostic()?;

        self.conn
            .execute("DELETE FROM inputs", [])
            .into_diagnostic()?;
        self.conn
            .execute("DELETE FROM outputs", [])
            .into_diagnostic()?;

        dbg!(hex::encode(&transaction_bytes));

        Ok(())
    }

    pub fn get_transactions(&self) -> miette::Result<Vec<crate::types::Transaction>> {
        let mut statement = self
            .conn
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

    pub fn get_frontier(&self) -> miette::Result<Option<NonEmptyFrontier<MerkleHashOrchard>>> {
        let (position, leaf, ommers): (u64, Vec<u8>, Vec<u8>) =
            match self
                .conn
                .query_row("SELECT position, leaf, ommers FROM frontier", [], |row| {
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

    pub fn update_frontier(
        &self,
        frontier: NonEmptyFrontier<MerkleHashOrchard>,
    ) -> miette::Result<()> {
        self.conn
            .execute("DELETE FROM frontier", [])
            .into_diagnostic()?;
        let (position, leaf, ommers) = frontier.into_parts();
        let position: u64 = position.into();
        let leaf: [u8; 32] = leaf.to_bytes();
        let ommers: Vec<[u8; 32]> = ommers.into_iter().map(|ommer| ommer.to_bytes()).collect();
        let ommers_bytes: Vec<u8> = bincode::serialize(&ommers).into_diagnostic()?;
        self.conn
            .execute(
                "INSERT INTO frontier (position, leaf, ommers) VALUES (?1, ?2, ?3)",
                (position, leaf, ommers_bytes),
            )
            .into_diagnostic()?;
        todo!();
    }

    pub fn insert_nullifier(&self, nullifier: &Nullifier) -> miette::Result<()> {
        self.conn
            .execute(
                "INSERT INTO nullifiers (nullifier) VALUES (?1)",
                [nullifier.to_bytes()],
            )
            .into_diagnostic()?;
        Ok(())
    }

    pub fn nullifier_exists(&self, nullifier: &Nullifier) -> miette::Result<bool> {
        let nullifier_exists =
            match self
                .conn
                .query_row("SELECT nullifier FROM nullifiers", [], |row| {
                    let nullifier: Vec<u8> = row.get(0)?;
                    Ok(nullifier)
                }) {
                Ok(_) => true,
                Err(rusqlite::Error::QueryReturnedNoRows) => false,
                Err(err) => {
                    return Err(err).into_diagnostic();
                }
            };
        Ok(nullifier_exists)
    }

    pub fn store_block(&self, anchor: &Anchor, block: &Block) -> miette::Result<()> {
        let block_bytes = bincode::serialize(block).into_diagnostic()?;
        self.conn
            .execute(
                "INSERT INTO blocks (anchor, block) VALUES (?1, ?2)",
                (anchor.to_bytes(), block_bytes),
            )
            .into_diagnostic()?;
        Ok(())
    }

    pub fn clear_transactions(&self) -> miette::Result<()> {
        self.conn
            .execute("DELETE FROM transactions", [])
            .into_diagnostic()?;
        Ok(())
    }

    pub fn connect_block(&self, block: &Block) -> miette::Result<Anchor> {
        // TODO: Validate zkSNARK, authorizing signature, binding signature
        let nullifiers = block.nullifiers();
        for nullifier in &nullifiers {
            if self.nullifier_exists(nullifier)? {
                return Err(miette!("nullifier exists, note is already spent"));
            }
            self.insert_nullifier(nullifier)?;
        }

        let commitments = block.extracted_note_commitments();
        let frontier = self.get_frontier()?;
        let anchor: MerkleHashOrchard = match frontier {
            Some(mut frontier) => {
                for cmx in &commitments {
                    let leaf = MerkleHashOrchard::from_cmx(cmx);
                    frontier.append(leaf);
                }
                let anchor = frontier.root(None);
                self.update_frontier(frontier)?;
                anchor
            }
            None => {
                let cmx = &commitments[0];
                let leaf = MerkleHashOrchard::from_cmx(cmx);
                let mut frontier = NonEmptyFrontier::new(leaf);
                for cmx in &commitments[1..] {
                    let leaf = MerkleHashOrchard::from_cmx(cmx);
                    frontier.append(leaf);
                }
                let anchor = frontier.root(None);
                self.update_frontier(frontier)?;
                anchor
            }
        };
        let anchor = Anchor::from(anchor);
        Ok(anchor)
    }

    pub fn mine(&self) -> miette::Result<()> {
        let transactions = self.get_transactions()?;
        if transactions.len() == 0 {
            return Ok(());
        }
        let block = Block { transactions };

        let anchor = self.connect_block(&block)?;

        self.store_block(&anchor, &block)?;
        self.clear_transactions()?;

        Ok(())
    }
}

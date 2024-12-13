use incrementalmerkletree::frontier::Frontier;
use miette::{miette, IntoDiagnostic};
use orchard::{
    builder::BundleType,
    bundle::Flags,
    keys::{Diversifier, FullViewingKey, SpendingKey},
    tree::MerkleHashOrchard,
    value::NoteValue,
    Action, Address, Anchor, Bundle,
};
use rand::SeedableRng;
use rusqlite::Connection;
use rusqlite_migration::{Migrations, M};
use zip32::AccountId;

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
                    transactions BLOB NOT NULL
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
            // In the future, add more migrations here:
            //M::up("ALTER TABLE friend ADD COLUMN email TEXT;"),
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

        let transaction: crate::types::Transaction = bundle.into();

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

    pub fn get_last_anchor(&self) -> miette::Result<Anchor> {
        let mut statement = self
            .conn
            .prepare("SELECT anchor FROM blocks WHERE id = (SELECT MAX(id) FROM blocks)")
            .into_diagnostic()?;
        let anchor: Vec<Vec<u8>> = statement
            .query_map([], |row| Ok(row.get(0)?))
            .into_diagnostic()?
            .collect::<Result<Vec<_>, _>>()
            .into_diagnostic()?;
        if anchor.len() == 0 {
            return Ok(Anchor::empty_tree());
        }
        let anchor: [u8; 32] = anchor[0]
            .clone()
            .try_into()
            .map_err(|_err| miette!("anchor length is wrong"))?;
        let anchor = Anchor::from_bytes(anchor).unwrap();
        Ok(anchor)
    }

    pub fn mine(&self) -> miette::Result<()> {
        let anchor = self.get_last_anchor()?;
        dbg!(&anchor);
        let transactions = self.get_transactions()?;
        if transactions.len() == 0 {
            return Ok(());
        }
        let mut commitments = vec![];
        for transaction in &transactions {
            for action in &transaction.actions {
                let action = Action::from(action);
                let cmx = action.cmx().clone();
                commitments.push(cmx);
            }
        }
        dbg!(commitments);

        let mut frontier = Frontier::<MerkleHashOrchard, NOTE_COMMITMENT_TREE_DEPTH>::empty();

        let transactions = bincode::serialize(&transactions).into_diagnostic()?;

        /*
        self.conn
            .execute(
                "INSERT INTO blocks (anchor, transactions) VALUES (?1, ?2)",
                (anchor.to_bytes(), &transactions),
            )
            .into_diagnostic()?;
        self.conn
            .execute("DELETE FROM transactions", [])
            .into_diagnostic()?;
        */
        Ok(())
    }
}
const NOTE_COMMITMENT_TREE_DEPTH: u8 = 32;

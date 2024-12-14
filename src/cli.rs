use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Print out wallet data
    Wallet,
    /// Create a new transparent utxo in pending transaction
    CreateUtxo { value: u64 },
    /// Create spend an existing transparent utxo in pending transaction
    SpendUtxo { utxo_id: u32 },
    /// Create a new note in pending transaction
    CreateNote {
        value: u64,
        recipient: Option<String>,
    },
    /// Spend a note in pending transaction
    SpendNote { note_id: u32 },
    /// Submit pending transaction to mempool
    SubmitTxn,
    /// Mine a block
    Mine,
    /// Get wallet seed mnemonic 12 words
    GetMnemonic,
    /// Get new shielded address
    GetNewAddress,
    /// Get total transparent and shielded value
    ValuePools,
    /// Create a new UTXO out of thin air
    ConjureUtxo { value: u64 },
}

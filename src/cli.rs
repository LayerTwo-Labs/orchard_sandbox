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
    SpendUtxo { id: u32 },
    /// Create a new note in pending transaction
    CreateNote {
        value: i64,
        recipient: Option<String>,
    },
    /// Spend a note in pending transaction
    SpendNote { id: u32 },
    /// Submit pending transaction to mempool
    SubmitTxn,
    /// Mine a block
    Mine,
    /// Get wallet seed mnemonic 12 words
    GetMnemonic,
    /// Get new shielded address
    GetNewAddress,
}

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// Optional name to operate on
    pub name: Option<String>,

    /// Sets a custom config file
    #[arg(short, long, value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Turn debugging information on
    #[arg(short, long, action = clap::ArgAction::Count)]
    pub debug: u8,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Print out wallet data
    Wallet,
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
}

mod cli;
mod db;
mod types;

use clap::Parser as _;
use miette::IntoDiagnostic;

fn main() -> miette::Result<()> {
    let cli = cli::Cli::parse();
    let mut db = db::Db::new()?;
    match &cli.command {
        cli::Commands::Wallet => {
            let tx = db.conn.transaction().into_diagnostic()?;
            let outputs = db::Db::get_shielded_outputs(&tx)?;
            for (recipient, value) in outputs {
                let recipient = bs58::encode(recipient).with_check().into_string();
                println!("{recipient}: {value}");
            }
        }
        cli::Commands::CreateUtxo { value } => {
            db.create_utxo(*value)?;
        }
        cli::Commands::SpendUtxo { utxo_id } => {
            db.spend_utxo(*utxo_id)?;
        }
        cli::Commands::CreateNote { value, recipient } => {
            db.create_note(recipient.clone(), *value)?;
        }
        cli::Commands::SpendNote { note_id } => {
            db.spend_note(*note_id)?;
        }
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

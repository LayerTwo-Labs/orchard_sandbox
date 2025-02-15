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
            let mut transparent_value_in = 0;
            let mut transparent_value_out = 0;

            let tx = db.conn.transaction().into_diagnostic()?;
            let inputs = db::Db::get_inputs(&tx)?;
            println!("Inputs: ");
            for utxo_id in inputs {
                let value = db::Db::get_utxo_value(&tx, utxo_id)?;
                println!("utxo_id: {utxo_id} value: {value}");

                transparent_value_in += value;
            }
            let outputs = db::Db::get_outputs(&tx)?;

            println!();

            println!("Outputs: ");
            for output in outputs {
                println!("value: {}", output.value);

                transparent_value_out += output.value;
            }

            println!();

            println!("Transparent value in: {transparent_value_in}");
            println!("Transparent value out: {transparent_value_out}");

            println!();

            let mut shielded_value_in = 0;
            let mut shielded_value_out = 0;

            println!("Shielded inputs: ");
            let shielded_inputs = db::Db::get_shielded_inputs(&tx)?;
            for note_id in &shielded_inputs {
                let (note, _) = db::Db::get_note(&tx, *note_id)?;
                let value = note.value().inner();
                println!("note_id: {note_id} value: {value}");

                shielded_value_in += value;
            }

            let shielded_outputs = db::Db::get_shielded_outputs(&tx)?;

            println!();

            println!("Shielded outputs: ");
            for (recipient, value) in shielded_outputs {
                let recipient = bs58::encode(recipient).with_check().into_string();
                println!("recipient: {recipient}, value: {value}");

                shielded_value_out += value;
            }

            println!();

            println!("Sielded value in: {shielded_value_in}");
            println!("Sielded value out: {shielded_value_out}");

            let fee = transparent_value_in as i64 + shielded_value_in as i64
                - transparent_value_out as i64
                - shielded_value_out as i64;

            println!();

            println!("Transaction fee: {fee}");
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
        cli::Commands::ClearTxn => {
            db.clear_transaction()?;
        }
        cli::Commands::Mine => {
            db.mine()?;
        }
        cli::Commands::GetMnemonic => {
            let tx = db.conn.transaction().into_diagnostic()?;
            let mnemonic = db::Db::get_mnemonic(&tx)?;
            let phrase = mnemonic.phrase().to_string();
            println!("{phrase}");
        }
        cli::Commands::GetNewAddress => {
            let address = db.get_new_address()?;
            let address_bytes = address.to_raw_address_bytes();
            let address_string = bs58::encode(address_bytes).with_check().into_string();
            println!("{address_string}");
        }
        cli::Commands::ValuePools => {
            let total_transparent_value = db.get_total_transparent_value()?;
            println!("Total transparent value: {total_transparent_value}");

            let total_shielded_value = db.get_total_shielded_value()?;
            println!("Total shielded value: {total_shielded_value}");
        }
        cli::Commands::ConjureUtxo { value } => {
            db.conjure_utxo(*value)?;
        }
        cli::Commands::GetUtxos => {
            println!("transparent utxos: ");
            let utxos = db.get_utxos()?;
            for (id, value) in utxos {
                println!("id: {id} value: {value}");
            }
            println!();
            println!("shielded notes: ");
            let notes = db.get_wallet_notes()?;
            for (id, note, _witness) in notes {
                let recipient = note.recipient().to_raw_address_bytes();
                let recipient = bs58::encode(&recipient).with_check().into_string();
                let value = note.value().inner();
                println!("id: {id} recipient: {recipient} value: {value}");
            }
        }
    }
    Ok(())
}

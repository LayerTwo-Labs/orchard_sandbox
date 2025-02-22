use crate::address::Address;
use crate::error::{Error, Result};
use blake2::{Blake2b512, Digest};
use rusqlite::Transaction as SqlTransaction;
use serde::{Deserialize, Serialize};

/// Represents the type of transaction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransactionType {
    Deposit,
    Transparent,
    Shield,
    ShieldToShield,
    Deshield,
}

impl std::fmt::Display for TransactionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransactionType::Deposit => write!(f, "deposit"),
            TransactionType::Transparent => write!(f, "transparent"),
            TransactionType::Shield => write!(f, "shield"),
            TransactionType::ShieldToShield => write!(f, "shield_to_shield"),
            TransactionType::Deshield => write!(f, "deshield"),
        }
    }
}

/// Represents a transparent input
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransparentInput {
    pub output_id: Vec<u8>,
    pub address: Address,
    pub amount: u64,
    pub signature: Vec<u8>,  // Would be proper signature in production
}

/// Represents a transparent output
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransparentOutput {
    pub address: Address,
    pub amount: u64,
}

/// Represents a shielded note
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShieldedNote {
    pub commitment: Vec<u8>,
    pub ephemeral_key: Vec<u8>,
    pub amount: Vec<u8>,  // encrypted
    pub memo: Option<Vec<u8>>,
}

/// Represents a transaction in the system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    pub tx_type: TransactionType,
    pub transparent_inputs: Vec<TransparentInput>,
    pub transparent_outputs: Vec<TransparentOutput>,
    pub shielded_inputs: Vec<ShieldedNote>,
    pub shielded_outputs: Vec<ShieldedNote>,
    pub nullifiers: Vec<Vec<u8>>,
    pub proof: Option<Vec<u8>>,  // zk-SNARK proof for shielded transactions
}

impl Transaction {
    /// Create a new deposit transaction
    pub fn new_deposit(address: Address, amount: u64) -> Result<Self> {
        if !matches!(address.address_type, crate::address::AddressType::Transparent) {
            return Err(Error::invalid_transaction("Deposit must be to transparent address"));
        }

        Ok(Self {
            tx_type: TransactionType::Deposit,
            transparent_inputs: Vec::new(),
            transparent_outputs: vec![TransparentOutput { address, amount }],
            shielded_inputs: Vec::new(),
            shielded_outputs: Vec::new(),
            nullifiers: Vec::new(),
            proof: None,
        })
    }

    /// Create a new transparent transaction
    pub fn new_transparent(
        inputs: Vec<TransparentInput>,
        outputs: Vec<TransparentOutput>,
    ) -> Result<Self> {
        // Verify all addresses are transparent
        for input in &inputs {
            if !matches!(input.address.address_type, crate::address::AddressType::Transparent) {
                return Err(Error::invalid_transaction("All inputs must be transparent"));
            }
        }
        for output in &outputs {
            if !matches!(output.address.address_type, crate::address::AddressType::Transparent) {
                return Err(Error::invalid_transaction("All outputs must be transparent"));
            }
        }

        Ok(Self {
            tx_type: TransactionType::Transparent,
            transparent_inputs: inputs,
            transparent_outputs: outputs,
            shielded_inputs: Vec::new(),
            shielded_outputs: Vec::new(),
            nullifiers: Vec::new(),
            proof: None,
        })
    }

    /// Create a new shield transaction
    pub fn new_shield(
        input: TransparentInput,
        note: ShieldedNote,
    ) -> Result<Self> {
        if !matches!(input.address.address_type, crate::address::AddressType::Transparent) {
            return Err(Error::invalid_transaction("Shield input must be transparent"));
        }

        Ok(Self {
            tx_type: TransactionType::Shield,
            transparent_inputs: vec![input],
            transparent_outputs: Vec::new(),
            shielded_inputs: Vec::new(),
            shielded_outputs: vec![note],
            nullifiers: Vec::new(),  // Will be set when proof is generated
            proof: None,  // Will be set when proof is generated
        })
    }

    /// Create a new shield-to-shield transaction
    pub fn new_shield_to_shield(
        input_note: ShieldedNote,
        output_note: ShieldedNote,
        nullifier: Vec<u8>,
        proof: Vec<u8>,
    ) -> Result<Self> {
        Ok(Self {
            tx_type: TransactionType::ShieldToShield,
            transparent_inputs: Vec::new(),
            transparent_outputs: Vec::new(),
            shielded_inputs: vec![input_note],
            shielded_outputs: vec![output_note],
            nullifiers: vec![nullifier],
            proof: Some(proof),
        })
    }

    /// Create a new deshield transaction
    pub fn new_deshield(
        input_note: ShieldedNote,
        output: TransparentOutput,
        nullifier: Vec<u8>,
        proof: Vec<u8>,
    ) -> Result<Self> {
        if !matches!(output.address.address_type, crate::address::AddressType::Transparent) {
            return Err(Error::invalid_transaction("Deshield output must be transparent"));
        }

        Ok(Self {
            tx_type: TransactionType::Deshield,
            transparent_inputs: Vec::new(),
            transparent_outputs: vec![output],
            shielded_inputs: vec![input_note],
            shielded_outputs: Vec::new(),
            nullifiers: vec![nullifier],
            proof: Some(proof),
        })
    }

    /// Calculate the transaction hash
    pub fn calculate_hash(&self) -> Result<Vec<u8>> {
        let serialized = serde_json::to_vec(self)
            .map_err(|e| Error::serialization_error(e.to_string()))?;
        
        let mut hasher = Blake2b512::new();
        hasher.update(&serialized);
        Ok(hasher.finalize().to_vec())
    }

    /// Get the nullifiers used in this transaction
    pub fn get_nullifiers(&self) -> &[Vec<u8>] {
        &self.nullifiers
    }

    /// Validate the transaction
    pub fn validate(&self, db: &crate::database::Database) -> Result<bool> {
        match self.tx_type {
            TransactionType::Deposit => self.validate_deposit(),
            TransactionType::Transparent => self.validate_transparent(db),
            TransactionType::Shield => self.validate_shield(db),
            TransactionType::ShieldToShield => self.validate_shield_to_shield(db),
            TransactionType::Deshield => self.validate_deshield(db),
        }
    }

    /// Connect the transaction to the chain
    pub fn connect(&self, tx: &SqlTransaction, block_height: i64) -> Result<()> {
        // Insert transaction record
        let tx_hash = self.calculate_hash()?;
        tx.execute(
            "INSERT INTO transactions (tx_hash, block_height, tx_type, raw_data, proof_data)
             VALUES (?, ?, ?, ?, ?)",
            rusqlite::params![
                &tx_hash,
                block_height,
                self.tx_type.to_string(),
                serde_json::to_vec(self).map_err(|e| Error::serialization_error(e.to_string()))?,
                &self.proof,
            ],
        )?;

        // Process transparent outputs
        for output in &self.transparent_outputs {
            tx.execute(
                "INSERT INTO transparent_outputs (output_id, tx_hash, address, amount)
                 VALUES (?, ?, ?, ?)",
                rusqlite::params![
                    blake2b_hash(&tx_hash),  // Derive output ID from tx hash
                    &tx_hash,
                    serde_json::to_vec(&output.address).map_err(|e| Error::serialization_error(e.to_string()))?,
                    output.amount,
                ],
            )?;
        }

        // Process shielded notes
        for (i, note) in self.shielded_outputs.iter().enumerate() {
            tx.execute(
                "INSERT INTO shielded_notes
                 (note_commitment, ephemeral_key, amount, memo, tx_hash, block_height, merkle_position)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    &note.commitment,
                    &note.ephemeral_key,
                    &note.amount,
                    &note.memo,
                    &tx_hash,
                    block_height,
                    i as i64,  // Use index as position
                ],
            )?;
        }

        // Add nullifiers
        for nullifier in &self.nullifiers {
            tx.execute(
                "INSERT INTO nullifier_set (nullifier, block_height, tx_hash)
                 VALUES (?, ?, ?)",
                rusqlite::params![nullifier, block_height, &tx_hash],
            )?;
        }

        Ok(())
    }

    /// Disconnect the transaction from the chain
    pub fn disconnect(&self, tx: &SqlTransaction) -> Result<()> {
        let tx_hash = self.calculate_hash()?;

        // Remove nullifiers
        tx.execute(
            "DELETE FROM nullifier_set WHERE tx_hash = ?",
            [&tx_hash],
        )?;

        // Remove shielded notes
        tx.execute(
            "DELETE FROM shielded_notes WHERE tx_hash = ?",
            [&tx_hash],
        )?;

        // Remove transparent outputs
        tx.execute(
            "DELETE FROM transparent_outputs WHERE tx_hash = ?",
            [&tx_hash],
        )?;

        // Remove transaction
        tx.execute(
            "DELETE FROM transactions WHERE tx_hash = ?",
            [&tx_hash],
        )?;

        Ok(())
    }

    // Private validation methods
    fn validate_deposit(&self) -> Result<bool> {
        // Deposits should only have transparent outputs
        if !self.transparent_inputs.is_empty() 
            || !self.shielded_inputs.is_empty()
            || !self.shielded_outputs.is_empty()
            || !self.nullifiers.is_empty()
            || self.proof.is_some() {
            return Ok(false);
        }

        // Should have exactly one output
        if self.transparent_outputs.len() != 1 {
            return Ok(false);
        }

        Ok(true)
    }

    fn validate_transparent(&self, db: &crate::database::Database) -> Result<bool> {
        // Should only have transparent inputs/outputs
        if !self.shielded_inputs.is_empty()
            || !self.shielded_outputs.is_empty()
            || !self.nullifiers.is_empty()
            || self.proof.is_some() {
            return Ok(false);
        }

        // Verify inputs exist and are unspent
        for input in &self.transparent_inputs {
            let is_unspent = db.with_transaction(|tx| {
                let count: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM transparent_outputs 
                     WHERE output_id = ? AND spent_in_tx IS NULL",
                    [&input.output_id],
                    |row| row.get(0),
                )?;
                Ok(count > 0)
            })?;

            if !is_unspent {
                return Ok(false);
            }
        }

        // Verify input amount equals output amount
        let input_sum: u64 = self.transparent_inputs.iter().map(|i| i.amount).sum();
        let output_sum: u64 = self.transparent_outputs.iter().map(|o| o.amount).sum();
        if input_sum != output_sum {
            return Ok(false);
        }

        Ok(true)
    }

    fn validate_shield(&self, db: &crate::database::Database) -> Result<bool> {
        // Should have one transparent input and one shielded output
        if self.transparent_inputs.len() != 1
            || !self.transparent_outputs.is_empty()
            || !self.shielded_inputs.is_empty()
            || self.shielded_outputs.len() != 1 {
            return Ok(false);
        }

        // Verify input exists and is unspent
        let input = &self.transparent_inputs[0];
        let is_unspent = db.with_transaction(|tx| {
            let count: i64 = tx.query_row(
                "SELECT COUNT(*) FROM transparent_outputs 
                 WHERE output_id = ? AND spent_in_tx IS NULL",
                [&input.output_id],
                |row| row.get(0),
            )?;
            Ok(count > 0)
        })?;

        if !is_unspent {
            return Ok(false);
        }

        // Verify proof if present
        if let Some(proof) = &self.proof {
            // TODO: Implement zk-SNARK proof verification
            // For now, just check it exists
            if proof.is_empty() {
                return Ok(false);
            }
        }

        Ok(true)
    }

    fn validate_shield_to_shield(&self, db: &crate::database::Database) -> Result<bool> {
        // Should have one shielded input and output
        if !self.transparent_inputs.is_empty()
            || !self.transparent_outputs.is_empty()
            || self.shielded_inputs.len() != 1
            || self.shielded_outputs.len() != 1
            || self.nullifiers.len() != 1 {
            return Ok(false);
        }

        // Must have a proof
        if self.proof.is_none() {
            return Ok(false);
        }

        // TODO: Implement zk-SNARK proof verification
        // For now, just check it exists and isn't empty
        if self.proof.as_ref().unwrap().is_empty() {
            return Ok(false);
        }

        Ok(true)
    }

    fn validate_deshield(&self, db: &crate::database::Database) -> Result<bool> {
        // Should have one shielded input and one transparent output
        if !self.transparent_inputs.is_empty()
            || self.transparent_outputs.len() != 1
            || self.shielded_inputs.len() != 1
            || !self.shielded_outputs.is_empty()
            || self.nullifiers.len() != 1 {
            return Ok(false);
        }

        // Must have a proof
        if self.proof.is_none() {
            return Ok(false);
        }

        // TODO: Implement zk-SNARK proof verification
        // For now, just check it exists and isn't empty
        if self.proof.as_ref().unwrap().is_empty() {
            return Ok(false);
        }

        Ok(true)
    }
}

// Helper function to create a hash
fn blake2b_hash(data: &[u8]) -> Vec<u8> {
    let mut hasher = Blake2b512::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::address::{Address, AddressType, KeyPair};
    use tempfile::NamedTempFile;

    fn create_test_db() -> Result<(NamedTempFile, crate::database::Database)> {
        let temp_file = NamedTempFile::new().unwrap();
        let db = crate::database::Database::new(temp_file.path().to_str().unwrap())?;
        db.initialize()?;
        Ok((temp_file, db))
    }

    #[test]
    fn test_deposit_transaction() -> Result<()> {
        let (_temp_file, db) = create_test_db()?;
        
        // Create test address
        let key_pair = KeyPair::generate(AddressType::Transparent)?;
        let address = Address::from_key_pair(&key_pair)?;
        
        // Create deposit transaction
        let tx = Transaction::new_deposit(address, 1000)?;
        
        // Validate
        assert!(tx.validate(&db)?);
        
        Ok(())
    }

    #[test]
    fn test_transaction_connection() -> Result<()> {
        let (_temp_file, db) = create_test_db()?;
        
        // Create test address and transaction
        let key_pair = KeyPair::generate(AddressType::Transparent)?;
        let address = Address::from_key_pair(&key_pair)?;
        let tx = Transaction::new_deposit(address, 1000)?;
        
        // Connect transaction within a block
        db.with_transaction(|dbtx| {
            // First create a block (required by foreign key constraints)
            dbtx.execute(
                "INSERT INTO blocks (height, hash, parent_hash, timestamp, merkle_root, status)
                 VALUES (?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    0i64,
                    vec![0u8; 32],
                    vec![0u8; 32],
                    db.get_current_timestamp(),
                    vec![0u8; 32],
                    "active"
                ],
            )?;
            
            // Connect transaction
            tx.connect(dbtx, 0)?;
            
            Ok(())
        })?;
        
        Ok(())
    }
}

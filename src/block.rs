use crate::error::{Error, Result};
use crate::database::Database;
use crate::transaction::Transaction;
use blake2::{Blake2b512, Digest};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Represents a block in the chain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    /// Block height
    pub height: i64,
    /// Block hash
    pub hash: Vec<u8>,
    /// Parent block hash
    pub parent_hash: Vec<u8>,
    /// Block timestamp
    pub timestamp: i64,
    /// Merkle root of transactions
    pub merkle_root: Vec<u8>,
    /// List of transactions in the block
    pub transactions: Vec<Transaction>,
}

impl Block {
    /// Create a new block with the given parameters
    pub fn new(
        height: i64,
        parent_hash: Vec<u8>,
        timestamp: i64,
        transactions: Vec<Transaction>,
    ) -> Result<Self> {
        // Calculate merkle root
        let merkle_root = Self::calculate_merkle_root(&transactions)?;
        
        // Create block without hash
        let mut block = Self {
            height,
            hash: Vec::new(),
            parent_hash,
            timestamp,
            merkle_root,
            transactions,
        };
        
        // Calculate and set block hash
        block.hash = block.calculate_hash()?;
        
        Ok(block)
    }

    /// Calculate the merkle root of the transactions
    fn calculate_merkle_root(transactions: &[Transaction]) -> Result<Vec<u8>> {
        if transactions.is_empty() {
            return Ok(vec![0; 32]); // Empty merkle root for empty block
        }

        // Get transaction hashes
        let mut hashes: Vec<Vec<u8>> = transactions
            .iter()
            .map(|tx| tx.calculate_hash())
            .collect::<Result<Vec<_>>>()?;

        // Build merkle tree
        while hashes.len() > 1 {
            let mut new_hashes = Vec::new();
            
            for pair in hashes.chunks(2) {
                let mut hasher = Blake2b512::new();
                hasher.update(&pair[0]);
                if pair.len() > 1 {
                    hasher.update(&pair[1]);
                } else {
                    hasher.update(&pair[0]); // Duplicate last hash if odd number
                }
                new_hashes.push(hasher.finalize().to_vec());
            }
            
            hashes = new_hashes;
        }

        Ok(hashes.remove(0))
    }

    /// Calculate the block hash
    pub fn calculate_hash(&self) -> Result<Vec<u8>> {
        let mut hasher = Blake2b512::new();
        
        // Hash block header components
        hasher.update(&self.height.to_le_bytes());
        hasher.update(&self.parent_hash);
        hasher.update(&self.timestamp.to_le_bytes());
        hasher.update(&self.merkle_root);
        
        Ok(hasher.finalize().to_vec())
    }

    /// Validate the block
    pub fn validate(&self, db: &Database) -> Result<bool> {
        // Check block hash
        let calculated_hash = self.calculate_hash()?;
        if calculated_hash != self.hash {
            return Ok(false);
        }

        // Check merkle root
        let calculated_merkle_root = Self::calculate_merkle_root(&self.transactions)?;
        if calculated_merkle_root != self.merkle_root {
            return Ok(false);
        }

        // Check parent exists (except for genesis block)
        if self.height > 0 {
            let parent_exists = db.with_transaction(|tx| {
                let count: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM blocks WHERE hash = ? AND height = ?",
                    rusqlite::params![&self.parent_hash, self.height - 1],
                    |row| row.get(0),
                )?;
                Ok(count > 0)
            })?;

            if !parent_exists {
                return Ok(false);
            }
        }

        // Validate each transaction
        let mut nullifiers = HashSet::new();
        for tx in &self.transactions {
            // Check transaction validity
            if !tx.validate(db)? {
                return Ok(false);
            }

            // Check for duplicate nullifiers within block
            for nullifier in tx.get_nullifiers() {
                if !nullifiers.insert(nullifier.clone()) {
                    return Ok(false);
                }
            }

            // Check nullifiers don't already exist in chain
            for nullifier in tx.get_nullifiers() {
                if db.has_nullifier(&nullifier)? {
                    return Ok(false);
                }
            }
        }

        Ok(true)
    }

    /// Connect the block to the chain
    pub fn connect(&self, db: &Database) -> Result<()> {
        db.with_transaction(|tx| {
            // Insert block
            tx.execute(
                "INSERT INTO blocks (height, hash, parent_hash, timestamp, merkle_root, status)
                 VALUES (?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    self.height,
                    &self.hash,
                    &self.parent_hash,
                    self.timestamp,
                    &self.merkle_root,
                    "active"
                ],
            )?;

            // Process each transaction
            for transaction in &self.transactions {
                transaction.connect(tx, self.height)?;
            }

            Ok(())
        })
    }

    /// Disconnect the block from the chain
    pub fn disconnect(&self, db: &Database) -> Result<()> {
        db.with_transaction(|tx| {
            // Process transactions in reverse order
            for transaction in self.transactions.iter().rev() {
                transaction.disconnect(tx)?;
            }

            // Mark block as orphaned
            tx.execute(
                "UPDATE blocks SET status = 'orphaned' WHERE height = ?",
                [self.height],
            )?;

            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn create_test_db() -> Result<(NamedTempFile, Database)> {
        let temp_file = NamedTempFile::new().unwrap();
        let db = Database::new(temp_file.path().to_str().unwrap())?;
        db.initialize()?;
        Ok((temp_file, db))
    }

    #[test]
    fn test_block_creation_and_validation() -> Result<()> {
        let (_temp_file, db) = create_test_db()?;
        
        // Create genesis block
        let genesis = Block::new(
            0,
            vec![0; 32],
            db.get_current_timestamp(),
            Vec::new(),
        )?;

        // Validate genesis block
        assert!(genesis.validate(&db)?);

        // Connect genesis block
        genesis.connect(&db)?;

        // Create next block
        let block = Block::new(
            1,
            genesis.hash.clone(),
            db.get_current_timestamp(),
            Vec::new(),
        )?;

        // Validate block
        assert!(block.validate(&db)?);

        Ok(())
    }

    #[test]
    fn test_block_connection_and_disconnection() -> Result<()> {
        let (_temp_file, db) = create_test_db()?;
        
        // Create and connect genesis block
        let genesis = Block::new(
            0,
            vec![0; 32],
            db.get_current_timestamp(),
            Vec::new(),
        )?;
        genesis.connect(&db)?;

        // Create and connect block 1
        let block1 = Block::new(
            1,
            genesis.hash.clone(),
            db.get_current_timestamp(),
            Vec::new(),
        )?;
        block1.connect(&db)?;

        // Verify chain height
        assert_eq!(db.get_chain_height()?, 1);

        // Disconnect block 1
        block1.disconnect(&db)?;

        // Verify chain height reverted
        assert_eq!(db.get_chain_height()?, 0);

        Ok(())
    }
}

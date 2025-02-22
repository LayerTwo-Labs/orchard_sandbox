use crate::error::{Error, Result};
use rusqlite::Transaction as SqlTransaction;
use std::collections::HashSet;

/// Manages the nullifier set to prevent double-spending
pub struct NullifierSet {
    // Cache of active nullifiers for quick lookup
    cache: HashSet<Vec<u8>>,
}

impl NullifierSet {
    /// Create a new nullifier set manager
    pub fn new() -> Self {
        Self {
            cache: HashSet::new(),
        }
    }

    /// Initialize the nullifier set from the database
    pub fn initialize(&mut self, tx: &SqlTransaction) -> Result<()> {
        let mut stmt = tx.prepare(
            "SELECT nullifier FROM nullifier_set"
        )?;

        let nullifiers = stmt.query_map([], |row| {
            let nullifier: Vec<u8> = row.get(0)?;
            Ok(nullifier)
        })?;

        for nullifier in nullifiers {
            self.cache.insert(nullifier?);
        }

        Ok(())
    }

    /// Add a nullifier to the set
    pub fn add(
        &mut self,
        tx: &SqlTransaction,
        nullifier: Vec<u8>,
        block_height: i64,
        tx_hash: &[u8],
    ) -> Result<()> {
        // Check if nullifier already exists
        if self.contains(&nullifier) {
            return Err(Error::nullifier_error("Nullifier already exists"));
        }

        // Insert into database
        tx.execute(
            "INSERT INTO nullifier_set (nullifier, block_height, tx_hash)
             VALUES (?, ?, ?)",
            rusqlite::params![nullifier, block_height, tx_hash],
        )?;

        // Add to cache
        self.cache.insert(nullifier);

        Ok(())
    }

    /// Remove a nullifier from the set
    pub fn remove(&mut self, tx: &SqlTransaction, nullifier: &[u8]) -> Result<()> {
        // Remove from database
        tx.execute(
            "DELETE FROM nullifier_set WHERE nullifier = ?",
            [nullifier],
        )?;

        // Remove from cache
        self.cache.remove(nullifier);

        Ok(())
    }

    /// Check if a nullifier exists in the set
    pub fn contains(&self, nullifier: &[u8]) -> bool {
        self.cache.contains(nullifier)
    }

    /// Get all nullifiers added in a specific block
    pub fn get_block_nullifiers(
        &self,
        tx: &SqlTransaction,
        block_height: i64,
    ) -> Result<Vec<Vec<u8>>> {
        let mut stmt = tx.prepare(
            "SELECT nullifier FROM nullifier_set WHERE block_height = ?"
        )?;

        let nullifiers = stmt.query_map([block_height], |row| {
            let nullifier: Vec<u8> = row.get(0)?;
            Ok(nullifier)
        })?;

        let mut result = Vec::new();
        for nullifier in nullifiers {
            result.push(nullifier?);
        }

        Ok(result)
    }

    /// Revert the nullifier set to a previous state
    pub fn revert(&mut self, tx: &SqlTransaction, block_height: i64) -> Result<()> {
        // Get nullifiers to remove
        let nullifiers = self.get_block_nullifiers(tx, block_height)?;

        // Remove from database
        tx.execute(
            "DELETE FROM nullifier_set WHERE block_height >= ?",
            [block_height],
        )?;

        // Remove from cache
        for nullifier in nullifiers {
            self.cache.remove(&nullifier);
        }

        Ok(())
    }

    /// Clear the cache and reload from database
    pub fn reload(&mut self, tx: &SqlTransaction) -> Result<()> {
        self.cache.clear();
        self.initialize(tx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    use crate::database::Database;

    fn create_test_db() -> Result<(NamedTempFile, Database)> {
        let temp_file = NamedTempFile::new().unwrap();
        let db = Database::new(temp_file.path().to_str().unwrap())?;
        db.initialize()?;
        Ok((temp_file, db))
    }

    #[test]
    fn test_nullifier_set_operations() -> Result<()> {
        let (_temp_file, db) = create_test_db()?;
        let mut nullifier_set = NullifierSet::new();

        db.with_transaction(|tx| {
            // First create a block (required by foreign key constraints)
            tx.execute(
                "INSERT INTO blocks (height, hash, parent_hash, timestamp, merkle_root, status)
                 VALUES (?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    0i64,
                    vec![0u8; 32],
                    vec![0u8; 32],
                    0i64,
                    vec![0u8; 32],
                    "active"
                ],
            )?;

            // Create a transaction (required by foreign key constraints)
            tx.execute(
                "INSERT INTO transactions (tx_hash, block_height, tx_type, raw_data)
                 VALUES (?, ?, ?, ?)",
                rusqlite::params![
                    vec![0u8; 32],
                    0i64,
                    "transparent",
                    vec![0u8; 32]
                ],
            )?;

            nullifier_set.initialize(tx)?;

            // Test adding a nullifier
            let nullifier1 = vec![1u8; 32];
            nullifier_set.add(tx, nullifier1.clone(), 0, &vec![0u8; 32])?;
            assert!(nullifier_set.contains(&nullifier1));

            // Test adding duplicate nullifier
            let result = nullifier_set.add(tx, nullifier1.clone(), 0, &vec![0u8; 32]);
            assert!(result.is_err());

            // Test removing a nullifier
            nullifier_set.remove(tx, &nullifier1)?;
            assert!(!nullifier_set.contains(&nullifier1));

            Ok(())
        })?;

        Ok(())
    }

    #[test]
    fn test_nullifier_set_revert() -> Result<()> {
        let (_temp_file, db) = create_test_db()?;
        let mut nullifier_set = NullifierSet::new();

        db.with_transaction(|tx| {
            // Setup blocks and transactions
            for i in 0..2 {
                tx.execute(
                    "INSERT INTO blocks (height, hash, parent_hash, timestamp, merkle_root, status)
                     VALUES (?, ?, ?, ?, ?, ?)",
                    rusqlite::params![
                        i,
                        vec![i as u8; 32],
                        vec![0u8; 32],
                        0i64,
                        vec![0u8; 32],
                        "active"
                    ],
                )?;

                tx.execute(
                    "INSERT INTO transactions (tx_hash, block_height, tx_type, raw_data)
                     VALUES (?, ?, ?, ?)",
                    rusqlite::params![
                        vec![i as u8; 32],
                        i,
                        "transparent",
                        vec![0u8; 32]
                    ],
                )?;
            }

            nullifier_set.initialize(tx)?;

            // Add nullifiers in different blocks
            let nullifier1 = vec![1u8; 32];
            let nullifier2 = vec![2u8; 32];
            
            nullifier_set.add(tx, nullifier1.clone(), 0, &vec![0u8; 32])?;
            nullifier_set.add(tx, nullifier2.clone(), 1, &vec![0u8; 32])?;

            // Verify both exist
            assert!(nullifier_set.contains(&nullifier1));
            assert!(nullifier_set.contains(&nullifier2));

            // Revert to block 0
            nullifier_set.revert(tx, 1)?;

            // Only nullifier1 should exist
            assert!(nullifier_set.contains(&nullifier1));
            assert!(!nullifier_set.contains(&nullifier2));

            Ok(())
        })?;

        Ok(())
    }
}

use crate::error::{Error, Result};
use rusqlite::{Connection, Transaction};
use std::path::Path;
use time::OffsetDateTime;

/// Represents the database connection and provides high-level database operations
pub struct Database {
    conn: Connection,
}

impl Database {
    /// Create a new database connection
    pub fn new(path: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        
        // Enable foreign key constraints
        conn.execute_batch("PRAGMA foreign_keys = ON")?;
        
        Ok(Self { conn })
    }

    /// Initialize the database schema
    pub fn initialize(&self) -> Result<()> {
        self.conn.execute_batch(include_str!("../schema/init.sql"))?;
        Ok(())
    }

    /// Begin a new transaction
    pub fn transaction(&self) -> Result<Transaction> {
        Ok(self.conn.transaction()?)
    }

    /// Execute a function within a transaction
    pub fn with_transaction<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Transaction) -> Result<T>,
    {
        let tx = self.transaction()?;
        match f(&tx) {
            Ok(result) => {
                tx.commit()?;
                Ok(result)
            }
            Err(e) => {
                tx.rollback()?;
                Err(e)
            }
        }
    }

    /// Get the current chain height
    pub fn get_chain_height(&self) -> Result<i64> {
        self.conn
            .query_row(
                "SELECT COALESCE(MAX(height), -1) FROM blocks WHERE status = 'active'",
                [],
                |row| row.get(0),
            )
            .map_err(Error::from)
    }

    /// Check if a nullifier exists
    pub fn has_nullifier(&self, nullifier: &[u8]) -> Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM nullifier_set WHERE nullifier = ?",
            [nullifier],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Get the current timestamp
    pub fn get_current_timestamp(&self) -> i64 {
        OffsetDateTime::now_utc().unix_timestamp()
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
    fn test_database_initialization() -> Result<()> {
        let (_temp_file, db) = create_test_db()?;
        
        // Verify we can query the chain height
        let height = db.get_chain_height()?;
        assert_eq!(height, -1); // No blocks yet
        
        Ok(())
    }

    #[test]
    fn test_transaction_rollback() -> Result<()> {
        let (_temp_file, db) = create_test_db()?;
        
        // Try to execute an invalid SQL statement within a transaction
        let result = db.with_transaction(|tx| {
            tx.execute("INSERT INTO nonexistent_table (col) VALUES (?)", ["value"])?;
            Ok(())
        });
        
        // Should fail and rollback
        assert!(result.is_err());
        
        Ok(())
    }

    #[test]
    fn test_nullifier_operations() -> Result<()> {
        let (_temp_file, db) = create_test_db()?;
        let nullifier = vec![1u8; 32];
        
        // Initially should not exist
        assert!(!db.has_nullifier(&nullifier)?);
        
        // Add a block and nullifier
        db.with_transaction(|tx| {
            // First add a block (required by foreign key constraint)
            tx.execute(
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
            
            // Add a transaction (required by foreign key constraint)
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
            
            // Add the nullifier
            tx.execute(
                "INSERT INTO nullifier_set (nullifier, block_height, tx_hash) VALUES (?, ?, ?)",
                rusqlite::params![nullifier, 0i64, vec![0u8; 32]],
            )?;
            
            Ok(())
        })?;
        
        // Now should exist
        assert!(db.has_nullifier(&nullifier)?);
        
        Ok(())
    }
}

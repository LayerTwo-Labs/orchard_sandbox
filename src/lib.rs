pub mod address;
pub mod block;
pub mod database;
pub mod error;
pub mod merkle;
pub mod nullifier;
pub mod transaction;

use crate::error::Result;

/// Core type representing a hash value
pub type Hash = [u8; 32];

/// Represents the chain state manager
pub struct ChainState {
    db: database::Database,
}

impl ChainState {
    /// Create a new chain state with the given database connection
    pub fn new(db: database::Database) -> Self {
        Self { db }
    }

    /// Initialize a new chain state with the given database path
    pub fn initialize(db_path: &str) -> Result<Self> {
        let db = database::Database::new(db_path)?;
        db.initialize()?;
        Ok(Self::new(db))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_chain_state_initialization() -> Result<()> {
        let temp_file = NamedTempFile::new().unwrap();
        let chain_state = ChainState::initialize(temp_file.path().to_str().unwrap())?;
        Ok(())
    }
}

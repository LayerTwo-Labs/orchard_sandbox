use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("Invalid block: {0}")]
    InvalidBlock(String),

    #[error("Invalid transaction: {0}")]
    InvalidTransaction(String),

    #[error("Invalid address: {0}")]
    InvalidAddress(String),

    #[error("State error: {0}")]
    State(String),

    #[error("Cryptographic error: {0}")]
    Crypto(String),

    #[error("Merkle tree error: {0}")]
    MerkleTree(String),

    #[error("Nullifier error: {0}")]
    Nullifier(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Proof verification error: {0}")]
    ProofVerification(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn invalid_block(msg: impl Into<String>) -> Self {
        Error::InvalidBlock(msg.into())
    }

    pub fn invalid_transaction(msg: impl Into<String>) -> Self {
        Error::InvalidTransaction(msg.into())
    }

    pub fn invalid_address(msg: impl Into<String>) -> Self {
        Error::InvalidAddress(msg.into())
    }

    pub fn state_error(msg: impl Into<String>) -> Self {
        Error::State(msg.into())
    }

    pub fn crypto_error(msg: impl Into<String>) -> Self {
        Error::Crypto(msg.into())
    }

    pub fn merkle_tree_error(msg: impl Into<String>) -> Self {
        Error::MerkleTree(msg.into())
    }

    pub fn nullifier_error(msg: impl Into<String>) -> Self {
        Error::Nullifier(msg.into())
    }

    pub fn serialization_error(msg: impl Into<String>) -> Self {
        Error::Serialization(msg.into())
    }

    pub fn proof_verification_error(msg: impl Into<String>) -> Self {
        Error::ProofVerification(msg.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_creation() {
        let err = Error::invalid_block("test error");
        assert!(matches!(err, Error::InvalidBlock(_)));
        
        let err = Error::invalid_transaction("test error");
        assert!(matches!(err, Error::InvalidTransaction(_)));
        
        let err = Error::state_error("test error");
        assert!(matches!(err, Error::State(_)));
    }

    #[test]
    fn test_error_display() {
        let err = Error::invalid_block("test error");
        assert_eq!(err.to_string(), "Invalid block: test error");
        
        let err = Error::crypto_error("test error");
        assert_eq!(err.to_string(), "Cryptographic error: test error");
    }
}

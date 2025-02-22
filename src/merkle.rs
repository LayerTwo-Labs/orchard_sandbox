use crate::error::{Error, Result};
use blake2::{Blake2b512, Digest};
use rusqlite::Transaction as SqlTransaction;
use std::collections::HashMap;

const TREE_DEPTH: usize = 32;  // Depth of the Merkle tree

/// Represents a node in the Merkle tree
#[derive(Debug, Clone)]
pub struct Node {
    pub height: i64,
    pub position: i64,
    pub hash: Vec<u8>,
}

/// Represents a Merkle path for proving inclusion
#[derive(Debug, Clone)]
pub struct MerklePath {
    pub authentication_path: Vec<Vec<u8>>,
    pub position: u64,
}

/// Manages the incremental Merkle tree
pub struct MerkleTreeManager {
    cache: HashMap<(i64, i64), Vec<u8>>,  // Cache of (height, position) -> hash
}

impl MerkleTreeManager {
    /// Create a new Merkle tree manager
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    /// Initialize the Merkle tree with empty nodes
    pub fn initialize(&self, tx: &SqlTransaction) -> Result<()> {
        // Create empty root node
        let empty_hash = vec![0u8; 32];
        tx.execute(
            "INSERT INTO merkle_tree (height, position, hash, block_height, is_active)
             VALUES (?, ?, ?, ?, ?)",
            rusqlite::params![
                TREE_DEPTH as i64,
                0i64,
                &empty_hash,
                0i64,
                true
            ],
        )?;

        Ok(())
    }

    /// Add a new leaf to the tree
    pub fn append(
        &mut self,
        tx: &SqlTransaction,
        block_height: i64,
        commitment: &[u8],
    ) -> Result<MerklePath> {
        // Get current number of leaves
        let position: i64 = tx.query_row(
            "SELECT COUNT(*) FROM merkle_tree WHERE height = 0 AND is_active = true",
            [],
            |row| row.get(0),
        )?;

        // Insert leaf node
        tx.execute(
            "INSERT INTO merkle_tree (height, position, hash, block_height, is_active)
             VALUES (?, ?, ?, ?, ?)",
            rusqlite::params![
                0i64,
                position,
                commitment,
                block_height,
                true
            ],
        )?;

        // Update path to root
        let mut current_hash = commitment.to_vec();
        let mut auth_path = Vec::new();
        let mut current_position = position;

        for height in 0..TREE_DEPTH {
            let sibling_position = if current_position % 2 == 0 {
                current_position + 1
            } else {
                current_position - 1
            };

            // Get or create sibling node
            let sibling_hash = match self.get_node(tx, height as i64, sibling_position)? {
                Some(node) => {
                    auth_path.push(node.hash.clone());
                    node.hash
                },
                None => {
                    let empty_hash = vec![0u8; 32];
                    auth_path.push(empty_hash.clone());
                    empty_hash
                }
            };

            // Calculate parent hash
            let parent_hash = if current_position % 2 == 0 {
                self.combine_hashes(&current_hash, &sibling_hash)
            } else {
                self.combine_hashes(&sibling_hash, &current_hash)
            };

            // Store parent node
            current_position /= 2;
            tx.execute(
                "INSERT INTO merkle_tree (height, position, hash, block_height, is_active)
                 VALUES (?, ?, ?, ?, ?)",
                rusqlite::params![
                    (height + 1) as i64,
                    current_position,
                    &parent_hash,
                    block_height,
                    true
                ],
            )?;

            current_hash = parent_hash;
        }

        Ok(MerklePath {
            authentication_path: auth_path,
            position: position as u64,
        })
    }

    /// Get a node from the tree
    pub fn get_node(
        &self,
        tx: &SqlTransaction,
        height: i64,
        position: i64,
    ) -> Result<Option<Node>> {
        // Check cache first
        if let Some(hash) = self.cache.get(&(height, position)) {
            return Ok(Some(Node {
                height,
                position,
                hash: hash.clone(),
            }));
        }

        // Query database
        let result = tx.query_row(
            "SELECT hash FROM merkle_tree 
             WHERE height = ? AND position = ? AND is_active = true",
            [height, position],
            |row| {
                let hash: Vec<u8> = row.get(0)?;
                Ok(Node {
                    height,
                    position,
                    hash,
                })
            },
        );

        match result {
            Ok(node) => Ok(Some(node)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(Error::from(e)),
        }
    }

    /// Get the current root hash
    pub fn get_root(&self, tx: &SqlTransaction) -> Result<Vec<u8>> {
        let root = tx.query_row(
            "SELECT hash FROM merkle_tree 
             WHERE height = ? AND position = 0 AND is_active = true
             ORDER BY block_height DESC LIMIT 1",
            [TREE_DEPTH as i64],
            |row| {
                let hash: Vec<u8> = row.get(0)?;
                Ok(hash)
            },
        )?;

        Ok(root)
    }

    /// Revert the tree to a previous state
    pub fn revert(&mut self, tx: &SqlTransaction, block_height: i64) -> Result<()> {
        // Mark nodes from this block and later as inactive
        tx.execute(
            "UPDATE merkle_tree SET is_active = false 
             WHERE block_height >= ?",
            [block_height],
        )?;

        // Clear cache
        self.cache.clear();

        Ok(())
    }

    /// Verify a Merkle path
    pub fn verify_path(
        &self,
        commitment: &[u8],
        path: &MerklePath,
        root: &[u8],
    ) -> Result<bool> {
        let mut current_hash = commitment.to_vec();
        let mut current_position = path.position;

        for sibling in &path.authentication_path {
            current_hash = if current_position % 2 == 0 {
                self.combine_hashes(&current_hash, sibling)
            } else {
                self.combine_hashes(sibling, &current_hash)
            };
            current_position /= 2;
        }

        Ok(current_hash == root)
    }

    // Helper function to combine two hashes
    fn combine_hashes(&self, left: &[u8], right: &[u8]) -> Vec<u8> {
        let mut hasher = Blake2b512::new();
        hasher.update(left);
        hasher.update(right);
        hasher.finalize().to_vec()
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
    fn test_merkle_tree_initialization() -> Result<()> {
        let (_temp_file, db) = create_test_db()?;
        let mut tree = MerkleTreeManager::new();

        db.with_transaction(|tx| {
            tree.initialize(tx)?;
            let root = tree.get_root(tx)?;
            assert_eq!(root.len(), 32);
            Ok(())
        })?;

        Ok(())
    }

    #[test]
    fn test_merkle_tree_append() -> Result<()> {
        let (_temp_file, db) = create_test_db()?;
        let mut tree = MerkleTreeManager::new();

        db.with_transaction(|tx| {
            tree.initialize(tx)?;
            
            // Add first commitment
            let commitment1 = vec![1u8; 32];
            let path1 = tree.append(tx, 0, &commitment1)?;
            assert_eq!(path1.position, 0);
            
            // Add second commitment
            let commitment2 = vec![2u8; 32];
            let path2 = tree.append(tx, 0, &commitment2)?;
            assert_eq!(path2.position, 1);
            
            // Verify paths
            let root = tree.get_root(tx)?;
            assert!(tree.verify_path(&commitment1, &path1, &root)?);
            assert!(tree.verify_path(&commitment2, &path2, &root)?);
            
            Ok(())
        })?;

        Ok(())
    }

    #[test]
    fn test_merkle_tree_revert() -> Result<()> {
        let (_temp_file, db) = create_test_db()?;
        let mut tree = MerkleTreeManager::new();

        db.with_transaction(|tx| {
            tree.initialize(tx)?;
            
            // Add commitments in block 1
            let commitment1 = vec![1u8; 32];
            tree.append(tx, 1, &commitment1)?;
            
            // Add commitments in block 2
            let commitment2 = vec![2u8; 32];
            tree.append(tx, 2, &commitment2)?;
            
            // Remember root before revert
            let root_before = tree.get_root(tx)?;
            
            // Revert to block 1
            tree.revert(tx, 2)?;
            
            // Root should be different
            let root_after = tree.get_root(tx)?;
            assert_ne!(root_before, root_after);
            
            Ok(())
        })?;

        Ok(())
    }
}

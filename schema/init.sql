-- Address Management
CREATE TABLE IF NOT EXISTS keys (
    key_id BLOB PRIMARY KEY,
    key_type TEXT NOT NULL,  -- 'transparent' or 'shielded'
    private_key BLOB NOT NULL,
    public_key BLOB NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS addresses (
    address_id BLOB PRIMARY KEY,
    key_id BLOB NOT NULL,
    address_type TEXT NOT NULL,  -- 't-addr' or 'z-addr'
    address_data BLOB NOT NULL,  -- encoded address
    FOREIGN KEY(key_id) REFERENCES keys(key_id)
);

-- Block Storage
CREATE TABLE IF NOT EXISTS blocks (
    height INTEGER PRIMARY KEY,
    hash BLOB NOT NULL,
    parent_hash BLOB NOT NULL,
    timestamp INTEGER NOT NULL,
    merkle_root BLOB NOT NULL,
    status TEXT NOT NULL  -- 'active', 'orphaned'
);

CREATE INDEX IF NOT EXISTS idx_blocks_hash ON blocks(hash);
CREATE INDEX IF NOT EXISTS idx_blocks_parent ON blocks(parent_hash);

-- Transaction Management
CREATE TABLE IF NOT EXISTS transactions (
    tx_hash BLOB PRIMARY KEY,
    block_height INTEGER NOT NULL,
    tx_type TEXT NOT NULL,  -- 'deposit', 'transparent', 'shield', 'shield_to_shield', 'deshield'
    raw_data BLOB NOT NULL,
    proof_data BLOB,  -- NULL for transparent transactions
    FOREIGN KEY(block_height) REFERENCES blocks(height)
);

CREATE TABLE IF NOT EXISTS transparent_outputs (
    output_id BLOB PRIMARY KEY,
    tx_hash BLOB NOT NULL,
    address BLOB NOT NULL,
    amount INTEGER NOT NULL,
    spent_in_tx BLOB,  -- NULL if unspent
    FOREIGN KEY(tx_hash) REFERENCES transactions(tx_hash),
    FOREIGN KEY(spent_in_tx) REFERENCES transactions(tx_hash)
);

CREATE TABLE IF NOT EXISTS shielded_notes (
    note_commitment BLOB PRIMARY KEY,
    ephemeral_key BLOB NOT NULL,
    amount BLOB NOT NULL,  -- encrypted
    memo BLOB,
    nullifier BLOB,  -- NULL if unspent
    tx_hash BLOB NOT NULL,
    block_height INTEGER NOT NULL,
    merkle_position INTEGER NOT NULL,
    FOREIGN KEY(tx_hash) REFERENCES transactions(tx_hash),
    FOREIGN KEY(block_height) REFERENCES blocks(height)
);

-- State Management
CREATE TABLE IF NOT EXISTS nullifier_set (
    nullifier BLOB PRIMARY KEY,
    block_height INTEGER NOT NULL,
    tx_hash BLOB NOT NULL,
    FOREIGN KEY(block_height) REFERENCES blocks(height),
    FOREIGN KEY(tx_hash) REFERENCES transactions(tx_hash)
);

CREATE TABLE IF NOT EXISTS merkle_tree (
    height INTEGER NOT NULL,
    position INTEGER NOT NULL,
    hash BLOB NOT NULL,
    block_height INTEGER NOT NULL,
    is_active BOOLEAN NOT NULL DEFAULT true,
    PRIMARY KEY(height, position),
    FOREIGN KEY(block_height) REFERENCES blocks(height)
);

CREATE TABLE IF NOT EXISTS chain_state (
    key TEXT PRIMARY KEY,
    value BLOB NOT NULL,
    last_updated INTEGER NOT NULL
);

-- Indexes for performance
CREATE INDEX IF NOT EXISTS idx_tx_block ON transactions(block_height);
CREATE INDEX IF NOT EXISTS idx_notes_height ON shielded_notes(block_height);
CREATE INDEX IF NOT EXISTS idx_nullifiers_height ON nullifier_set(block_height);
CREATE INDEX IF NOT EXISTS idx_merkle_active ON merkle_tree(height, position) WHERE is_active = true;

use zcash_sidechain::{
    address::{Address, AddressType, KeyPair},
    block::Block,
    database::Database,
    error::Result,
    transaction::{Transaction, TransparentOutput, ShieldedNote},
};

fn main() -> Result<()> {
    // Initialize database
    let db = Database::initialize("zcash_test.db")?;

    // Generate addresses
    let t_key_a = KeyPair::generate(AddressType::Transparent)?;
    let t_addr_a = Address::from_key_pair(&t_key_a)?;
    println!("Transparent address A: {}", t_addr_a.to_string());

    let t_key_b = KeyPair::generate(AddressType::Transparent)?;
    let t_addr_b = Address::from_key_pair(&t_key_b)?;
    println!("Transparent address B: {}", t_addr_b.to_string());

    let z_key_x = KeyPair::generate(AddressType::Shielded)?;
    let z_addr_x = Address::from_key_pair(&z_key_x)?;
    println!("Shielded address X: {}", z_addr_x.to_string());

    let z_key_y = KeyPair::generate(AddressType::Shielded)?;
    let z_addr_y = Address::from_key_pair(&z_key_y)?;
    println!("Shielded address Y: {}", z_addr_y.to_string());

    // Create and connect genesis block
    let genesis = Block::new(
        0,
        vec![0; 32],
        db.get_current_timestamp(),
        Vec::new(),
    )?;
    
    if !genesis.validate(&db)? {
        println!("Genesis block validation failed!");
        return Ok(());
    }
    genesis.connect(&db)?;
    println!("Genesis block connected");

    // Create deposit transaction to t_addr_a
    let deposit_tx = Transaction::new_deposit(t_addr_a.clone(), 1000)?;
    let deposit_block = Block::new(
        1,
        genesis.hash.clone(),
        db.get_current_timestamp(),
        vec![deposit_tx],
    )?;

    if !deposit_block.validate(&db)? {
        println!("Deposit block validation failed!");
        return Ok(());
    }
    deposit_block.connect(&db)?;
    println!("Deposited 1000 coins to transparent address A");

    // Transfer from t_addr_a to t_addr_b
    let transparent_tx = Transaction::new_transparent(
        vec![/* Input would be created from deposit UTXO */],
        vec![TransparentOutput {
            address: t_addr_b.clone(),
            amount: 500,
        }],
    )?;
    let transparent_block = Block::new(
        2,
        deposit_block.hash.clone(),
        db.get_current_timestamp(),
        vec![transparent_tx],
    )?;

    if !transparent_block.validate(&db)? {
        println!("Transparent block validation failed!");
        return Ok(());
    }
    transparent_block.connect(&db)?;
    println!("Transferred 500 coins from transparent address A to B");

    // Shield coins from t_addr_b to z_addr_x
    let shield_note = ShieldedNote {
        commitment: vec![0; 32],  // Would be properly generated
        ephemeral_key: vec![0; 32],  // Would be properly generated
        amount: vec![0; 32],  // Would be encrypted
        memo: None,
    };
    let shield_tx = Transaction::new_shield(
        /* Input would be created from t_addr_b UTXO */,
        shield_note,
    )?;
    let shield_block = Block::new(
        3,
        transparent_block.hash.clone(),
        db.get_current_timestamp(),
        vec![shield_tx],
    )?;

    if !shield_block.validate(&db)? {
        println!("Shield block validation failed!");
        return Ok(());
    }
    shield_block.connect(&db)?;
    println!("Shielded coins from transparent address B to shielded address X");

    // Shield-to-shield transfer from z_addr_x to z_addr_y
    let shield_to_shield_tx = Transaction::new_shield_to_shield(
        /* Input note would be from previous shield tx */,
        ShieldedNote {
            commitment: vec![1; 32],  // Would be properly generated
            ephemeral_key: vec![1; 32],  // Would be properly generated
            amount: vec![1; 32],  // Would be encrypted
            memo: None,
        },
        vec![2; 32],  // Nullifier
        vec![3; 32],  // Proof
    )?;
    let shield_to_shield_block = Block::new(
        4,
        shield_block.hash.clone(),
        db.get_current_timestamp(),
        vec![shield_to_shield_tx],
    )?;

    if !shield_to_shield_block.validate(&db)? {
        println!("Shield-to-shield block validation failed!");
        return Ok(());
    }
    shield_to_shield_block.connect(&db)?;
    println!("Transferred shielded coins from address X to Y");

    // Deshield from z_addr_y back to t_addr_a
    let deshield_tx = Transaction::new_deshield(
        /* Input note would be from previous shield-to-shield tx */,
        TransparentOutput {
            address: t_addr_a.clone(),
            amount: 300,
        },
        vec![4; 32],  // Nullifier
        vec![5; 32],  // Proof
    )?;
    let deshield_block = Block::new(
        5,
        shield_to_shield_block.hash.clone(),
        db.get_current_timestamp(),
        vec![deshield_tx],
    )?;

    if !deshield_block.validate(&db)? {
        println!("Deshield block validation failed!");
        return Ok(());
    }
    deshield_block.connect(&db)?;
    println!("Deshielded coins back to transparent address A");

    // Test chain reorganization
    println!("\nTesting chain reorganization...");
    println!("Current chain height: {}", db.get_chain_height()?);
    
    // Disconnect last two blocks
    deshield_block.disconnect(&db)?;
    shield_to_shield_block.disconnect(&db)?;
    
    println!("After disconnecting two blocks, chain height: {}", db.get_chain_height()?);

    Ok(())
}

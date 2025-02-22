use crate::error::{Error, Result};
use blake2::{Blake2b512, Digest};
use rand::RngCore;
use serde::{Deserialize, Serialize};

/// Represents the type of address
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AddressType {
    Transparent,
    Shielded,
}

impl std::fmt::Display for AddressType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AddressType::Transparent => write!(f, "t-addr"),
            AddressType::Shielded => write!(f, "z-addr"),
        }
    }
}

/// Represents a key pair for either transparent or shielded addresses
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyPair {
    pub key_type: AddressType,
    pub private_key: Vec<u8>,
    pub public_key: Vec<u8>,
}

impl KeyPair {
    /// Generate a new key pair for the specified address type
    pub fn generate(key_type: AddressType) -> Result<Self> {
        // For now, we'll use a simple key generation scheme
        // In production, this would use proper EC cryptography
        let mut rng = rand::thread_rng();
        let mut private_key = vec![0u8; 32];
        rng.fill_bytes(&mut private_key);

        // Derive public key using Blake2b (this is just for demonstration)
        // In reality, we would use proper EC key derivation
        let mut hasher = Blake2b512::new();
        hasher.update(&private_key);
        let public_key = hasher.finalize().to_vec();

        Ok(Self {
            key_type,
            private_key,
            public_key,
        })
    }
}

/// Represents an address that can be either transparent or shielded
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Address {
    pub address_type: AddressType,
    pub data: Vec<u8>,
}

impl Address {
    /// Create a new address from a key pair
    pub fn from_key_pair(key_pair: &KeyPair) -> Result<Self> {
        // For demonstration, we'll create a simple address format
        // In production, this would use proper address encoding schemes
        let mut hasher = Blake2b512::new();
        hasher.update(&key_pair.public_key);
        
        // Add a prefix based on address type
        let prefix = match key_pair.key_type {
            AddressType::Transparent => b"t1",
            AddressType::Shielded => b"z1",
        };
        
        hasher.update(prefix);
        let address_bytes = hasher.finalize();

        Ok(Self {
            address_type: key_pair.key_type,
            data: address_bytes.to_vec(),
        })
    }

    /// Format the address as a string
    pub fn to_string(&self) -> String {
        let prefix = match self.address_type {
            AddressType::Transparent => "t1",
            AddressType::Shielded => "z1",
        };
        format!("{}{}", prefix, hex::encode(&self.data))
    }

    /// Parse an address from a string
    pub fn from_string(s: &str) -> Result<Self> {
        if s.len() < 3 {
            return Err(Error::invalid_address("Address too short"));
        }

        let (prefix, hex_data) = s.split_at(2);
        let address_type = match prefix {
            "t1" => AddressType::Transparent,
            "z1" => AddressType::Shielded,
            _ => return Err(Error::invalid_address("Invalid address prefix")),
        };

        let data = hex::decode(hex_data)
            .map_err(|e| Error::invalid_address(format!("Invalid hex: {}", e)))?;

        Ok(Self {
            address_type,
            data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_pair_generation() -> Result<()> {
        let t_keys = KeyPair::generate(AddressType::Transparent)?;
        assert_eq!(t_keys.key_type, AddressType::Transparent);
        assert_eq!(t_keys.private_key.len(), 32);
        assert!(!t_keys.public_key.is_empty());

        let z_keys = KeyPair::generate(AddressType::Shielded)?;
        assert_eq!(z_keys.key_type, AddressType::Shielded);
        assert_eq!(z_keys.private_key.len(), 32);
        assert!(!z_keys.public_key.is_empty());

        Ok(())
    }

    #[test]
    fn test_address_creation() -> Result<()> {
        let key_pair = KeyPair::generate(AddressType::Transparent)?;
        let address = Address::from_key_pair(&key_pair)?;
        assert_eq!(address.address_type, AddressType::Transparent);
        assert!(!address.data.is_empty());

        let addr_str = address.to_string();
        assert!(addr_str.starts_with("t1"));

        let parsed = Address::from_string(&addr_str)?;
        assert_eq!(parsed.address_type, address.address_type);
        assert_eq!(parsed.data, address.data);

        Ok(())
    }

    #[test]
    fn test_invalid_address_parsing() {
        assert!(Address::from_string("invalid").is_err());
        assert!(Address::from_string("x1abcd").is_err());
        assert!(Address::from_string("t1").is_err());
        assert!(Address::from_string("t1invalid_hex").is_err());
    }
}

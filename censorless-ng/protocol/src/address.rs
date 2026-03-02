use crate::{ProtocolError, Result};
use std::io::{Cursor, Read};
use std::net::{Ipv4Addr, Ipv6Addr};

/// SOCKS5 address format
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SocksAddress {
    IPv4(Ipv4Addr),
    Domain(String),
    IPv6(Ipv6Addr),
}

impl SocksAddress {
    /// Encode the address into SOCKS5 format
    pub fn encode(&self, buf: &mut Vec<u8>) -> Result<()> {
        match self {
            SocksAddress::IPv4(addr) => {
                buf.push(0x01); // Type byte for IPv4
                buf.extend_from_slice(&addr.octets());
            }
            SocksAddress::Domain(domain) => {
                if domain.len() > 255 {
                    return Err(ProtocolError::InvalidData(
                        "Domain name too long".to_string(),
                    ));
                }
                buf.push(0x03); // Type byte for domain
                buf.push(domain.len() as u8); // Length byte
                buf.extend_from_slice(domain.as_bytes());
            }
            SocksAddress::IPv6(addr) => {
                buf.push(0x04); // Type byte for IPv6
                buf.extend_from_slice(&addr.octets());
            }
        }
        Ok(())
    }

    /// Decode the address from SOCKS5 format
    pub fn decode(cursor: &mut Cursor<&[u8]>) -> Result<Self> {
        let mut type_buf = [0u8; 1];
        cursor.read_exact(&mut type_buf)?;
        let addr_type = type_buf[0];

        match addr_type {
            0x01 => {
                // IPv4
                let mut addr_buf = [0u8; 4];
                cursor.read_exact(&mut addr_buf)?;
                Ok(SocksAddress::IPv4(Ipv4Addr::from(addr_buf)))
            }
            0x03 => {
                // Domain name
                let mut len_buf = [0u8; 1];
                cursor.read_exact(&mut len_buf)?;
                let len = len_buf[0] as usize;

                let mut domain_buf = vec![0u8; len];
                cursor.read_exact(&mut domain_buf)?;
                let domain = String::from_utf8(domain_buf)
                    .map_err(|_| ProtocolError::InvalidData("Invalid domain name".to_string()))?;
                Ok(SocksAddress::Domain(domain))
            }
            0x04 => {
                // IPv6
                let mut addr_buf = [0u8; 16];
                cursor.read_exact(&mut addr_buf)?;
                Ok(SocksAddress::IPv6(Ipv6Addr::from(addr_buf)))
            }
            _ => Err(ProtocolError::InvalidAddressType(addr_type)),
        }
    }

    /// Get the encoded size of this address
    pub fn encoded_size(&self) -> usize {
        match self {
            SocksAddress::IPv4(_) => 1 + 4,             // type + 4 bytes
            SocksAddress::Domain(d) => 1 + 1 + d.len(), // type + len + domain
            SocksAddress::IPv6(_) => 1 + 16,            // type + 16 bytes
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ipv4_encode_decode() {
        let addr = SocksAddress::IPv4(Ipv4Addr::new(127, 0, 0, 1));
        let mut buf = Vec::new();
        addr.encode(&mut buf).unwrap();

        let mut cursor = Cursor::new(buf.as_slice());
        let decoded = SocksAddress::decode(&mut cursor).unwrap();
        assert_eq!(addr, decoded);
    }

    #[test]
    fn test_domain_encode_decode() {
        let addr = SocksAddress::Domain("example.com".to_string());
        let mut buf = Vec::new();
        addr.encode(&mut buf).unwrap();

        let mut cursor = Cursor::new(buf.as_slice());
        let decoded = SocksAddress::decode(&mut cursor).unwrap();
        assert_eq!(addr, decoded);
    }

    #[test]
    fn test_ipv6_encode_decode() {
        let addr = SocksAddress::IPv6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1));
        let mut buf = Vec::new();
        addr.encode(&mut buf).unwrap();

        let mut cursor = Cursor::new(buf.as_slice());
        let decoded = SocksAddress::decode(&mut cursor).unwrap();
        assert_eq!(addr, decoded);
    }
}

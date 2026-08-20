//! CRC32 (IEEE 802.3) implemented in-crate so we depend only on std + serde.

/// Precomputed CRC32 IEEE table, built once at first use.
struct Crc32Table {
    table: [u32; 256],
}

impl Crc32Table {
    const fn new() -> Self {
        let mut table = [0u32; 256];
        let mut i = 0usize;
        while i < 256 {
            let mut crc = i as u32;
            let mut j = 0;
            while j < 8 {
                if crc & 1 != 0 {
                    crc = 0xEDB8_8320 ^ (crc >> 1);
                } else {
                    crc >>= 1;
                }
                j += 1;
            }
            table[i] = crc;
            i += 1;
        }
        Self { table }
    }
}

static CRC_TABLE: Crc32Table = Crc32Table::new();

/// Compute the CRC32 (IEEE 802.3) of `data`.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        let idx = ((crc ^ (b as u32)) & 0xFF) as usize;
        crc = CRC_TABLE.table[idx] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

#[cfg(test)]
mod tests {
    use super::crc32;

    #[test]
    fn known_vectors() {
        // "123456789" => 0xCBF43926 is the canonical CRC32 check value.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0x0000_0000);
        assert_eq!(crc32(b"a"), 0xE8B7_BE43);
    }

    #[test]
    fn detects_single_bit_flip() {
        let a = crc32(b"hello world");
        let b = crc32(b"hellp world"); // one char changed
        assert_ne!(a, b);
    }
}

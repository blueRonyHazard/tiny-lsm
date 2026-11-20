//! CRC-32 checksum (the same one used by zip, gzip and PNG).
//!
//! Every record in the write-ahead log carries a checksum. On restart we
//! recompute it; if the numbers differ, the record was only half written
//! (or the disk flipped a bit) and we must not trust it.

/// Lookup table built at compile time. `const fn` runs inside the compiler,
/// so there is no start-up cost.
const TABLE: [u32; 256] = build_table();

const fn build_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    // `for` loops are not allowed in `const fn` yet, hence `while`.
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

/// Incremental hasher, so a record can be checksummed piece by piece
/// without first copying it into one buffer.
#[derive(Clone, Copy)]
pub struct Crc32(u32);

impl Default for Crc32 {
    fn default() -> Self {
        Self::new()
    }
}

impl Crc32 {
    pub fn new() -> Self {
        Crc32(0xFFFF_FFFF)
    }

    pub fn update(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = TABLE[((self.0 ^ b as u32) & 0xFF) as usize] ^ (self.0 >> 8);
        }
    }

    pub fn finish(self) -> u32 {
        self.0 ^ 0xFFFF_FFFF
    }
}

/// One-shot convenience function.
pub fn checksum(bytes: &[u8]) -> u32 {
    let mut c = Crc32::new();
    c.update(bytes);
    c.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_known_values() {
        // Standard check value for CRC-32.
        assert_eq!(checksum(b"123456789"), 0xCBF4_3926);
        assert_eq!(checksum(b""), 0);
    }

    #[test]
    fn incremental_equals_one_shot() {
        let mut c = Crc32::new();
        c.update(b"hello, ");
        c.update(b"world");
        assert_eq!(c.finish(), checksum(b"hello, world"));
    }
}

//! IFC GlobalIds: 128 bits written as 22 characters of IFC's base-64
//! alphabet. Forma derives them from a stable key (shape id and part name),
//! so exporting the same model twice gives the same ids and a model checker
//! sees the same objects.

const CHARS: &[u8; 64] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz_$";

/// 128 bits from a key (two independent 64-bit FNV-1a passes, mixed).
pub fn bits(key: &str) -> u128 {
    let fnv = |seed: u64| {
        let mut h = seed;
        for b in key.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        // splitmix finaliser
        h ^= h >> 30;
        h = h.wrapping_mul(0xbf58476d1ce4e5b9);
        h ^= h >> 27;
        h = h.wrapping_mul(0x94d049bb133111eb);
        h ^ (h >> 31)
    };
    ((fnv(0xcbf29ce484222325) as u128) << 64) | fnv(0x84222325cbf29ce4) as u128
}

/// The compressed GlobalId of 128 bits.
pub fn encode(v: u128) -> String {
    let mut out = [0u8; 22];
    // First character carries the top 2 bits, the other 21 six bits each.
    out[0] = CHARS[((v >> 126) & 0x3) as usize];
    for i in 0..21 {
        let shift = 120 - 6 * i;
        out[i + 1] = CHARS[((v >> shift) & 0x3f) as usize];
    }
    String::from_utf8(out.to_vec()).unwrap()
}

/// The 128 bits of a compressed GlobalId, if it is one.
pub fn decode(s: &str) -> Option<u128> {
    if s.len() != 22 {
        return None;
    }
    let mut v: u128 = 0;
    for (i, c) in s.bytes().enumerate() {
        let d = CHARS.iter().position(|&x| x == c)? as u128;
        if i == 0 && d > 3 {
            return None;
        }
        v = (v << if i == 0 { 0 } else { 6 }) | d;
    }
    Some(v)
}

/// The GlobalId for a key.
pub fn of(key: &str) -> String {
    encode(bits(key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_is_stable() {
        let g = of("wall 1");
        assert_eq!(g.len(), 22);
        assert_eq!(g, of("wall 1"));
        assert_ne!(g, of("wall 2"));
        assert_eq!(decode(&g), Some(bits("wall 1")));
        assert_eq!(encode(0), "0000000000000000000000");
        assert_eq!(encode(u128::MAX), "3$$$$$$$$$$$$$$$$$$$$$");
    }
}

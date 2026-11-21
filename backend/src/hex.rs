//! hex ของไบต์ และ id สุ่ม — ใช้เป็น session id / transfer id / pty id และแสดง SHA-256

pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// id สุ่มยาว `bytes`*2 ตัวอักษร จาก OS random
pub fn random_id(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::fill(&mut buf[..]);
    to_hex(&buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_and_random_id_have_expected_shape() {
        assert_eq!(to_hex(&[0x00, 0xab, 0xff]), "00abff");
        assert_eq!(random_id(8).len(), 16);
        assert_ne!(random_id(8), random_id(8));
    }
}

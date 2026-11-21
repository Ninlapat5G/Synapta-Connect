//! เลขรุ่น MAJOR.MINOR.PATCH (ตัวเลขล้วน) — ไม่รับ pre-release/ตัวอักษร: รุ่นที่ไม่เข้ารูปถูกปฏิเสธ ไม่เดา

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u32, pub u32, pub u32);

impl Version {
    pub fn parse(text: &str) -> Option<Version> {
        let mut parts = text.split('.');
        let mut next = || {
            let p = parts.next()?;
            // "01" / "+1" / "" ไม่ใช่เลขรุ่นที่ถูกต้อง — กันสองข้อความต่างกันแต่ถูกอ่านเป็นรุ่นเดียวกัน
            if p.is_empty()
                || p.len() > 9
                || !p.bytes().all(|b| b.is_ascii_digit())
                || (p.len() > 1 && p.starts_with('0'))
            {
                return None;
            }
            p.parse::<u32>().ok()
        };
        let v = Version(next()?, next()?, next()?);
        parts.next().is_none().then_some(v)
    }

    /// รุ่นของโปรแกรมที่กำลังรัน
    pub fn current() -> Version {
        Version::parse(env!("CARGO_PKG_VERSION")).expect("Cargo.toml version ต้องเป็น x.y.z")
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_orders_and_rejects_malformed() {
        assert_eq!(Version::parse("0.3.10"), Some(Version(0, 3, 10)));
        assert!(Version::parse("0.3.10") > Version::parse("0.3.9"));
        assert!(Version::parse("1.0.0") > Version::parse("0.99.99"));
        for bad in [
            "",
            "1",
            "1.2",
            "1.2.3.4",
            "1.2.x",
            "1.2.3-beta",
            "01.2.3",
            "1..3",
            "+1.2.3",
            "v1.2.3",
            "99999999999.0.0",
        ] {
            assert_eq!(Version::parse(bad), None, "{bad}");
        }
        assert_eq!(Version(1, 2, 3).to_string(), "1.2.3");
    }
}

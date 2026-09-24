#[derive(Debug, PartialEq, Eq, Clone, Copy, PartialOrd, Ord)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl Version {
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().trim_start_matches('v');
        let mut parts = s.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch_part = parts.next()?.split(['-', '+']).next()?;
        let patch = patch_part.parse().ok()?;
        Some(Self {
            major,
            minor,
            patch,
        })
    }

    #[must_use]
    pub fn is_newer_than(&self, other: &Self) -> bool {
        self > other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_parse_and_comparison() {
        let v1 = Version::parse("0.1.0").unwrap();
        let v2 = Version::parse("v0.1.1").unwrap();
        let v3 = Version::parse("v0.2.0-beta.1").unwrap();
        let v4 = Version::parse("1.0.0").unwrap();

        assert_eq!(
            v1,
            Version {
                major: 0,
                minor: 1,
                patch: 0
            }
        );
        assert_eq!(
            v2,
            Version {
                major: 0,
                minor: 1,
                patch: 1
            }
        );
        assert_eq!(
            v3,
            Version {
                major: 0,
                minor: 2,
                patch: 0
            }
        );
        assert_eq!(
            v4,
            Version {
                major: 1,
                minor: 0,
                patch: 0
            }
        );

        assert!(v2.is_newer_than(&v1));
        assert!(v3.is_newer_than(&v2));
        assert!(v4.is_newer_than(&v3));
        assert!(!v1.is_newer_than(&v2));
        assert!(!v1.is_newer_than(&v1));
    }

    #[test]
    fn test_version_parse_invalid() {
        assert_eq!(Version::parse("invalid"), None);
        assert_eq!(Version::parse("1.0"), None);
        assert_eq!(Version::parse(""), None);
    }
}

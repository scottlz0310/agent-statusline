#![allow(dead_code)]

pub const RESET: &str = "\x1b[0m";
pub const BOLD: &str = "\x1b[1m";
pub const DIMMED: &str = "\x1b[2m";
pub const RED: &str = "\x1b[31m";
pub const GREEN: &str = "\x1b[32m";
pub const YELLOW: &str = "\x1b[33m";
pub const BLUE: &str = "\x1b[34m";
pub const MAGENTA: &str = "\x1b[35m";
pub const CYAN: &str = "\x1b[36m";
pub const GRAY: &str = "\x1b[90m";

pub fn pct_color(pct: u8) -> &'static str {
    if pct >= 80 {
        RED
    } else if pct >= 50 {
        YELLOW
    } else {
        GREEN
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pct_color() {
        let cases = [
            (0, GREEN),
            (49, GREEN),
            (50, YELLOW),
            (79, YELLOW),
            (80, RED),
            (100, RED),
        ];

        for (pct, expected) in cases {
            assert_eq!(pct_color(pct), expected, "failed for pct: {pct}");
        }
    }
}

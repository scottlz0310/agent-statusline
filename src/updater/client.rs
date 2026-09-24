use serde::{Deserialize, Serialize};
use std::io;

pub const DEFAULT_REPO: &str = "scottlz0310/agent-statusline";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitHubRelease {
    pub tag_name: String,
    pub name: Option<String>,
    pub prerelease: bool,
    pub assets: Vec<GitHubAsset>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitHubAsset {
    pub name: String,
    pub browser_download_url: String,
    pub size: u64,
}

/// Fetch latest release metadata from GitHub Releases API.
pub fn fetch_latest_release() -> io::Result<GitHubRelease> {
    let url = format!("https://api.github.com/repos/{DEFAULT_REPO}/releases/latest");
    let mut req = ureq::get(&url)
        .header("User-Agent", "agent-statusline-updater")
        .header("Accept", "application/vnd.github.v3+json");

    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        req = req.header("Authorization", format!("Bearer {token}"));
    }

    let body = req
        .call()
        .map_err(|e| io::Error::other(format!("HTTP request failed: {e}")))?
        .body_mut()
        .read_to_string()
        .map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Failed to read response body: {e}"),
            )
        })?;

    parse_release_json(&body)
}

/// Download raw asset bytes from a download URL.
pub fn download_asset(url: &str) -> io::Result<Vec<u8>> {
    let mut req = ureq::get(url).header("User-Agent", "agent-statusline-updater");

    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        req = req.header("Authorization", format!("Bearer {token}"));
    }

    let mut reader = req
        .call()
        .map_err(|e| io::Error::other(format!("Failed to download asset: {e}")))?
        .into_body()
        .into_reader();

    let mut buf = Vec::new();
    io::copy(&mut reader, &mut buf)?;
    Ok(buf)
}

pub fn parse_release_json(json_str: &str) -> io::Result<GitHubRelease> {
    serde_json::from_str(json_str).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Failed to parse GitHub release JSON: {e}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_release_json() {
        let sample = r#"{
            "tag_name": "v0.2.0",
            "name": "Release 0.2.0",
            "prerelease": false,
            "assets": [
                {
                    "name": "agent-statusline-x86_64-pc-windows-msvc.zip",
                    "browser_download_url": "https://github.com/scottlz0310/agent-statusline/releases/download/v0.2.0/agent-statusline-x86_64-pc-windows-msvc.zip",
                    "size": 1234567
                }
            ]
        }"#;

        let release = parse_release_json(sample).unwrap();
        assert_eq!(release.tag_name, "v0.2.0");
        assert_eq!(release.assets.len(), 1);
        assert_eq!(
            release.assets[0].name,
            "agent-statusline-x86_64-pc-windows-msvc.zip"
        );
    }
}

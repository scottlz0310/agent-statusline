pub mod cache;
pub mod client;
pub mod replace;
pub mod semver;

use std::fmt::Write;
use std::io;
use std::path::Path;

use sha2::{Digest, Sha256};

pub use cache::{UpdateCache, check_update_background_if_needed, get_cache_file_path};
pub use client::fetch_latest_release;
pub use replace::{clean_old_executable, extract_binary_from_zip, replace_executable};
pub use semver::Version;

/// Execute the self-update subcommand logic.
pub fn run_update(check_only: bool, force: bool, background: bool) -> io::Result<()> {
    if background {
        run_background_update();
        return Ok(());
    }

    println!("Checking for updates...");
    let release = fetch_latest_release()?;
    let current = env!("CARGO_PKG_VERSION");

    let cur_ver = Version::parse(current).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "Failed to parse current version",
        )
    })?;
    let latest_ver = Version::parse(&release.tag_name)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Failed to parse release tag"))?;

    let is_newer = latest_ver.is_newer_than(&cur_ver);

    if check_only {
        println!("Current version: v{current}");
        println!("Latest version:  {}", release.tag_name);
        if is_newer {
            println!("An update is available! Run 'agent-statusline update' to upgrade.");
        } else {
            println!("agent-statusline is up to date.");
        }
        return Ok(());
    }

    if !is_newer && !force {
        println!("agent-statusline is already up to date (v{current}).");
        return Ok(());
    }

    let target_asset_name = get_target_asset_name()?;
    let bin_name = if cfg!(windows) {
        "agent-statusline.exe"
    } else {
        "agent-statusline"
    };
    let current_exe = std::env::current_exe()?;
    update_executable_from_release(
        &release,
        target_asset_name,
        bin_name,
        &current_exe,
        client::download_asset,
    )?;

    // Update cache
    let cache = UpdateCache {
        last_checked_at: chrono::Utc::now().timestamp(),
        latest_version: release.tag_name.clone(),
        has_update: false,
    };
    let _ = cache.save(&get_cache_file_path());

    println!(
        "Successfully updated agent-statusline to {}!",
        release.tag_name
    );
    Ok(())
}

fn run_background_update() {
    let current = env!("CARGO_PKG_VERSION");
    let cache_path = get_cache_file_path();
    let existing = UpdateCache::load(&cache_path);

    if let Ok(release) = fetch_latest_release() {
        let has_update = match (Version::parse(current), Version::parse(&release.tag_name)) {
            (Some(c), Some(l)) => l.is_newer_than(&c),
            _ => false,
        };
        let cache = UpdateCache {
            last_checked_at: chrono::Utc::now().timestamp(),
            latest_version: release.tag_name,
            has_update,
        };
        let _ = cache.save(&cache_path);
    } else {
        // If API fetch fails (offline, rate limit, GitHub down),
        // ensure last_checked_at is updated to prevent continuous retries on every render
        let cache = existing.unwrap_or_else(|| UpdateCache {
            last_checked_at: chrono::Utc::now().timestamp(),
            latest_version: current.to_string(),
            has_update: false,
        });
        let updated = UpdateCache {
            last_checked_at: chrono::Utc::now().timestamp(),
            ..cache
        };
        let _ = updated.save(&cache_path);
    }
}

pub fn get_target_asset_name() -> io::Result<&'static str> {
    get_asset_name_for(std::env::consts::OS, std::env::consts::ARCH)
}

pub fn get_asset_name_for(os: &str, arch: &str) -> io::Result<&'static str> {
    match (os, arch) {
        ("windows", "x86_64") => Ok("agent-statusline-x86_64-pc-windows-msvc.zip"),
        ("linux", "aarch64") => Ok("agent-statusline-aarch64-unknown-linux-musl.zip"),
        ("linux", "x86_64") => Ok("agent-statusline-x86_64-unknown-linux-musl.zip"),
        _ => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("No release archive for {os}/{arch}"),
        )),
    }
}

fn release_asset<'a>(
    release: &'a client::GitHubRelease,
    name: &str,
) -> io::Result<&'a client::GitHubAsset> {
    release
        .assets
        .iter()
        .find(|asset| asset.name == name)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("Release asset '{name}' not found in {}", release.tag_name),
            )
        })
}

fn verify_archive(archive: &[u8], checksum: &[u8], archive_name: &str) -> io::Result<()> {
    let checksum = std::str::from_utf8(checksum).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Invalid checksum text: {e}"),
        )
    })?;
    let line = checksum.trim_end_matches(['\r', '\n']);
    let (expected, filename) = line.split_once(' ').ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "Checksum must contain a hash and archive basename",
        )
    })?;
    let filename = filename
        .trim_start_matches(' ')
        .strip_prefix('*')
        .unwrap_or(filename.trim_start_matches(' '));
    if expected.len() != 64
        || !expected.bytes().all(|byte| byte.is_ascii_hexdigit())
        || filename != archive_name
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Invalid checksum for '{archive_name}'"),
        ));
    }
    let actual = sha256_hex(archive);
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("SHA-256 mismatch for '{archive_name}'"),
        ));
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

fn update_executable_from_release(
    release: &client::GitHubRelease,
    archive_name: &str,
    binary_name: &str,
    destination: &Path,
    download: impl Fn(&str) -> io::Result<Vec<u8>>,
) -> io::Result<()> {
    let archive_asset = release_asset(release, archive_name)?;
    let checksum_name = format!("{archive_name}.sha256");
    let checksum_asset = release_asset(release, &checksum_name)?;
    let archive = download(&archive_asset.browser_download_url)?;
    let checksum = download(&checksum_asset.browser_download_url)?;
    verify_archive(&archive, &checksum, archive_name)?;
    let binary = extract_binary_from_zip(&archive, binary_name)?;
    replace_executable(destination, &binary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    #[test]
    fn test_asset_names_are_all_zip() {
        let targets = [
            ("windows", "x86_64"),
            ("linux", "aarch64"),
            ("linux", "x86_64"),
        ];

        for (os, arch) in targets {
            let asset = get_asset_name_for(os, arch).unwrap();
            assert!(
                std::path::Path::new(asset)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("zip")),
                "Asset '{asset}' for {os}/{arch} must be a .zip file"
            );
        }

        assert!(
            std::path::Path::new(get_target_asset_name().unwrap())
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
        );
    }

    #[test]
    fn unsupported_targets_are_rejected() {
        for (os, arch) in [
            ("macos", "x86_64"),
            ("windows", "aarch64"),
            ("linux", "arm"),
        ] {
            assert_eq!(
                get_asset_name_for(os, arch).unwrap_err().kind(),
                io::ErrorKind::Unsupported
            );
        }
    }

    #[test]
    fn verified_update_preserves_old_binary_on_invalid_assets() {
        let archive_name = "agent-statusline-x86_64-pc-windows-msvc.zip";
        let mut archive = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(io::Cursor::new(&mut archive));
            writer
                .start_file(
                    "agent-statusline.exe",
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
            writer.write_all(b"new binary").unwrap();
            writer.finish().unwrap();
        }
        let valid_checksum = format!("{} *{archive_name}\n", sha256_hex(&archive));
        let mut no_binary_archive = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(io::Cursor::new(&mut no_binary_archive));
            writer
                .start_file("README.md", zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"no binary").unwrap();
            writer.finish().unwrap();
        }
        let cases = [
            (
                "valid",
                true,
                true,
                archive.clone(),
                valid_checksum.clone(),
                true,
            ),
            (
                "missing archive",
                false,
                true,
                archive.clone(),
                valid_checksum.clone(),
                false,
            ),
            (
                "missing checksum",
                true,
                false,
                archive.clone(),
                valid_checksum.clone(),
                false,
            ),
            (
                "bad checksum",
                true,
                true,
                archive.clone(),
                format!("{} *{archive_name}\n", "0".repeat(64)),
                false,
            ),
            (
                "wrong filename",
                true,
                true,
                archive.clone(),
                format!("{} *other.zip\n", sha256_hex(&archive)),
                false,
            ),
            (
                "missing binary",
                true,
                true,
                no_binary_archive.clone(),
                format!("{} *{archive_name}\n", sha256_hex(&no_binary_archive)),
                false,
            ),
        ];

        for (name, include_archive, include_checksum, bytes, checksum, succeeds) in cases {
            let dir = tempfile::tempdir().unwrap();
            let destination = dir.path().join("agent-statusline.exe");
            fs::write(&destination, b"old binary").unwrap();
            let mut assets = Vec::new();
            if include_archive {
                assets.push(client::GitHubAsset {
                    name: archive_name.to_string(),
                    browser_download_url: "archive".to_string(),
                    size: 0,
                });
            }
            if include_checksum {
                assets.push(client::GitHubAsset {
                    name: format!("{archive_name}.sha256"),
                    browser_download_url: "checksum".to_string(),
                    size: 0,
                });
            }
            let release = client::GitHubRelease {
                tag_name: "v0.2.0".to_string(),
                name: None,
                prerelease: false,
                assets,
            };
            let result = update_executable_from_release(
                &release,
                archive_name,
                "agent-statusline.exe",
                &destination,
                |url| {
                    Ok(if url == "archive" {
                        bytes.clone()
                    } else {
                        checksum.as_bytes().to_vec()
                    })
                },
            );
            assert_eq!(result.is_ok(), succeeds, "{name}: {result:?}");
            assert_eq!(
                fs::read(&destination).unwrap(),
                if succeeds {
                    b"new binary".as_slice()
                } else {
                    b"old binary".as_slice()
                },
                "{name}"
            );
        }
    }
}

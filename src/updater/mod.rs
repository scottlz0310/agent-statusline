pub mod cache;
pub mod client;
pub mod replace;
pub mod semver;

use std::io;

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

    let target_asset_name = get_target_asset_name();
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == target_asset_name)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "Release asset '{target_asset_name}' not found in {}",
                    release.tag_name
                ),
            )
        })?;

    println!(
        "Downloading {} from {}...",
        asset.name, asset.browser_download_url
    );
    let archive_bytes = client::download_asset(&asset.browser_download_url)?;

    println!("Extracting binary...");
    let bin_name = if cfg!(windows) {
        "agent-statusline.exe"
    } else {
        "agent-statusline"
    };
    let new_binary = extract_binary_from_zip(&archive_bytes, bin_name)?;

    let current_exe = std::env::current_exe()?;
    println!(
        "Replacing current executable ({})...",
        current_exe.display()
    );
    replace_executable(&current_exe, &new_binary)?;

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

#[must_use]
pub fn get_target_asset_name() -> &'static str {
    let os = if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "x86_64"
    };
    get_asset_name_for(os, arch)
}

#[must_use]
pub fn get_asset_name_for(os: &str, arch: &str) -> &'static str {
    match (os, arch) {
        ("windows", "x86_64") => "agent-statusline-x86_64-pc-windows-msvc.zip",
        ("linux", "aarch64") => "agent-statusline-aarch64-unknown-linux-musl.zip",
        _ => "agent-statusline-x86_64-unknown-linux-musl.zip",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_asset_names_are_all_zip() {
        let targets = [
            ("windows", "x86_64"),
            ("linux", "aarch64"),
            ("linux", "x86_64"),
        ];

        for (os, arch) in targets {
            let asset = get_asset_name_for(os, arch);
            assert!(
                std::path::Path::new(asset)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("zip")),
                "Asset '{asset}' for {os}/{arch} must be a .zip file"
            );
        }

        assert!(
            std::path::Path::new(get_target_asset_name())
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
        );
    }
}

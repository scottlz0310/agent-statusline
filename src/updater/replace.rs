use std::fs;
use std::io;
use std::path::Path;

/// Replace executable with atomic staging and rollback protection.
///
/// 1. Write `new_binary_bytes` to `current_exe.new`.
/// 2. If `current_exe` exists, move it to `current_exe.old` (Windows permits renaming running exes).
/// 3. Move `current_exe.new` to `current_exe`.
/// 4. If activation fails, rollback `current_exe.old` to `current_exe`.
/// 5. Clean up temporary files in all execution paths.
pub fn replace_executable(current_exe: &Path, new_binary_bytes: &[u8]) -> io::Result<()> {
    let parent = current_exe.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "Parent directory of executable not found",
        )
    })?;

    let file_name = current_exe
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("agent-statusline.exe");

    let stage_path = parent.join(format!("{file_name}.new"));
    let old_path = parent.join(format!("{file_name}.old"));

    // 1. Stage new binary first
    if stage_path.exists() {
        let _ = fs::remove_file(&stage_path);
    }
    fs::write(&stage_path, new_binary_bytes)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&stage_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&stage_path, perms)?;
    }

    let mut backed_up = false;
    let res = (|| -> io::Result<()> {
        // 2. Backup existing binary to .old
        if current_exe.exists() {
            if old_path.exists() {
                let _ = fs::remove_file(&old_path);
            }
            fs::rename(current_exe, &old_path)?;
            backed_up = true;
        }

        // 3. Move staged binary to target
        fs::rename(&stage_path, current_exe)?;
        Ok(())
    })();

    if let Err(err) = res {
        // Rollback on failure
        if backed_up && old_path.exists() && !current_exe.exists() {
            let _ = fs::rename(&old_path, current_exe);
        }
        if stage_path.exists() {
            let _ = fs::remove_file(&stage_path);
        }
        return Err(err);
    }

    if stage_path.exists() {
        let _ = fs::remove_file(&stage_path);
    }

    Ok(())
}

/// Remove any leftover `.old` executable file from prior update sessions.
pub fn clean_old_executable(current_exe: &Path) {
    if let Some(parent) = current_exe.parent()
        && let Some(name) = current_exe.file_name().and_then(|n| n.to_str())
    {
        let old_path = parent.join(format!("{name}.old"));
        if old_path.exists() {
            let _ = fs::remove_file(&old_path);
        }
    }
}

/// Extract specific binary by name from a ZIP archive byte slice.
pub fn extract_binary_from_zip(zip_bytes: &[u8], binary_name: &str) -> io::Result<Vec<u8>> {
    let cursor = io::Cursor::new(zip_bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        let name = file.name();
        if name == binary_name
            || name.ends_with(&format!("/{binary_name}"))
            || name.ends_with(&format!("\\{binary_name}"))
        {
            let cap = usize::try_from(file.size()).unwrap_or(0);
            let mut out = Vec::with_capacity(cap);
            io::copy(&mut file, &mut out)?;
            return Ok(out);
        }
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("Binary '{binary_name}' not found inside archive"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_replace_executable_happy_path() {
        let dir = tempfile::tempdir().unwrap();
        let target_exe = dir.path().join("agent-statusline.exe");

        // Initial binary
        fs::write(&target_exe, b"original-version-1").unwrap();

        // Replace with new binary
        replace_executable(&target_exe, b"updated-version-2").unwrap();

        let updated_content = fs::read_to_string(&target_exe).unwrap();
        assert_eq!(updated_content, "updated-version-2");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&target_exe).unwrap().permissions().mode() & 0o777,
                0o755
            );
        }

        let old_file = dir.path().join("agent-statusline.exe.old");
        assert!(old_file.exists());
        assert_eq!(fs::read_to_string(&old_file).unwrap(), "original-version-1");

        // Clean old
        clean_old_executable(&target_exe);
        assert!(!old_file.exists());
    }

    #[test]
    fn test_replace_executable_from_scratch() {
        let dir = tempfile::tempdir().unwrap();
        let target_exe = dir.path().join("agent-statusline.exe");

        replace_executable(&target_exe, b"first-install").unwrap();
        assert_eq!(fs::read_to_string(&target_exe).unwrap(), "first-install");
    }

    #[test]
    fn test_extract_binary_from_zip() {
        use std::io::Write;
        use zip::write::SimpleFileOptions;

        let mut zip_buf = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(io::Cursor::new(&mut zip_buf));
            zip.start_file("README.md", SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"readme content").unwrap();

            // Windows binary in subdirectory (cargo-dist archive structure)
            zip.start_file(
                "agent-statusline-x86_64-pc-windows-msvc/agent-statusline.exe",
                SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(b"windows-binary-content").unwrap();

            // Linux binary in subdirectory
            zip.start_file(
                "agent-statusline-x86_64-unknown-linux-musl/agent-statusline",
                SimpleFileOptions::default(),
            )
            .unwrap();
            zip.write_all(b"linux-binary-content").unwrap();

            zip.finish().unwrap();
        }

        // Verify Windows binary extraction
        let win_extracted = extract_binary_from_zip(&zip_buf, "agent-statusline.exe").unwrap();
        assert_eq!(win_extracted, b"windows-binary-content");

        // Verify Linux binary extraction
        let linux_extracted = extract_binary_from_zip(&zip_buf, "agent-statusline").unwrap();
        assert_eq!(linux_extracted, b"linux-binary-content");

        // Verify missing binary
        let err = extract_binary_from_zip(&zip_buf, "nonexistent.exe");
        assert!(err.is_err());
    }
}

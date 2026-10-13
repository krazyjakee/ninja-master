//! Runtime ROM discovery; explicit paths take precedence over remembered paths.
use ninja_master_native::{assets::crc32, settings};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(crate) fn remembered_path() -> Option<PathBuf> {
    settings::config_path().map(|path| path.with_file_name("rom-path.txt"))
}

pub(crate) fn resolve(explicit: Option<&Path>) -> PathBuf {
    if let Some(path) = explicit {
        return path.to_owned();
    }
    if let Some(path) = std::env::var_os("NINJA_MASTER_ROM").filter(|p| !p.is_empty()) {
        return path.into();
    }
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join("shinobi3_us.bin"));
        // App bundle: Contents/MacOS -> archive directory alongside the .app.
        if dir.file_name().is_some_and(|name| name == "MacOS")
            && let Some(root) = dir.ancestors().nth(3)
        {
            candidates.push(root.join("shinobi3_us.bin"));
        }
    }
    if let Some(saved) = remembered_path().and_then(|path| fs::read_to_string(path).ok()) {
        candidates.push(PathBuf::from(saved));
    }
    candidates.push(PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../rom/shinobi3_us.bin"
    )));
    first_existing(&candidates)
}

fn first_existing(candidates: &[PathBuf]) -> PathBuf {
    candidates
        .iter()
        .find(|path| path.is_file())
        .unwrap_or_else(|| candidates.last().unwrap())
        .clone()
}

pub(crate) fn read(path: &Path) -> Result<Vec<u8>, String> {
    // Bound the allocation even when the picker selects an unrelated large file.
    let size = fs::metadata(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .len();
    if size != 1_048_576 {
        return Err("Choose a plain 1 MiB Shinobi III US cartridge image (.bin).".into());
    }
    let data = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if data.len() != 1_048_576
        || data.get(0x100..0x104) != Some(b"SEGA")
        || crc32(&data) != 0x5381506f
    {
        return Err("This is not the supported US ROM (CRC32 5381506f). EU, JP and interleaved images are not supported.".into());
    }
    Ok(data)
}

#[cfg(feature = "desktop")]
pub(crate) fn remember(path: &Path) -> Result<(), String> {
    let Some(saved) = remembered_path() else {
        return Ok(());
    };
    let path = fs::canonicalize(path).map_err(|e| e.to_string())?;
    let value = path
        .to_str()
        .ok_or("The ROM path cannot be saved as UTF-8.")?;
    if let Some(parent) = saved.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(saved, value).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rom_selection_rejects_missing_truncated_and_unrecognized_images() {
        let dir = std::env::temp_dir().join(format!("ninja-rom-selection-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("synthetic.bin");
        assert!(read(&dir.join("missing.bin")).is_err());
        fs::write(&file, b"synthetic short file").unwrap();
        assert!(read(&file).unwrap_err().contains("1 MiB"));
        let mut data = vec![0; 1_048_576];
        data[0x100..0x104].copy_from_slice(b"SEGA");
        fs::write(&file, data).unwrap();
        assert!(read(&file).unwrap_err().contains("supported US ROM"));
        assert_eq!(resolve(Some(&file)), file);
        fs::remove_dir_all(dir).unwrap();
    }
}

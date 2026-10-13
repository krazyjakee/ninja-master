//! Bounded file/archive reads. ZIP members are decoded directly, never extracted.
use super::{MANIFEST, MAX_IMAGE_BYTES, MAX_PACK_BYTES, Manifest};
use std::{
    collections::BTreeMap,
    fs,
    io::{Cursor, Read},
    path::{Component, Path, PathBuf},
};
use zip::ZipArchive;

const MAX_MANIFEST_BYTES: usize = 16 * 1024 * 1024;
const MAX_PNG_BYTES: usize = MAX_IMAGE_BYTES * 2 + 1024 * 1024;

pub(super) enum Source {
    Directory {
        root: PathBuf,
        manifest: PathBuf,
    },
    Zip {
        archive: ZipArchive<fs::File>,
        entries: BTreeMap<String, usize>,
        root: String,
        manifest: String,
    },
}

pub(super) fn is_zip(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
}

fn relative(name: &str) -> Result<&Path, String> {
    let path = Path::new(name);
    if name.is_empty()
        || name.contains(['\\', ':'])
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        || name
            .split('/')
            .any(|c| c.is_empty() || c == "." || c == "..")
    {
        return Err(format!("pack files must use relative paths: {name}"));
    }
    Ok(path)
}

fn read_bounded(reader: impl Read, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err(format!("pack file exceeds {} MiB", limit / 1024 / 1024));
    }
    Ok(bytes)
}

impl Source {
    pub(super) fn open(path: &Path) -> Result<Self, String> {
        if is_zip(path) && !path.is_dir() {
            if fs::metadata(path).map_err(|e| e.to_string())?.len() > MAX_PACK_BYTES as u64 {
                return Err("ZIP exceeds 1 GiB".into());
            }
            let mut archive = ZipArchive::new(fs::File::open(path).map_err(|e| e.to_string())?)
                .map_err(|e| format!("ZIP: {e}"))?;
            if archive.len() > 100_000 {
                return Err("ZIP contains too many entries".into());
            }
            let mut entries = BTreeMap::new();
            let mut manifests = Vec::new();
            for i in 0..archive.len() {
                let file = archive.by_index(i).map_err(|e| format!("ZIP: {e}"))?;
                let name = file.name().trim_end_matches('/');
                relative(name)?;
                if file.is_symlink() {
                    return Err("ZIP cannot contain symbolic links".into());
                }
                if file.is_dir() {
                    continue;
                }
                if entries.insert(name.to_owned(), i).is_some() {
                    return Err(format!("duplicate ZIP entry: {name}"));
                }
                if name == MANIFEST || name.ends_with(&format!("/{MANIFEST}")) {
                    manifests.push(name.to_owned());
                }
            }
            if manifests.len() != 1 {
                return Err(
                    "ZIP must contain exactly one pack.json, at its root or inside a pack folder"
                        .into(),
                );
            }
            let manifest = manifests.pop().unwrap();
            let root = manifest.strip_suffix(MANIFEST).unwrap().to_owned();
            Ok(Self::Zip {
                archive,
                entries,
                root,
                manifest,
            })
        } else {
            let manifest = if path.is_dir() {
                path.join(MANIFEST)
            } else {
                path.to_owned()
            };
            let root = fs::canonicalize(
                manifest
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or(Path::new(".")),
            )
            .map_err(|e| e.to_string())?;
            Ok(Self::Directory { root, manifest })
        }
    }

    pub(super) fn manifest(&mut self) -> Result<Manifest, String> {
        let bytes = match self {
            Self::Directory { manifest, .. } => read_bounded(
                fs::File::open(manifest).map_err(|e| e.to_string())?,
                MAX_MANIFEST_BYTES,
            )?,
            Self::Zip {
                archive,
                entries,
                manifest,
                ..
            } => {
                let index = entries[manifest];
                let file = archive.by_index(index).map_err(|e| format!("ZIP: {e}"))?;
                read_bounded(file, MAX_MANIFEST_BYTES)?
            }
        };
        let manifest: Manifest =
            serde_json::from_slice(&bytes).map_err(|e| format!("pack.json: {e}"))?;
        if manifest.version != 1 {
            return Err(format!("unsupported art pack version {}", manifest.version));
        }
        if manifest.name.contains('\0') {
            return Err("pack name cannot contain a null character".into());
        }
        // Discovery validates references without decoding potentially large PNGs.
        for sheet in &manifest.sheets {
            self.location(&sheet.file)?;
        }
        for sheet in &manifest.player_frames {
            self.location(&sheet.file)?;
        }
        Ok(manifest)
    }

    fn location(&self, name: &str) -> Result<String, String> {
        let relative = relative(name)?;
        match self {
            Self::Directory { root, .. } => {
                let path =
                    fs::canonicalize(root.join(relative)).map_err(|e| format!("{name}: {e}"))?;
                if !path.starts_with(root) || !path.is_file() {
                    return Err(format!(
                        "sheet leaves pack directory or is not a file: {name}"
                    ));
                }
                Ok(name.to_owned())
            }
            Self::Zip { entries, root, .. } => {
                let key = format!("{root}{name}");
                if !entries.contains_key(&key) {
                    return Err(format!("ZIP sheet is missing: {name}"));
                }
                Ok(key)
            }
        }
    }

    pub(super) fn image(&mut self, name: &str) -> Result<(usize, usize, Vec<[u8; 4]>), String> {
        let location = self.location(name)?;
        match self {
            Self::Directory { root, .. } => super::read_png(&root.join(location)),
            Self::Zip {
                archive, entries, ..
            } => {
                let file = archive
                    .by_index(entries[&location])
                    .map_err(|e| format!("ZIP: {e}"))?;
                let bytes = read_bounded(file, MAX_PNG_BYTES)?;
                super::decode_png(Cursor::new(bytes))
            }
        }
    }
}

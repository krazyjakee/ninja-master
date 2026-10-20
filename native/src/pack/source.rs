//! Bounded file/archive reads. ZIP members are decoded directly, never extracted.
use super::Kind;
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};
use zip::ZipArchive;

const MAX_MANIFEST_BYTES: usize = 16 * 1024 * 1024;

pub enum Source {
    Directory {
        kind: &'static Kind,
        root: PathBuf,
        manifest: PathBuf,
    },
    Zip {
        kind: &'static Kind,
        archive: ZipArchive<fs::File>,
        entries: BTreeMap<String, usize>,
        root: String,
        manifest: String,
    },
}

/// A pack file: a path inside a pack folder, or a ZIP member's bytes.
pub enum Member {
    File(PathBuf),
    Bytes(Vec<u8>),
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

pub fn read_bounded(reader: impl Read, limit: usize) -> Result<Vec<u8>, String> {
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
    /// A pack directory, its manifest, or a ZIP with one root/wrapped manifest.
    pub fn open(path: &Path, kind: &'static Kind) -> Result<Self, String> {
        let name = kind.manifest;
        if is_zip(path) && !path.is_dir() {
            if fs::metadata(path).map_err(|e| e.to_string())?.len() > kind.max_bytes as u64 {
                return Err(format!("ZIP exceeds {} MiB", kind.max_bytes / 1024 / 1024));
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
                let member = file.name().trim_end_matches('/');
                relative(member)?;
                if file.is_symlink() {
                    return Err("ZIP cannot contain symbolic links".into());
                }
                if file.is_dir() {
                    continue;
                }
                if entries.insert(member.to_owned(), i).is_some() {
                    return Err(format!("duplicate ZIP entry: {member}"));
                }
                if member == name || member.ends_with(&format!("/{name}")) {
                    manifests.push(member.to_owned());
                }
            }
            if manifests.len() != 1 {
                return Err(format!(
                    "ZIP must contain exactly one {name}, at its root or inside a pack folder"
                ));
            }
            let manifest = manifests.pop().unwrap();
            let root = manifest.strip_suffix(name).unwrap().to_owned();
            Ok(Self::Zip {
                kind,
                archive,
                entries,
                root,
                manifest,
            })
        } else {
            let manifest = if path.is_dir() {
                path.join(name)
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
            Ok(Self::Directory {
                kind,
                root,
                manifest,
            })
        }
    }

    pub fn kind(&self) -> &'static Kind {
        match self {
            Self::Directory { kind, .. } | Self::Zip { kind, .. } => kind,
        }
    }

    pub fn manifest_bytes(&mut self) -> Result<Vec<u8>, String> {
        match self {
            Self::Directory { manifest, .. } => read_bounded(
                fs::File::open(manifest).map_err(|e| e.to_string())?,
                MAX_MANIFEST_BYTES,
            ),
            Self::Zip {
                archive,
                entries,
                manifest,
                ..
            } => {
                let index = entries[manifest];
                let file = archive.by_index(index).map_err(|e| format!("ZIP: {e}"))?;
                read_bounded(file, MAX_MANIFEST_BYTES)
            }
        }
    }

    /// Check that `name` stays inside the pack and exists, without reading it.
    pub fn location(&self, name: &str) -> Result<String, String> {
        let relative = relative(name)?;
        match self {
            Self::Directory { root, .. } => {
                let path =
                    fs::canonicalize(root.join(relative)).map_err(|e| format!("{name}: {e}"))?;
                if !path.starts_with(root) || !path.is_file() {
                    return Err(format!(
                        "file leaves pack directory or is not a file: {name}"
                    ));
                }
                Ok(name.to_owned())
            }
            Self::Zip { entries, root, .. } => {
                let key = format!("{root}{name}");
                if !entries.contains_key(&key) {
                    return Err(format!("ZIP file is missing: {name}"));
                }
                Ok(key)
            }
        }
    }

    /// A folder file's path, or up to `limit` bytes of a ZIP member.
    pub fn member(&mut self, name: &str, limit: usize) -> Result<Member, String> {
        let location = self.location(name)?;
        match self {
            Self::Directory { root, .. } => Ok(Member::File(root.join(location))),
            Self::Zip {
                archive, entries, ..
            } => {
                let file = archive
                    .by_index(entries[&location])
                    .map_err(|e| format!("ZIP: {e}"))?;
                read_bounded(file, limit).map(Member::Bytes)
            }
        }
    }

    /// Up to `limit` bytes of a pack file.
    pub fn read(&mut self, name: &str, limit: usize) -> Result<Vec<u8>, String> {
        match self.member(name, limit)? {
            Member::File(path) => {
                read_bounded(fs::File::open(path).map_err(|e| e.to_string())?, limit)
            }
            Member::Bytes(bytes) => Ok(bytes),
        }
    }
}

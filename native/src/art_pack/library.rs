//! A fixed drop-in folder containing pack directories and ZIP files.
use super::{
    MANIFEST,
    source::{Source, is_zip},
};
use std::{
    fs,
    path::{Component, Path, PathBuf},
    time::SystemTime,
};

/// Override the normal portable/source pack folder.
pub const LIBRARY_ENV: &str = "NINJA_MASTER_ART_PACKS";

pub fn library_directory() -> PathBuf {
    if let Some(path) = std::env::var_os(LIBRARY_ENV).filter(|p| !p.is_empty()) {
        let path = PathBuf::from(path);
        return if path.is_absolute() {
            path
        } else {
            std::env::current_dir().unwrap_or_default().join(path)
        };
    }
    let checkout = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("art-packs");
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("ninja-master-native"));
    directory_for(&exe, &checkout)
}

/// New selections use library-relative names; existing absolute paths still work.
pub fn resolve_selection(root: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        return path.to_owned();
    }
    let installed = root.join(path);
    if installed.exists() || !path.exists() {
        installed
    } else {
        path.to_owned()
    }
}

pub fn selection_value(root: &Path, path: &Path) -> String {
    let root = fs::canonicalize(root).unwrap_or_else(|_| root.to_owned());
    let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
    path.strip_prefix(root)
        .unwrap_or(&path)
        .to_string_lossy()
        .into_owned()
}

fn directory_for(exe: &Path, checkout: &Path) -> PathBuf {
    let dir = exe.parent().unwrap_or(Path::new("."));
    let dir = if dir.file_name().is_some_and(|n| n == "MacOS") {
        dir.ancestors().nth(3).unwrap_or(dir)
    } else {
        dir
    };
    let portable = dir.join("art-packs");
    if portable.is_dir() || dir.join("release.json").is_file() || !checkout.is_dir() {
        portable
    } else {
        checkout.to_owned()
    }
}

#[derive(Clone, Debug)]
pub struct PackEntry {
    pub name: String,
    pub path: PathBuf,
}

impl PackEntry {
    pub fn read(path: &Path) -> Result<Self, String> {
        let manifest = Source::open(path)?.manifest()?;
        let name = if manifest.name.trim().is_empty() {
            path.file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        } else {
            manifest.name
        };
        let path = if !path.is_dir() && path.file_name().is_some_and(|n| n == MANIFEST) {
            path.parent().unwrap_or(path)
        } else {
            path
        };
        Ok(Self {
            name,
            path: fs::canonicalize(path).map_err(|e| e.to_string())?,
        })
    }

    /// Filenames distinguish multiple editions with the same manifest title.
    pub fn label(&self) -> String {
        format!(
            "{} ({})",
            self.name,
            self.path.file_name().unwrap_or_default().to_string_lossy()
        )
    }

    pub fn matches(&self, selected: &Path) -> bool {
        let path = if selected.file_name().is_some_and(|n| n == MANIFEST) {
            selected.parent().unwrap_or(selected)
        } else {
            selected
        };
        fs::canonicalize(path).is_ok_and(|p| p == self.path)
    }
}

#[derive(PartialEq, Eq)]
struct Signature {
    path: PathBuf,
    size: u64,
    modified: Option<SystemTime>,
}

pub struct Library {
    pub root: PathBuf,
    pub packs: Vec<PackEntry>,
    pub errors: Vec<String>,
    signature: Option<Vec<Signature>>,
}

impl Library {
    pub fn rescan(&mut self) {
        self.signature = None;
        self.refresh();
    }
    pub fn new(root: PathBuf) -> Self {
        let mut library = Self {
            root,
            packs: Vec::new(),
            errors: Vec::new(),
            signature: None,
        };
        library.refresh();
        library
    }

    /// Rescans on changes only: large manifests are not parsed every UI frame.
    pub fn refresh(&mut self) {
        let result = (|| {
            fs::create_dir_all(&self.root).map_err(|e| e.to_string())?;
            let mut signature = Vec::new();
            for entry in fs::read_dir(&self.root).map_err(|e| e.to_string())? {
                let path = entry.map_err(|e| e.to_string())?.path();
                let source = if path.is_dir() && path.join(MANIFEST).is_file() {
                    path.join(MANIFEST)
                } else if path.is_file() && is_zip(&path) {
                    path.clone()
                } else {
                    continue;
                };
                let metadata = fs::metadata(source).map_err(|e| e.to_string())?;
                signature.push(Signature {
                    path,
                    size: metadata.len(),
                    modified: metadata.modified().ok(),
                });
            }
            signature.sort_by(|a, b| a.path.cmp(&b.path));
            Ok::<_, String>(signature)
        })();
        let signature = match result {
            Ok(signature) => signature,
            Err(error) => {
                self.errors = vec![format!("Could not read pack folder: {error}")];
                self.signature = None;
                return;
            }
        };
        if self.signature.as_ref() == Some(&signature) && self.errors.is_empty() {
            return;
        }
        self.packs.clear();
        self.errors.clear();
        for candidate in &signature {
            match PackEntry::read(&candidate.path) {
                Ok(pack) => self.packs.push(pack),
                Err(error) => self.errors.push(format!(
                    "{}: {error}",
                    candidate
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                )),
            }
        }
        self.packs.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.path.cmp(&b.path))
        });
        self.signature = Some(signature);
    }

    pub fn template_path(&self, name: &str) -> Result<PathBuf, String> {
        let name = name.trim();
        let path = Path::new(name);
        if name.is_empty()
            || name.contains(['/', '\\', ':', '\0'])
            || path.components().count() != 1
            || !matches!(path.components().next(), Some(Component::Normal(_)))
        {
            return Err("Choose a folder name inside the art-packs folder.".into());
        }
        Ok(self.root.join(path))
    }
}

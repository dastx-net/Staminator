//! Versioned, portable schema for user-defined physical keyboard geometry.

use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    path::Path,
};

pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_FILE_BYTES: u64 = 1_048_576;
pub const MAX_KEYS: usize = 256;
pub const MAX_CANVAS_SIZE: f64 = 4096.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyboardSchema {
    pub version: u32,
    pub name: String,
    pub canvas: Canvas,
    pub keys: Vec<Key>,
    #[serde(
        default = "default_show_finger_colors",
        skip_serializing_if = "is_true"
    )]
    pub show_finger_colors: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<KeyboardStyle>,
}

fn default_show_finger_colors() -> bool {
    true
}
fn is_true(value: &bool) -> bool {
    *value
}

/// Optional visual styling, independent of physical layout and finger assignments.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct KeyboardStyle {
    pub background_color: Option<String>,
    pub text_color: Option<String>,
    pub active_color: Option<String>,
    pub active_border_color: Option<String>,
    pub shadow_color: Option<String>,
    pub key_corner_radius: Option<f32>,
    pub font: Option<KeyboardFont>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyboardFont {
    Soft,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Canvas {
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Key {
    /// Stable `KeyboardEvent.code` style identifier, independent of language
    /// (for example `KeyA`, `Digit1`, `Enter`, `NumpadEnter`, or `IntlBackslash`).
    pub physical_id: String,
    pub geometry: Geometry,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shifted_label: Option<String>,
    /// Optional #RRGGBB or #RRGGBBAA color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finger: Option<Finger>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case", deny_unknown_fields)]
pub enum Geometry {
    Rect {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    },
    Polygon {
        points: Vec<Point>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Finger {
    LeftLittle,
    LeftRing,
    LeftMiddle,
    LeftIndex,
    LeftThumb,
    RightThumb,
    RightIndex,
    RightMiddle,
    RightRing,
    RightLittle,
}

#[derive(Debug)]
pub enum SchemaError {
    Io(io::Error),
    TooLarge,
    Json(serde_json::Error),
    Invalid(String),
}

impl std::fmt::Display for SchemaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "keyboard schema I/O error: {e}"),
            Self::TooLarge => write!(f, "keyboard schema exceeds {MAX_FILE_BYTES} bytes"),
            Self::Json(e) => write!(f, "invalid keyboard schema JSON: {e}"),
            Self::Invalid(e) => write!(f, "invalid keyboard schema: {e}"),
        }
    }
}
impl std::error::Error for SchemaError {}
impl From<io::Error> for SchemaError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<serde_json::Error> for SchemaError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

impl KeyboardSchema {
    pub fn validate(&self) -> Result<(), SchemaError> {
        if self.version != SCHEMA_VERSION {
            return Err(invalid(format!("unsupported version {}", self.version)));
        }
        if self.name.trim().is_empty() || self.name.len() > 128 {
            return Err(invalid("name must be 1 to 128 bytes"));
        }
        if !valid_dimension(self.canvas.width) || !valid_dimension(self.canvas.height) {
            return Err(invalid(
                "canvas dimensions must be finite and between 0 and 4096",
            ));
        }
        if let Some(style) = &self.style {
            for value in [
                &style.background_color,
                &style.text_color,
                &style.active_color,
                &style.active_border_color,
                &style.shadow_color,
            ]
            .into_iter()
            .flatten()
            {
                if !valid_color(value) {
                    return Err(invalid("style colors must be #RRGGBB or #RRGGBBAA"));
                }
            }
            if style
                .key_corner_radius
                .is_some_and(|radius| !radius.is_finite() || !(0.0..=0.5).contains(&radius))
            {
                return Err(invalid("key corner radius must be between 0 and 0.5"));
            }
        }
        if self.keys.len() > MAX_KEYS {
            return Err(invalid(format!("more than {MAX_KEYS} keys")));
        }
        let mut ids = HashSet::with_capacity(self.keys.len());
        for (index, key) in self.keys.iter().enumerate() {
            if !valid_physical_id(&key.physical_id) {
                return Err(invalid(format!(
                    "key {index} physical_id must be a supported KeyboardEvent.code identifier"
                )));
            }
            if !ids.insert(&key.physical_id) {
                return Err(invalid(format!(
                    "duplicate physical_id {:?}",
                    key.physical_id
                )));
            }
            for (label_name, label) in
                [("label", &key.label), ("shifted_label", &key.shifted_label)]
            {
                if label.as_ref().is_some_and(|s| s.len() > 128) {
                    return Err(invalid(format!("key {index} {label_name} is too long")));
                }
            }
            if let Some(color) = &key.color
                && !valid_color(color)
            {
                return Err(invalid(format!(
                    "key {index} color must be #RRGGBB or #RRGGBBAA"
                )));
            }
            match &key.geometry {
                Geometry::Rect {
                    x,
                    y,
                    width,
                    height,
                } => {
                    if !valid_point(*x, *y)
                        || !valid_dimension(*width)
                        || !valid_dimension(*height)
                        || *width == 0.0
                        || *height == 0.0
                        || x + width > self.canvas.width
                        || y + height > self.canvas.height
                    {
                        return Err(invalid(format!(
                            "key {index} rectangle is invalid or outside canvas"
                        )));
                    }
                }
                Geometry::Polygon { points } => {
                    if points.len() < 3 || points.len() > 64 {
                        return Err(invalid(format!("key {index} polygon needs 3 to 64 points")));
                    }
                    if points.iter().any(|p| {
                        !valid_point(p.x, p.y)
                            || p.x > self.canvas.width
                            || p.y > self.canvas.height
                    }) {
                        return Err(invalid(format!(
                            "key {index} polygon has invalid or out-of-canvas point"
                        )));
                    }
                    for i in 0..points.len() {
                        for j in 0..i {
                            if points[i].x == points[j].x && points[i].y == points[j].y {
                                return Err(invalid(format!(
                                    "key {index} polygon has duplicate vertices"
                                )));
                            }
                        }
                    }
                    for i in 0..points.len() {
                        for j in (i + 1)..points.len() {
                            if j == i + 1 || (i == 0 && j == points.len() - 1) {
                                continue;
                            }
                            let a = points[i];
                            let b = points[(i + 1) % points.len()];
                            let c = points[j];
                            let d = points[(j + 1) % points.len()];
                            if segments_intersect(a, b, c, d) {
                                return Err(invalid(format!(
                                    "key {index} polygon is self-intersecting"
                                )));
                            }
                        }
                    }
                    let area: f64 = (0..points.len())
                        .map(|i| {
                            let a = points[i];
                            let b = points[(i + 1) % points.len()];
                            a.x * b.y - b.x * a.y
                        })
                        .sum();
                    if !area.is_finite() || area.abs() < f64::EPSILON {
                        return Err(invalid(format!("key {index} polygon has zero area")));
                    }
                }
            }
        }
        Ok(())
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, SchemaError> {
        let file = fs::File::open(path)?;
        if file.metadata()?.len() > MAX_FILE_BYTES {
            return Err(SchemaError::TooLarge);
        }
        let mut bytes = Vec::new();
        file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(SchemaError::TooLarge);
        }
        let schema: Self = serde_json::from_slice(&bytes)?;
        schema.validate()?;
        Ok(schema)
    }

    /// Writes a new file only. Existing files are never truncated or replaced.
    pub fn save_new(&self, path: impl AsRef<Path>) -> Result<(), SchemaError> {
        self.validate()?;
        let bytes = serde_json::to_vec_pretty(self)?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(SchemaError::TooLarge);
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        if let Err(error) = file.write_all(&bytes).and_then(|_| file.flush()) {
            drop(file);
            let _ = fs::remove_file(&path);
            return Err(SchemaError::Io(error));
        }
        Ok(())
    }
}

/// File-backed collection of validated keyboard schemas.
/// Each collection entry is a directory containing `keyboard.json`.
pub mod storage {
    use super::{KeyboardSchema, MAX_FILE_BYTES, SchemaError};
    use std::{
        fs::{self, File, OpenOptions},
        io::{self, Read, Write},
        path::{Component, Path, PathBuf},
    };
    use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

    pub const MAX_ARCHIVE_BYTES: u64 = 2 * 1024 * 1024;
    pub const MAX_ARCHIVE_FILES: usize = 16;
    pub const MAX_ARCHIVE_UNPACKED_BYTES: u64 = 2 * 1024 * 1024;
    const SCHEMA_FILENAME: &str = "keyboard.json";

    #[derive(Debug)]
    pub enum StorageError {
        Io(io::Error),
        Schema(SchemaError),
        Archive(String),
    }
    impl std::fmt::Display for StorageError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::Io(e) => write!(f, "keyboard collection I/O error: {e}"),
                Self::Schema(e) => write!(f, "invalid keyboard: {e}"),
                Self::Archive(e) => write!(f, "invalid keyboard archive: {e}"),
            }
        }
    }
    impl std::error::Error for StorageError {}
    impl From<io::Error> for StorageError {
        fn from(e: io::Error) -> Self {
            Self::Io(e)
        }
    }
    impl From<SchemaError> for StorageError {
        fn from(e: SchemaError) -> Self {
            Self::Schema(e)
        }
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct KeyboardEntry {
        /// Stable collection directory name, suitable for passing to `remove`.
        pub id: String,
        pub name: String,
    }

    #[derive(Clone, Debug)]
    pub struct KeyboardCollection {
        root: PathBuf,
    }

    impl KeyboardCollection {
        pub fn new(root: impl Into<PathBuf>) -> Result<Self, StorageError> {
            let root = root.into();
            fs::create_dir_all(&root)?;
            if fs::symlink_metadata(&root)?.file_type().is_symlink() {
                return Err(StorageError::Archive(
                    "collection root cannot be a symlink".into(),
                ));
            }
            Ok(Self { root })
        }

        /// Lists valid entries; IDs use only ASCII letters, digits, `_` and `-`.
        pub fn list(&self) -> Result<Vec<KeyboardEntry>, StorageError> {
            let mut entries = Vec::new();
            for item in fs::read_dir(&self.root)? {
                let item = item?;
                let ty = item.file_type()?;
                if !ty.is_dir() || ty.is_symlink() {
                    continue;
                }
                let id = item.file_name().to_string_lossy().into_owned();
                if validate_id(&id).is_err() {
                    continue;
                }
                let path = item.path().join(SCHEMA_FILENAME);
                let Ok(schema_type) = fs::symlink_metadata(&path) else {
                    continue;
                };
                if !schema_type.is_file() || schema_type.file_type().is_symlink() {
                    continue;
                }
                if let Ok(schema) = KeyboardSchema::load(path) {
                    entries.push(KeyboardEntry {
                        id,
                        name: schema.name,
                    });
                }
            }
            entries.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
            Ok(entries)
        }

        pub fn load(&self, id: &str) -> Result<KeyboardSchema, StorageError> {
            validate_id(id)?;
            let dir = self.root.join(id);
            let ty = fs::symlink_metadata(&dir)?.file_type();
            if !ty.is_dir() || ty.is_symlink() {
                return Err(StorageError::Archive("invalid collection entry".into()));
            }
            let schema_path = dir.join(SCHEMA_FILENAME);
            let schema_type = fs::symlink_metadata(&schema_path)?;
            if !schema_type.is_file() || schema_type.file_type().is_symlink() {
                return Err(StorageError::Archive(
                    "keyboard.json must be a regular file".into(),
                ));
            }
            Ok(KeyboardSchema::load(schema_path)?)
        }

        pub fn save_new(&self, schema: &KeyboardSchema) -> Result<KeyboardEntry, StorageError> {
            schema.validate()?;
            let bytes = serde_json::to_vec_pretty(schema).map_err(SchemaError::from)?;
            if bytes.len() as u64 > MAX_FILE_BYTES {
                return Err(StorageError::Schema(SchemaError::TooLarge));
            }
            let base = safe_dir_name(&schema.name);
            let (id, dir) = (0u32..)
                .find_map(|n| {
                    let id = if n == 0 {
                        base.clone()
                    } else {
                        format!("{base}-{n}")
                    };
                    let path = self.root.join(&id);
                    match fs::create_dir(&path) {
                        Ok(()) => Some(Ok((id, path))),
                        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => None,
                        Err(e) => Some(Err(StorageError::Io(e))),
                    }
                })
                .expect("unbounded collision search eventually finds a free name")?;
            let path = dir.join(SCHEMA_FILENAME);
            let file_result = OpenOptions::new().write(true).create_new(true).open(&path);
            let mut file = match file_result {
                Ok(file) => file,
                Err(error) => {
                    let _ = fs::remove_dir_all(&dir);
                    return Err(StorageError::Io(error));
                }
            };
            if let Err(e) = file.write_all(&bytes).and_then(|_| file.flush()) {
                drop(file);
                let _ = fs::remove_dir_all(&dir);
                return Err(StorageError::Io(e));
            }
            Ok(KeyboardEntry {
                id,
                name: schema.name.clone(),
            })
        }

        /// Imports one `keyboard.json` file from an archive into a newly created directory.
        /// The schema is validated before any collection entry is created.
        pub fn import_zip(
            &self,
            archive_path: impl AsRef<Path>,
        ) -> Result<KeyboardEntry, StorageError> {
            let archive_path = archive_path.as_ref();
            let size = fs::metadata(archive_path)?.len();
            if size > MAX_ARCHIVE_BYTES {
                return Err(StorageError::Archive(
                    "compressed archive exceeds size limit".into(),
                ));
            }
            let file = File::open(archive_path)?;
            let mut archive =
                ZipArchive::new(file).map_err(|e| StorageError::Archive(e.to_string()))?;
            if archive.is_empty() || archive.len() > MAX_ARCHIVE_FILES {
                return Err(StorageError::Archive(
                    "archive file count is outside limits".into(),
                ));
            }
            let mut files: Vec<(PathBuf, Vec<u8>, bool)> = Vec::new();
            let mut seen = std::collections::HashSet::new();
            let mut schema_bytes = None;
            let mut total = 0u64;
            for i in 0..archive.len() {
                let item = archive
                    .by_index(i)
                    .map_err(|e| StorageError::Archive(e.to_string()))?;
                let path = item
                    .enclosed_name()
                    .ok_or_else(|| StorageError::Archive("unsafe archive path".into()))?;
                if path.as_os_str().is_empty()
                    || path
                        .components()
                        .any(|c| !matches!(c, Component::Normal(_)))
                {
                    return Err(StorageError::Archive("unsafe archive path".into()));
                }
                let key = path.to_string_lossy().replace('\\', "/").to_lowercase();
                if !seen.insert(key) {
                    return Err(StorageError::Archive("duplicate archive path".into()));
                }
                if item
                    .unix_mode()
                    .is_some_and(|mode| mode & 0o170000 == 0o120000)
                {
                    return Err(StorageError::Archive(
                        "symbolic links are not supported".into(),
                    ));
                }
                let is_dir = item.is_dir();
                let expected_size = item.size();
                total = total
                    .checked_add(expected_size)
                    .ok_or_else(|| StorageError::Archive("unpacked size overflow".into()))?;
                if expected_size > MAX_FILE_BYTES || total > MAX_ARCHIVE_UNPACKED_BYTES {
                    return Err(StorageError::Archive(
                        "unpacked archive exceeds size limit".into(),
                    ));
                }
                let mut bytes = Vec::with_capacity(expected_size as usize);
                if !is_dir {
                    item.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
                }
                if bytes.len() as u64 > MAX_FILE_BYTES || bytes.len() as u64 != expected_size {
                    return Err(StorageError::Archive(
                        "invalid or oversized schema entry".into(),
                    ));
                }
                if path == Path::new(SCHEMA_FILENAME)
                    && !is_dir
                    && schema_bytes.replace(bytes.clone()).is_some()
                {
                    return Err(StorageError::Archive("duplicate schema".into()));
                }
                files.push((path, bytes, is_dir));
            }
            let bytes = schema_bytes
                .ok_or_else(|| StorageError::Archive("keyboard.json is missing".into()))?;
            let schema: KeyboardSchema =
                serde_json::from_slice(&bytes).map_err(SchemaError::from)?;
            schema.validate()?;
            let base = safe_dir_name(&schema.name);
            let (id, dir) = (0u32..)
                .find_map(|n| {
                    let id = if n == 0 {
                        base.clone()
                    } else {
                        format!("{base}-{n}")
                    };
                    let path = self.root.join(&id);
                    match fs::create_dir(&path) {
                        Ok(()) => Some(Ok((id, path))),
                        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => None,
                        Err(e) => Some(Err(StorageError::Io(e))),
                    }
                })
                .expect("unbounded collision search eventually finds a free name")?;
            let write_result = (|| -> Result<(), StorageError> {
                for (relative, contents, is_dir) in &files {
                    let destination = dir.join(relative);
                    if *is_dir {
                        fs::create_dir_all(&destination)?;
                    } else {
                        if let Some(parent) = destination.parent() {
                            fs::create_dir_all(parent)?;
                        }
                        let mut output = OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(destination)?;
                        output.write_all(contents)?;
                    }
                }
                Ok(())
            })();
            if let Err(error) = write_result {
                let _ = fs::remove_dir_all(&dir);
                return Err(error);
            }
            Ok(KeyboardEntry {
                id,
                name: schema.name,
            })
        }

        pub fn remove(&self, id: &str) -> Result<(), StorageError> {
            validate_id(id)?;
            let dir = self.root.join(id);
            let ty = fs::symlink_metadata(&dir)?.file_type();
            if !ty.is_dir() || ty.is_symlink() {
                return Err(StorageError::Archive(
                    "entry is not a collection directory".into(),
                ));
            }
            fs::remove_dir_all(dir)?;
            Ok(())
        }

        pub fn export_zip(
            &self,
            id: &str,
            output_path: impl AsRef<Path>,
        ) -> Result<(), StorageError> {
            validate_id(id)?;
            let dir = self.root.join(id);
            let ty = fs::symlink_metadata(&dir)?.file_type();
            if !ty.is_dir() || ty.is_symlink() {
                return Err(StorageError::Archive("invalid collection entry".into()));
            }
            let schema_path = dir.join(SCHEMA_FILENAME);
            let schema_type = fs::symlink_metadata(&schema_path)?;
            if !schema_type.is_file() || schema_type.file_type().is_symlink() {
                return Err(StorageError::Archive(
                    "keyboard.json must be a regular file".into(),
                ));
            }
            let _schema = KeyboardSchema::load(&schema_path)?;
            let output_path = output_path.as_ref();
            let output_parent = output_path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let canonical_entry = fs::canonicalize(&dir)?;
            let canonical_parent = fs::canonicalize(output_parent)?;
            if canonical_parent
                .join(
                    output_path.file_name().ok_or_else(|| {
                        StorageError::Archive("export path has no filename".into())
                    })?,
                )
                .starts_with(&canonical_entry)
            {
                return Err(StorageError::Archive(
                    "export destination cannot be inside the collection entry".into(),
                ));
            }
            let output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(output_path)?;
            let mut zip = ZipWriter::new(output);
            let mut count = 0usize;
            let mut total = 0u64;
            if let Err(error) = add_tree_to_zip(&mut zip, &dir, &dir, &mut count, &mut total) {
                drop(zip);
                let _ = fs::remove_file(output_path);
                return Err(error);
            }
            if let Err(error) = zip.finish() {
                let _ = fs::remove_file(output_path);
                return Err(StorageError::Archive(error.to_string()));
            }
            if fs::metadata(output_path)?.len() > MAX_ARCHIVE_BYTES {
                let _ = fs::remove_file(output_path);
                return Err(StorageError::Archive(
                    "exported archive exceeds compressed size limit".into(),
                ));
            }
            Ok(())
        }
    }

    fn validate_id(id: &str) -> Result<(), StorageError> {
        if id.is_empty()
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(StorageError::Archive("invalid collection entry ID".into()));
        }
        Ok(())
    }

    fn add_tree_to_zip<W: Write + io::Seek>(
        zip: &mut ZipWriter<W>,
        root: &Path,
        current: &Path,
        count: &mut usize,
        total: &mut u64,
    ) -> Result<(), StorageError> {
        for entry in fs::read_dir(current)? {
            let entry = entry?;
            let ty = entry.file_type()?;
            if ty.is_symlink() {
                return Err(StorageError::Archive(
                    "collection contains a symbolic link".into(),
                ));
            }
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .map_err(|e| StorageError::Archive(e.to_string()))?;
            let name = relative.to_string_lossy().replace('\\', "/");
            *count += 1;
            if *count > MAX_ARCHIVE_FILES {
                return Err(StorageError::Archive(
                    "collection has too many files".into(),
                ));
            }
            if ty.is_dir() {
                zip.add_directory(format!("{name}/"), SimpleFileOptions::default())
                    .map_err(|e| StorageError::Archive(e.to_string()))?;
                add_tree_to_zip(zip, root, &path, count, total)?;
            } else if ty.is_file() {
                let expected_size = entry.metadata()?.len();
                if expected_size > MAX_FILE_BYTES {
                    return Err(StorageError::Archive(
                        "collection exceeds archive size limit".into(),
                    ));
                }
                let mut contents = Vec::with_capacity(expected_size as usize);
                File::open(path)?
                    .take(MAX_FILE_BYTES + 1)
                    .read_to_end(&mut contents)?;
                let actual_size = contents.len() as u64;
                if actual_size > MAX_FILE_BYTES || actual_size != expected_size {
                    return Err(StorageError::Archive(
                        "collection file changed or exceeds size limit".into(),
                    ));
                }
                *total = total
                    .checked_add(actual_size)
                    .ok_or_else(|| StorageError::Archive("archive size overflow".into()))?;
                if *total > MAX_ARCHIVE_UNPACKED_BYTES {
                    return Err(StorageError::Archive(
                        "collection exceeds archive size limit".into(),
                    ));
                }
                zip.start_file(
                    name,
                    SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Deflated),
                )
                .map_err(|e| StorageError::Archive(e.to_string()))?;
                zip.write_all(&contents)?;
            } else {
                return Err(StorageError::Archive(
                    "collection contains a special file".into(),
                ));
            }
        }
        Ok(())
    }

    fn safe_dir_name(name: &str) -> String {
        let mut value: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        value = value.trim_matches('-').to_string();
        if value.is_empty() {
            value = "keyboard".into();
        }
        value.chars().take(48).collect()
    }
}

fn valid_color(value: &str) -> bool {
    let bytes = value.as_bytes();
    matches!(bytes.len(), 7 | 9) && bytes[0] == b'#' && bytes[1..].iter().all(u8::is_ascii_hexdigit)
}
fn valid_dimension(n: f64) -> bool {
    n.is_finite() && n > 0.0 && n <= MAX_CANVAS_SIZE
}
fn valid_point(x: f64, y: f64) -> bool {
    x.is_finite() && y.is_finite() && x >= 0.0 && y >= 0.0
}
fn segments_intersect(a: Point, b: Point, c: Point, d: Point) -> bool {
    fn cross(a: Point, b: Point, c: Point) -> f64 {
        (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
    }
    fn on_segment(a: Point, b: Point, p: Point) -> bool {
        p.x >= a.x.min(b.x) && p.x <= a.x.max(b.x) && p.y >= a.y.min(b.y) && p.y <= a.y.max(b.y)
    }
    let (o1, o2, o3, o4) = (
        cross(a, b, c),
        cross(a, b, d),
        cross(c, d, a),
        cross(c, d, b),
    );
    let eps = 1e-10;
    if ((o1 > eps && o2 < -eps) || (o1 < -eps && o2 > eps))
        && ((o3 > eps && o4 < -eps) || (o3 < -eps && o4 > eps))
    {
        return true;
    }
    (o1.abs() <= eps && on_segment(a, b, c))
        || (o2.abs() <= eps && on_segment(a, b, d))
        || (o3.abs() <= eps && on_segment(c, d, a))
        || (o4.abs() <= eps && on_segment(c, d, b))
}
pub fn valid_physical_id(id: &str) -> bool {
    let one_byte = |prefix: &str| {
        id.strip_prefix(prefix)
            .filter(|tail| tail.len() == 1)
            .map(|tail| tail.as_bytes()[0])
    };
    if one_byte("Key").is_some_and(|b| b.is_ascii_uppercase())
        || one_byte("Digit").is_some_and(|b| b.is_ascii_digit())
        || one_byte("Numpad").is_some_and(|b| b.is_ascii_digit())
    {
        return true;
    }
    if let Some(number) = id.strip_prefix('F').and_then(|s| s.parse::<u8>().ok())
        && (1..=24).contains(&number)
        && id == format!("F{number}")
    {
        return true;
    }
    matches!(
        id,
        "Backquote"
            | "Backslash"
            | "BracketLeft"
            | "BracketRight"
            | "Comma"
            | "Equal"
            | "Minus"
            | "Period"
            | "Quote"
            | "Semicolon"
            | "Slash"
            | "AltLeft"
            | "AltRight"
            | "CapsLock"
            | "ControlLeft"
            | "ControlRight"
            | "MetaLeft"
            | "MetaRight"
            | "ShiftLeft"
            | "ShiftRight"
            | "Enter"
            | "Space"
            | "Tab"
            | "Backspace"
            | "Delete"
            | "End"
            | "Home"
            | "Insert"
            | "PageDown"
            | "PageUp"
            | "ArrowDown"
            | "ArrowLeft"
            | "ArrowRight"
            | "ArrowUp"
            | "Escape"
            | "PrintScreen"
            | "ScrollLock"
            | "Pause"
            | "ContextMenu"
            | "NumLock"
            | "NumpadAdd"
            | "NumpadBackspace"
            | "NumpadClear"
            | "NumpadClearEntry"
            | "NumpadComma"
            | "NumpadDecimal"
            | "NumpadDivide"
            | "NumpadEnter"
            | "NumpadEqual"
            | "NumpadMultiply"
            | "NumpadParenLeft"
            | "NumpadParenRight"
            | "NumpadSubtract"
            | "IntlBackslash"
            | "IntlRo"
            | "IntlYen"
            | "Lang1"
            | "Lang2"
            | "Lang3"
            | "Lang4"
            | "Lang5"
            | "Convert"
            | "NonConvert"
            | "KanaMode"
            | "KanjiMode"
            | "BrowserBack"
            | "BrowserFavorites"
            | "BrowserForward"
            | "BrowserHome"
            | "BrowserRefresh"
            | "BrowserSearch"
            | "BrowserStop"
            | "Eject"
            | "LaunchApp1"
            | "LaunchApp2"
            | "LaunchMail"
            | "MediaPlayPause"
            | "MediaSelect"
            | "MediaStop"
            | "MediaTrackNext"
            | "MediaTrackPrevious"
            | "Power"
            | "Sleep"
            | "AudioVolumeDown"
            | "AudioVolumeMute"
            | "AudioVolumeUp"
            | "WakeUp"
    )
}
fn invalid(message: impl Into<String>) -> SchemaError {
    SchemaError::Invalid(message.into())
}

#[cfg(test)]
mod storage_tests {
    use super::{
        storage::{
            KeyboardCollection, MAX_ARCHIVE_BYTES, MAX_ARCHIVE_FILES, MAX_ARCHIVE_UNPACKED_BYTES,
        },
        *,
    };
    use std::{fs, io::Write};
    use zip::{ZipWriter, write::SimpleFileOptions};

    fn sample() -> KeyboardSchema {
        KeyboardSchema {
            version: 1,
            name: "My board".into(),
            show_finger_colors: true,
            style: None,
            canvas: Canvas {
                width: 100.0,
                height: 50.0,
            },
            keys: vec![],
        }
    }

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "staminator-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn zip_file(path: &std::path::Path, entry: &str, bytes: &[u8]) {
        let file = fs::File::create(path).unwrap();
        let mut zip = ZipWriter::new(file);
        zip.start_file(entry, SimpleFileOptions::default()).unwrap();
        zip.write_all(bytes).unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn pink_keyboard_archive_can_be_imported_and_exported() {
        let root = temp_dir("pink-keyboard");
        let data = include_bytes!("../../../assets/keyboards/pink/keyboard.json");
        let archive = root.join("pink.zip");
        zip_file(&archive, "keyboard.json", data);
        let collection = KeyboardCollection::new(root.join("boards")).unwrap();
        let entry = collection.import_zip(&archive).unwrap();
        let schema = collection.load(&entry.id).unwrap();
        assert_eq!(schema.name, "Soft Pink ANSI");
        assert!(!schema.show_finger_colors);
        assert_eq!(
            schema.style.as_ref().unwrap().font,
            Some(KeyboardFont::Soft)
        );
        let exported = root.join("export.zip");
        collection.export_zip(&entry.id, &exported).unwrap();
        let other = KeyboardCollection::new(root.join("other-profile")).unwrap();
        let imported = other.import_zip(&exported).unwrap();
        assert_eq!(other.load(&imported.id).unwrap(), schema);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_keyboard_style_does_not_validate() {
        let mut schema = sample();
        schema.style = Some(KeyboardStyle {
            background_color: Some("pink".into()),
            ..Default::default()
        });
        assert!(schema.validate().is_err());
        schema.style = Some(KeyboardStyle {
            key_corner_radius: Some(0.6),
            ..Default::default()
        });
        assert!(schema.validate().is_err());
        schema.style = Some(KeyboardStyle {
            key_corner_radius: Some(f32::NAN),
            ..Default::default()
        });
        assert!(schema.validate().is_err());
    }

    #[test]
    fn collection_saves_lists_loads_exports_imports_and_removes() {
        let root = temp_dir("keyboard-store");
        let collection = KeyboardCollection::new(root.join("boards")).unwrap();
        let original = sample();
        let first = collection.save_new(&original).unwrap();
        assert_eq!(collection.load(&first.id).unwrap(), original);
        let self_export = root.join("boards").join(&first.id).join("loop.zip");
        assert!(collection.export_zip(&first.id, &self_export).is_err());
        assert!(!self_export.exists());
        let zip_path = root.join("board.zip");
        collection.export_zip(&first.id, &zip_path).unwrap();
        let second = collection.import_zip(&zip_path).unwrap();
        assert_ne!(first.id, second.id);
        assert_eq!(collection.list().unwrap().len(), 2);
        assert_eq!(collection.load(&second.id).unwrap(), original);
        let third = collection.import_zip(&zip_path).unwrap();
        assert_ne!(second.id, third.id);
        let saved_again = collection.save_new(&original).unwrap();
        assert_ne!(first.id, saved_again.id);
        let assets_zip = root.join("with-assets.zip");
        let file = fs::File::create(&assets_zip).unwrap();
        let mut writer = ZipWriter::new(file);
        writer
            .start_file("keyboard.json", SimpleFileOptions::default())
            .unwrap();
        writer
            .write_all(&serde_json::to_vec(&original).unwrap())
            .unwrap();
        writer
            .add_directory("assets/", SimpleFileOptions::default())
            .unwrap();
        writer
            .start_file("assets/preview.bin", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"asset-data").unwrap();
        writer.finish().unwrap();
        let with_assets = collection.import_zip(&assets_zip).unwrap();
        assert_eq!(
            fs::read(
                root.join("boards")
                    .join(&with_assets.id)
                    .join("assets/preview.bin")
            )
            .unwrap(),
            b"asset-data"
        );
        let exported_assets = root.join("with-assets-export.zip");
        collection
            .export_zip(&with_assets.id, &exported_assets)
            .unwrap();
        let mut exported = zip::ZipArchive::new(fs::File::open(exported_assets).unwrap()).unwrap();
        let mut asset = exported.by_name("assets/preview.bin").unwrap();
        let mut contents = Vec::new();
        asset.read_to_end(&mut contents).unwrap();
        assert_eq!(contents, b"asset-data");
        collection.remove(&first.id).unwrap();
        assert_eq!(collection.list().unwrap().len(), 4);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn import_rejects_path_traversal_and_invalid_schema_without_partial_entry() {
        let root = temp_dir("keyboard-unsafe-archive");
        let collection = KeyboardCollection::new(root.join("boards")).unwrap();
        let valid = serde_json::to_vec(&sample()).unwrap();
        let traversal = root.join("traversal.zip");
        zip_file(&traversal, "../keyboard.json", &valid);
        assert!(collection.import_zip(&traversal).is_err());
        let mut bad = sample();
        bad.version = 99;
        let bad_zip = root.join("bad-schema.zip");
        zip_file(
            &bad_zip,
            "keyboard.json",
            &serde_json::to_vec(&bad).unwrap(),
        );
        assert!(collection.import_zip(&bad_zip).is_err());
        assert!(collection.list().unwrap().is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn import_rejects_missing_schema_duplicate_entries_and_archive_limits() {
        let root = temp_dir("keyboard-archive-rejections");
        let collection = KeyboardCollection::new(root.join("boards")).unwrap();
        let valid = serde_json::to_vec(&sample()).unwrap();

        let missing = root.join("missing.zip");
        zip_file(&missing, "assets/only.bin", b"asset");
        assert!(collection.import_zip(&missing).is_err());

        let duplicate = root.join("duplicate.zip");
        let mut writer = ZipWriter::new(fs::File::create(&duplicate).unwrap());
        writer
            .start_file("keyboard.json", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(&valid).unwrap();
        writer
            .start_file("Keyboard.json", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(&valid).unwrap();
        writer.finish().unwrap();
        assert!(collection.import_zip(&duplicate).is_err());

        let too_many = root.join("too-many.zip");
        let mut writer = ZipWriter::new(fs::File::create(&too_many).unwrap());
        for index in 0..=MAX_ARCHIVE_FILES {
            writer
                .start_file(format!("file-{index}"), SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"x").unwrap();
        }
        writer.finish().unwrap();
        assert!(collection.import_zip(&too_many).is_err());

        let oversized = root.join("oversized-entry.zip");
        let mut writer = ZipWriter::new(fs::File::create(&oversized).unwrap());
        writer
            .start_file("keyboard.json", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(&valid).unwrap();
        writer
            .start_file("assets/large.bin", SimpleFileOptions::default())
            .unwrap();
        writer
            .write_all(&vec![b'x'; MAX_FILE_BYTES as usize + 1])
            .unwrap();
        writer.finish().unwrap();
        assert!(collection.import_zip(&oversized).is_err());

        let total_oversized = root.join("oversized-total.zip");
        let mut writer = ZipWriter::new(fs::File::create(&total_oversized).unwrap());
        writer
            .start_file("keyboard.json", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(&valid).unwrap();
        for name in ["assets/a.bin", "assets/b.bin"] {
            writer
                .start_file(name, SimpleFileOptions::default())
                .unwrap();
            writer
                .write_all(&vec![b'x'; MAX_FILE_BYTES as usize])
                .unwrap();
        }
        writer.finish().unwrap();
        assert!(collection.import_zip(&total_oversized).is_err());
        assert!(collection.list().unwrap().is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn import_rejects_malformed_json_symlink_entries_and_conflicting_tree() {
        let root = temp_dir("keyboard-archive-shape-errors");
        let collection = KeyboardCollection::new(root.join("boards")).unwrap();

        let malformed = root.join("malformed.zip");
        zip_file(&malformed, "keyboard.json", b"{not json");
        assert!(collection.import_zip(&malformed).is_err());

        let symlink = root.join("symlink.zip");
        let mut writer = ZipWriter::new(fs::File::create(&symlink).unwrap());
        writer
            .start_file("keyboard.json", SimpleFileOptions::default())
            .unwrap();
        writer
            .write_all(&serde_json::to_vec(&sample()).unwrap())
            .unwrap();
        writer
            .add_symlink(
                "assets/link",
                "../outside-target",
                SimpleFileOptions::default(),
            )
            .unwrap();
        writer.finish().unwrap();
        assert!(
            collection
                .import_zip(&symlink)
                .unwrap_err()
                .to_string()
                .contains("symbolic links")
        );

        let conflict = root.join("conflict.zip");
        let mut writer = ZipWriter::new(fs::File::create(&conflict).unwrap());
        writer
            .start_file("keyboard.json", SimpleFileOptions::default())
            .unwrap();
        writer
            .write_all(&serde_json::to_vec(&sample()).unwrap())
            .unwrap();
        writer
            .start_file("assets", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"file").unwrap();
        writer
            .start_file("assets/child.bin", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"child").unwrap();
        writer.finish().unwrap();
        assert!(collection.import_zip(&conflict).is_err());
        assert!(collection.list().unwrap().is_empty());
        assert_eq!(fs::read_dir(root.join("boards")).unwrap().count(), 0);
        let invalid_zip = root.join("not-a-zip.zip");
        fs::write(&invalid_zip, b"not a zip archive").unwrap();
        assert!(collection.import_zip(&invalid_zip).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn import_rejects_oversized_archive_before_parsing() {
        let root = temp_dir("keyboard-compressed-limit");
        let collection = KeyboardCollection::new(root.join("boards")).unwrap();
        let archive = root.join("too-large.zip");
        fs::write(&archive, vec![0u8; MAX_ARCHIVE_BYTES as usize + 1]).unwrap();
        assert!(
            collection
                .import_zip(&archive)
                .unwrap_err()
                .to_string()
                .contains("compressed archive")
        );
        assert!(collection.list().unwrap().is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn collection_reports_io_and_invalid_identifier_errors_without_mutation() {
        let root = temp_dir("keyboard-api-errors");
        let blocker = root.join("file-not-directory");
        fs::write(&blocker, b"file").unwrap();
        assert!(KeyboardCollection::new(&blocker).is_err());

        let collection = KeyboardCollection::new(root.join("boards")).unwrap();
        assert!(collection.import_zip(root.join("missing.zip")).is_err());
        assert!(collection.load("../outside").is_err());
        assert!(collection.remove("../outside").is_err());
        assert!(
            collection
                .export_zip("../outside", root.join("bad.zip"))
                .is_err()
        );

        let mut invalid = sample();
        invalid.version = 200;
        assert!(collection.save_new(&invalid).is_err());
        assert!(collection.list().unwrap().is_empty());

        let entry = collection.save_new(&sample()).unwrap();
        let occupied_output = root.join("exists.zip");
        fs::write(&occupied_output, b"keep me").unwrap();
        assert!(collection.export_zip(&entry.id, &occupied_output).is_err());
        assert_eq!(fs::read(&occupied_output).unwrap(), b"keep me");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn export_rejects_symlink_assets_and_cleans_partial_archive() {
        let root = temp_dir("keyboard-export-symlink");
        let collection = KeyboardCollection::new(root.join("boards")).unwrap();
        let entry = collection.save_new(&sample()).unwrap();
        let entry_dir = root.join("boards").join(&entry.id);
        let outside = root.join("outside.bin");
        fs::write(&outside, b"outside").unwrap();
        #[cfg(windows)]
        let linked =
            std::os::windows::fs::symlink_file(&outside, entry_dir.join("asset.bin")).is_ok();
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(&outside, entry_dir.join("asset.bin")).is_ok();
        if linked {
            let output = root.join("partial.zip");
            assert!(collection.export_zip(&entry.id, &output).is_err());
            assert!(!output.exists());
        }
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn export_rejects_special_files_and_cleans_partial_archive() {
        use std::os::unix::net::UnixListener;

        let root = temp_dir("keyboard-special-file");
        let collection = KeyboardCollection::new(root.join("boards")).unwrap();
        let entry = collection.save_new(&sample()).unwrap();
        let entry_dir = root.join("boards").join(&entry.id);
        let socket = UnixListener::bind(entry_dir.join("socket-entry")).unwrap();
        let output = root.join("special.zip");
        assert!(collection.export_zip(&entry.id, &output).is_err());
        assert!(!output.exists());
        drop(socket);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn export_removes_archive_that_exceeds_compressed_size_limit() {
        let root = temp_dir("keyboard-compressed-export-limit");
        let collection = KeyboardCollection::new(root.join("boards")).unwrap();
        let entry = collection.save_new(&sample()).unwrap();
        let entry_dir = root.join("boards").join(&entry.id);
        let schema_size = fs::metadata(entry_dir.join("keyboard.json")).unwrap().len();
        let remaining = (MAX_ARCHIVE_UNPACKED_BYTES - schema_size) as usize;
        let first_len = MAX_FILE_BYTES as usize;
        let second_len = remaining - first_len;
        let mut state = 0x9e37_79b9_u32;
        let mut noise = || {
            (0..first_len.max(second_len))
                .map(|_| {
                    state ^= state << 13;
                    state ^= state >> 17;
                    state ^= state << 5;
                    state as u8
                })
                .collect::<Vec<_>>()
        };
        let bytes = noise();
        fs::write(entry_dir.join("asset-a.bin"), &bytes[..first_len]).unwrap();
        fs::write(entry_dir.join("asset-b.bin"), &bytes[..second_len]).unwrap();

        let output = root.join("too-large-export.zip");
        assert!(
            collection
                .export_zip(&entry.id, &output)
                .unwrap_err()
                .to_string()
                .contains("compressed size limit")
        );
        assert!(!output.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn load_list_and_export_reject_symlink_schema() {
        let root = temp_dir("keyboard-symlink");
        let collection = KeyboardCollection::new(root.join("boards")).unwrap();
        let entry = collection.save_new(&sample()).unwrap();
        let entry_dir = root.join("boards").join(&entry.id);
        fs::remove_file(entry_dir.join("keyboard.json")).unwrap();
        let target = root.join("outside.json");
        fs::write(&target, serde_json::to_vec(&sample()).unwrap()).unwrap();
        #[cfg(windows)]
        let linked =
            std::os::windows::fs::symlink_file(&target, entry_dir.join("keyboard.json")).is_ok();
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(&target, entry_dir.join("keyboard.json")).is_ok();
        if linked {
            assert!(collection.load(&entry.id).is_err());
            assert!(collection.list().unwrap().is_empty());
            assert!(
                collection
                    .export_zip(&entry.id, root.join("bad.zip"))
                    .is_err()
            );
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn list_omits_entries_with_ids_the_api_cannot_select() {
        let root = temp_dir("keyboard-invalid-entry-id");
        let collection = KeyboardCollection::new(root.join("boards")).unwrap();
        let bad_dir = root.join("boards").join("manual board");
        fs::create_dir(&bad_dir).unwrap();
        sample().save_new(bad_dir.join("keyboard.json")).unwrap();
        assert!(collection.list().unwrap().is_empty());
        assert!(collection.load("manual board").is_err());
        let _ = fs::remove_dir_all(root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_codes_use_canonical_key_names() {
        for code in ["KeyA", "Digit1", "Numpad9", "F24"] {
            assert!(valid_physical_id(code), "{code}");
        }
        for code in ["Key1", "Keya", "DigitA", "NumpadA", "F01", "F25"] {
            assert!(!valid_physical_id(code), "{code}");
        }
    }

    fn sample() -> KeyboardSchema {
        KeyboardSchema {
            version: 1,
            show_finger_colors: true,
            style: None,
            name: "Test board".into(),
            canvas: Canvas {
                width: 300.0,
                height: 120.0,
            },
            keys: vec![
                Key {
                    physical_id: "KeyA".into(),
                    geometry: Geometry::Rect {
                        x: 10.0,
                        y: 20.0,
                        width: 40.0,
                        height: 40.0,
                    },
                    label: Some("a".into()),
                    shifted_label: Some("A".into()),
                    color: Some("#abcdef".into()),
                    finger: Some(Finger::LeftLittle),
                },
                Key {
                    physical_id: "Enter".into(),
                    geometry: Geometry::Polygon {
                        points: vec![
                            Point { x: 100.0, y: 20.0 },
                            Point { x: 160.0, y: 20.0 },
                            Point { x: 160.0, y: 60.0 },
                            Point { x: 130.0, y: 60.0 },
                        ],
                    },
                    label: None,
                    shifted_label: None,
                    color: None,
                    finger: None,
                },
            ],
        }
    }

    #[test]
    fn round_trip_rect_and_polygon_with_optional_properties() {
        let value = sample();
        let json = serde_json::to_vec(&value).unwrap();
        let decoded: KeyboardSchema = serde_json::from_slice(&json).unwrap();
        assert_eq!(decoded, value);
        decoded.validate().unwrap();
    }

    #[test]
    fn rejects_bad_version_duplicate_ids_and_out_of_bounds_geometry() {
        let mut value = sample();
        value.version = 2;
        assert!(
            value
                .validate()
                .unwrap_err()
                .to_string()
                .contains("unsupported version")
        );
        let mut value = sample();
        value.keys[1].physical_id = "KeyA".into();
        assert!(
            value
                .validate()
                .unwrap_err()
                .to_string()
                .contains("duplicate")
        );
        let mut value = sample();
        value.keys[0].geometry = Geometry::Rect {
            x: -1.0,
            y: 0.0,
            width: 2.0,
            height: 2.0,
        };
        assert!(value.validate().is_err());
        let mut value = sample();
        value.keys[1].geometry = Geometry::Polygon {
            points: vec![Point { x: 1.0, y: 1.0 }; 3],
        };
        assert!(
            value
                .validate()
                .unwrap_err()
                .to_string()
                .contains("duplicate vertices")
        );
        let mut value = sample();
        value.keys[0].physical_id = "A".into();
        assert!(
            value
                .validate()
                .unwrap_err()
                .to_string()
                .contains("KeyboardEvent.code")
        );
    }

    #[test]
    fn rejects_self_intersecting_and_duplicate_polygon_vertices() {
        let mut value = sample();
        value.keys[1].geometry = Geometry::Polygon {
            points: vec![
                Point { x: 10.0, y: 10.0 },
                Point { x: 20.0, y: 20.0 },
                Point { x: 10.0, y: 20.0 },
                Point { x: 20.0, y: 10.0 },
                Point { x: 15.0, y: 11.0 },
            ],
        };
        assert!(
            value
                .validate()
                .unwrap_err()
                .to_string()
                .contains("self-intersecting")
        );
        value.keys[1].geometry = Geometry::Polygon {
            points: vec![
                Point { x: 10.0, y: 10.0 },
                Point { x: 20.0, y: 10.0 },
                Point { x: 20.0, y: 10.0 },
            ],
        };
        assert!(
            value
                .validate()
                .unwrap_err()
                .to_string()
                .contains("duplicate vertices")
        );
    }

    #[test]
    fn validates_schema_size_text_color_and_geometry_boundaries() {
        let invalids = [
            {
                let mut s = sample();
                s.name = "  ".into();
                s
            },
            {
                let mut s = sample();
                s.name = "n".repeat(129);
                s
            },
            {
                let mut s = sample();
                s.canvas.width = 0.0;
                s
            },
            {
                let mut s = sample();
                s.canvas.height = f64::INFINITY;
                s
            },
            {
                let mut s = sample();
                s.canvas.width = MAX_CANVAS_SIZE + 1.0;
                s
            },
            {
                let mut s = sample();
                s.keys[0].label = Some("x".repeat(129));
                s
            },
            {
                let mut s = sample();
                s.keys[0].shifted_label = Some("x".repeat(129));
                s
            },
            {
                let mut s = sample();
                s.keys[0].color = Some("#xyzxyz".into());
                s
            },
            {
                let mut s = sample();
                s.keys[0].color = Some("#12345".into());
                s
            },
            {
                let mut s = sample();
                s.keys[0].geometry = Geometry::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 0.0,
                    height: 1.0,
                };
                s
            },
            {
                let mut s = sample();
                s.keys[0].geometry = Geometry::Rect {
                    x: 299.0,
                    y: 0.0,
                    width: 2.0,
                    height: 1.0,
                };
                s
            },
            {
                let mut s = sample();
                s.keys[0].geometry = Geometry::Rect {
                    x: f64::NAN,
                    y: 0.0,
                    width: 1.0,
                    height: 1.0,
                };
                s
            },
            {
                let mut s = sample();
                s.keys[1].geometry = Geometry::Polygon {
                    points: vec![Point { x: 1.0, y: 1.0 }, Point { x: 2.0, y: 2.0 }],
                };
                s
            },
            {
                let mut s = sample();
                s.keys[1].geometry = Geometry::Polygon {
                    points: (0..65)
                        .map(|i| Point {
                            x: i as f64,
                            y: (i % 2) as f64,
                        })
                        .collect(),
                };
                s
            },
            {
                let mut s = sample();
                s.keys[1].geometry = Geometry::Polygon {
                    points: vec![
                        Point { x: 1.0, y: 1.0 },
                        Point { x: 2.0, y: 2.0 },
                        Point { x: 3.0, y: 3.0 },
                    ],
                };
                s
            },
            {
                let mut s = sample();
                s.keys[1].geometry = Geometry::Polygon {
                    points: vec![
                        Point { x: 1.0, y: 1.0 },
                        Point { x: 2.0, y: 2.0 },
                        Point { x: 3.0, y: 3.0 },
                    ],
                };
                s
            },
            {
                let mut s = sample();
                s.keys[1].geometry = Geometry::Polygon {
                    points: vec![
                        Point { x: 299.0, y: 1.0 },
                        Point { x: 301.0, y: 1.0 },
                        Point { x: 300.0, y: 2.0 },
                    ],
                };
                s
            },
        ];
        for invalid in invalids {
            assert!(invalid.validate().is_err(), "{invalid:?}");
        }

        let mut too_many = sample();
        too_many.keys = vec![sample().keys[0].clone(); MAX_KEYS + 1];
        assert!(
            too_many
                .validate()
                .unwrap_err()
                .to_string()
                .contains("more than")
        );
    }

    #[test]
    fn load_is_bounded_and_save_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("staminator-keyboard-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("board.json");
        let _ = fs::remove_file(&path);
        let value = sample();
        value.save_new(&path).unwrap();
        assert_eq!(KeyboardSchema::load(&path).unwrap(), value);
        assert!(value.save_new(&path).is_err());
        fs::write(&path, vec![b' '; MAX_FILE_BYTES as usize + 1]).unwrap();
        assert!(matches!(
            KeyboardSchema::load(&path),
            Err(SchemaError::TooLarge)
        ));
        let _ = fs::remove_file(path);
        let _ = fs::remove_dir(dir);
    }
}

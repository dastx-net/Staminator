//! Installs bundled keyboards into a user's collection once.

use staminator_keyboard_schema::{KeyboardSchema, storage::KeyboardCollection};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

const PINK_JSON: &[u8] = include_bytes!("../../../assets/keyboards/pink/keyboard.json");
const MARKER: &str = ".bundled-pink-installed";
const SPLIT_JSON: &[u8] = include_bytes!("../../../assets/keyboards/split-black/keyboard.json");
const SPLIT_MARKER: &str = ".bundled-split-black-installed";

/// Installs each bundled keyboard once, without restoring user-deleted copies.
pub fn ensure(root: &Path, collection: &KeyboardCollection) -> Result<(), String> {
    let mut errors = Vec::new();
    for (marker, data) in [(MARKER, PINK_JSON), (SPLIT_MARKER, SPLIT_JSON)] {
        if let Err(error) = ensure_one(root, collection, marker, data) {
            errors.push(error);
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

fn ensure_one(
    root: &Path,
    collection: &KeyboardCollection,
    marker_name: &str,
    data: &[u8],
) -> Result<(), String> {
    let marker = root.join(marker_name);
    match fs::symlink_metadata(&marker) {
        Ok(metadata) => {
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err("bundled keyboard marker is not a regular file".into());
            }
            return validate_marker(&marker);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("cannot inspect bundled keyboard marker: {error}")),
    }

    let bundled: KeyboardSchema = serde_json::from_slice(data)
        .map_err(|error| format!("invalid bundled keyboard JSON: {error}"))?;
    bundled
        .validate()
        .map_err(|error| format!("invalid bundled keyboard schema: {error}"))?;

    let entries = collection
        .list()
        .map_err(|error| format!("cannot inspect keyboard collection: {error}"))?;
    let mut identical_exists = false;
    for entry in entries {
        let existing = collection
            .load(&entry.id)
            .map_err(|error| format!("cannot inspect keyboard '{}': {error}", entry.id))?;
        if existing == bundled {
            identical_exists = true;
            break;
        }
    }
    if !identical_exists {
        collection
            .save_new(&bundled)
            .map_err(|error| format!("cannot install bundled keyboard: {error}"))?;
    }

    // Create only after the install (or identical-entry check) succeeds. If
    // marker creation fails, a retry safely finds the identical entry.
    let mut file = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&marker)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return validate_marker(&marker);
        }
        Err(error) => return Err(format!("cannot write bundled keyboard marker: {error}")),
    };
    if let Err(error) = file.write_all(b"1\n").and_then(|_| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(&marker);
        return Err(format!("cannot write bundled keyboard marker: {error}"));
    }
    Ok(())
}

fn validate_marker(marker: &Path) -> Result<(), String> {
    let contents = fs::read(marker)
        .map_err(|error| format!("cannot read bundled keyboard marker: {error}"))?;
    if contents == b"1\n" {
        Ok(())
    } else {
        Err("bundled keyboard marker has invalid contents".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
    };

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "staminator-bundled-keyboards-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn setup() -> (TempDir, KeyboardCollection) {
        let temp = TempDir::new();
        let collection = KeyboardCollection::new(temp.0.join("keyboards")).unwrap();
        (temp, collection)
    }

    #[test]
    fn invalid_pink_marker_does_not_prevent_split_install() {
        let (temp, collection) = setup();
        let root = temp.0.join("keyboards");
        fs::write(root.join(MARKER), b"invalid").unwrap();
        assert!(ensure(&root, &collection).is_err());
        assert_eq!(fs::read(root.join(MARKER)).unwrap(), b"invalid");
        let entries = collection.list().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "Split Black ANSI");
    }

    #[test]
    fn split_keyboard_is_added_to_existing_install_without_restoring_deleted_pink() {
        let (temp, collection) = setup();
        let root = temp.0.join("keyboards");
        ensure_one(&root, &collection, MARKER, PINK_JSON).unwrap();
        let pink = collection
            .list()
            .unwrap()
            .into_iter()
            .find(|entry| entry.name == "Soft Pink ANSI")
            .unwrap();
        collection.remove(&pink.id).unwrap();
        ensure(&root, &collection).unwrap();
        let entries = collection.list().unwrap();
        assert_eq!(entries.len(), 1);
        let split = entries
            .iter()
            .find(|entry| entry.name == "Split Black ANSI")
            .unwrap();
        let schema = collection.load(&split.id).unwrap();
        assert!(!schema.show_finger_colors);
        assert!(schema.keys.iter().any(|key| key.physical_id == "Space"));
        ensure(&root, &collection).unwrap();
        assert_eq!(collection.list().unwrap().len(), 1);
        collection.remove(&split.id).unwrap();
        ensure(&root, &collection).unwrap();
        assert!(collection.list().unwrap().is_empty());
    }

    #[test]
    fn installs_on_first_run_even_when_collection_is_populated() {
        let (temp, collection) = setup();
        let other: KeyboardSchema = serde_json::from_slice(PINK_JSON).unwrap();
        let mut other = other;
        other.name = "Different keyboard".into();
        collection.save_new(&other).unwrap();

        ensure_one(&temp.0.join("keyboards"), &collection, MARKER, PINK_JSON).unwrap();
        let entries = collection.list().unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().any(|entry| entry.name == "Soft Pink ANSI"));
    }

    #[test]
    fn recognizes_identical_manual_install_without_duplicating() {
        let (temp, collection) = setup();
        let bundled: KeyboardSchema = serde_json::from_slice(PINK_JSON).unwrap();
        collection.save_new(&bundled).unwrap();
        ensure_one(&temp.0.join("keyboards"), &collection, MARKER, PINK_JSON).unwrap();
        assert_eq!(collection.list().unwrap().len(), 1);
    }

    #[test]
    fn never_overwrites_existing_keyboard_with_same_name() {
        let (temp, collection) = setup();
        let mut custom: KeyboardSchema = serde_json::from_slice(PINK_JSON).unwrap();
        custom.keys.pop();
        collection.save_new(&custom).unwrap();
        ensure_one(&temp.0.join("keyboards"), &collection, MARKER, PINK_JSON).unwrap();
        assert_eq!(collection.list().unwrap().len(), 2);
        assert_eq!(collection.load("soft-pink-ansi").unwrap(), custom);
    }

    #[test]
    fn deletion_after_install_is_preserved_on_restart() {
        let (temp, collection) = setup();
        let root = temp.0.join("keyboards");
        ensure_one(&root, &collection, MARKER, PINK_JSON).unwrap();
        let pink = collection
            .list()
            .unwrap()
            .into_iter()
            .find(|entry| entry.name == "Soft Pink ANSI")
            .unwrap();
        fs::remove_dir_all(root.join(pink.id)).unwrap();

        ensure_one(&root, &collection, MARKER, PINK_JSON).unwrap();
        assert!(collection.list().unwrap().is_empty());
    }

    #[test]
    fn rejects_incomplete_marker_without_replacing_it() {
        let (temp, collection) = setup();
        let root = temp.0.join("keyboards");
        let marker = root.join(MARKER);
        fs::write(&marker, b"").unwrap();
        assert!(ensure_one(&root, &collection, MARKER, PINK_JSON).is_err());
        assert_eq!(fs::read(marker).unwrap(), b"");
        assert!(collection.list().unwrap().is_empty());
    }
}

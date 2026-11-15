//! Locations for user-provided music and sound effects.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

/// Audio folders under the same application directory as other user resources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioResources {
    root: PathBuf,
}

impl AudioResources {
    pub fn new(app_directory: impl Into<PathBuf>) -> Self {
        Self {
            root: app_directory.into(),
        }
    }

    pub fn music(&self) -> PathBuf {
        self.root.join("Music")
    }

    pub fn sounds(&self) -> PathBuf {
        self.root.join("Sound")
    }

    /// Create missing directories without modifying existing resources.
    pub fn ensure_directories(&self) -> io::Result<()> {
        for path in [self.music(), self.sounds()] {
            fs::create_dir_all(&path).map_err(|error| {
                io::Error::new(error.kind(), format!("{}: {error}", path.display()))
            })?;
        }
        Ok(())
    }

    /// Find supported files directly in a resource folder, in stable order.
    pub fn files(directory: &Path) -> io::Result<Vec<PathBuf>> {
        let mut files = Vec::new();
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file()
                && path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| {
                        ["wav", "mp3"]
                            .iter()
                            .any(|allowed| extension.eq_ignore_ascii_case(allowed))
                    })
            {
                files.push(path);
            }
        }
        files.sort();
        Ok(files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "staminator-audio-folders-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn creates_folders_and_finds_user_files_without_overwriting_them() {
        let root = root();
        let resources = AudioResources::new(&root);
        resources.ensure_directories().unwrap();
        fs::write(resources.music().join("track.MP3"), b"user content").unwrap();
        fs::write(resources.music().join("notes.txt"), b"ignored").unwrap();
        fs::create_dir(resources.music().join("directory.wav")).unwrap();
        fs::write(resources.sounds().join("effect.wav"), b"effect").unwrap();
        resources.ensure_directories().unwrap();
        assert_eq!(
            AudioResources::files(&resources.music()).unwrap(),
            vec![resources.music().join("track.MP3")]
        );
        assert_eq!(
            AudioResources::files(&resources.sounds()).unwrap(),
            vec![resources.sounds().join("effect.wav")]
        );
        assert_eq!(
            fs::read(resources.music().join("track.MP3")).unwrap(),
            b"user content"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reports_inaccessible_or_missing_folders() {
        let root = root();
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("Music"), b"file").unwrap();
        let resources = AudioResources::new(&root);
        assert!(
            resources
                .ensure_directories()
                .unwrap_err()
                .to_string()
                .contains("Music")
        );
        assert!(AudioResources::files(&resources.sounds()).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}

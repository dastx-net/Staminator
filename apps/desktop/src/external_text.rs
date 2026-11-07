//! Loading and indexing user supplied text files.

use std::path::{Path, PathBuf};

/// Text decoded from an external file. `byte_positions[n]` is the source byte
/// range that produced the nth Unicode scalar in `text` (after newline folding).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalText {
    pub file_id: FileId,
    pub text: String,
    pub byte_positions: Vec<std::ops::Range<usize>>,
}

/// Stable, path-based identity for an opened external text file.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FileId {
    path: PathBuf,
    pub size: u64,
    pub content_hash: u64,
}

impl FileId {
    pub fn new(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        Self {
            path: path.canonicalize().unwrap_or_else(|_| path.to_path_buf()),
            size: 0,
            content_hash: 0,
        }
    }

    fn with_content(path: impl AsRef<Path>, bytes: &[u8]) -> Self {
        let mut id = Self::new(path);
        id.size = bytes.len() as u64;
        id.content_hash = bytes.iter().fold(0xcbf29ce484222325u64, |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
        });
        id
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl ExternalText {
    pub fn load(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path)?;
        Self::decode(FileId::with_content(path, &bytes), &bytes)
    }

    /// Reject non-UTF-8 input instead of silently replacing characters.
    pub fn decode(file_id: FileId, bytes: &[u8]) -> std::io::Result<Self> {
        let mut file_id = file_id;
        file_id.size = bytes.len() as u64;
        file_id.content_hash = bytes.iter().fold(0xcbf29ce484222325u64, |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
        });
        let decoded = std::str::from_utf8(bytes).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("файл должен быть в кодировке UTF-8: {error}"),
            )
        })?;
        if decoded.contains('\0') {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "файл должен быть текстом UTF-8 без нулевых байтов",
            ));
        }
        let (decoded, offset) = decoded
            .strip_prefix('\u{feff}')
            .map_or((decoded, 0), |text| (text, 3));
        let spans = utf8_spans(decoded)
            .into_iter()
            .map(|range| range.start + offset..range.end + offset)
            .collect();
        let (text, byte_positions) = normalize_whitespace(decoded, spans);
        Ok(Self {
            file_id,
            text,
            byte_positions,
        })
    }
}

fn utf8_spans(text: &str) -> Vec<std::ops::Range<usize>> {
    text.char_indices()
        .map(|(start, ch)| start..start + ch.len_utf8())
        .collect()
}

fn normalize_whitespace(
    text: &str,
    spans: Vec<std::ops::Range<usize>>,
) -> (String, Vec<std::ops::Range<usize>>) {
    let chars: Vec<char> = text.chars().collect();
    let mut normalized = String::with_capacity(text.len());
    let mut positions = Vec::with_capacity(chars.len());
    let mut i = 0;
    let mut pending_space: Option<std::ops::Range<usize>> = None;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            if let Some(range) = pending_space.as_mut() {
                range.end = spans[i].end;
            } else if !normalized.is_empty() {
                pending_space = Some(spans[i].clone());
            }
        } else {
            if let Some(range) = pending_space.take() {
                normalized.push(' ');
                positions.push(range);
            }
            normalized.push(chars[i]);
            positions.push(spans[i].clone());
        }
        i += 1;
    }
    (normalized, positions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_collapses_whitespace_and_tracks_original_byte_ranges() {
        let value =
            ExternalText::decode(FileId::new("lesson.txt"), "я\r\nA\rB".as_bytes()).unwrap();
        assert_eq!(value.text, "я A B");
        assert_eq!(value.byte_positions, vec![0..2, 2..4, 4..5, 5..6, 6..7]);
    }

    #[test]
    fn invalid_utf8_is_rejected() {
        let error = ExternalText::decode(
            FileId::new("lesson.txt"),
            &[0xCF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2],
        )
        .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("UTF-8"));
        assert!(ExternalText::decode(FileId::new("lesson.txt"), b"a\0b\0").is_err());
    }

    #[test]
    fn utf8_bom_is_not_training_text_and_offsets_include_it() {
        let value =
            ExternalText::decode(FileId::new("lesson.txt"), "\u{feff}я a".as_bytes()).unwrap();
        assert_eq!(value.text, "я a");
        assert_eq!(value.byte_positions, vec![3..5, 5..6, 6..7]);
    }
}

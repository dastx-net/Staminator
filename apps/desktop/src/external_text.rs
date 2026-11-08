//! Loading and indexing user supplied text files.

use std::io::{BufReader, Read, Seek};
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

/// Validated external file. Opening scans the complete file once but retains
/// only its identity and normalized character count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalFile {
    file_id: FileId,
    char_count: usize,
}

impl ExternalFile {
    pub fn open(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref();
        let mut stream = ExternalTextStream::new(std::fs::File::open(path)?, FileId::new(path));
        let mut char_count = 0;
        while let Some(fragment) = stream.next_fragment(4096)? {
            char_count += fragment.text.chars().count();
        }
        Ok(Self {
            file_id: stream.file_id().clone(),
            char_count,
        })
    }

    pub fn file_id(&self) -> &FileId {
        &self.file_id
    }
    pub fn char_count(&self) -> usize {
        self.char_count
    }

    /// Starts a sequential reader at a normalized character index.
    pub fn stream_at_index(
        &self,
        index: usize,
    ) -> std::io::Result<ExternalTextStream<std::fs::File>> {
        if index > self.char_count {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "индекс за концом файла",
            ));
        }
        let offset = if index == 0 {
            0
        } else if index == self.char_count {
            self.file_id.size as usize
        } else {
            let mut stream = self.fresh_stream()?;
            let mut seen = 0;
            loop {
                let part = stream.next_fragment(4096)?.expect("index is before EOF");
                let count = part.byte_positions.len();
                if seen + count >= index {
                    let local = index - seen;
                    if local == count {
                        break part
                            .byte_positions
                            .last()
                            .map_or(self.file_id.size as usize, |span| span.end);
                    }
                    break part.byte_positions[local].start;
                } else {
                    seen += count;
                    continue;
                }
            }
        };
        self.stream_at_byte(offset)
    }

    /// Starts a sequential reader at a source byte offset that begins a
    /// normalized character (or at EOF). The offset is checked against the
    /// validated character map.
    pub fn stream_at_byte(
        &self,
        offset: usize,
    ) -> std::io::Result<ExternalTextStream<std::fs::File>> {
        if offset == self.file_id.size as usize {
            return self.fresh_stream_at(offset);
        }
        let mut scan = self.fresh_stream()?;
        let mut valid = offset == 0;
        while let Some(part) = scan.next_fragment(4096)? {
            if part.byte_positions.iter().any(|span| span.start == offset) {
                valid = true;
                break;
            }
        }
        if !valid {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "смещение не соответствует нормализованному символу",
            ));
        }
        self.fresh_stream_at(offset)
    }

    pub fn index_at_byte(&self, offset: usize) -> std::io::Result<usize> {
        if offset == self.file_id.size as usize {
            return Ok(self.char_count);
        }
        if offset == 0 {
            return Ok(0);
        }
        let mut scan = self.fresh_stream()?;
        let mut index = 0;
        while let Some(part) = scan.next_fragment(4096)? {
            for span in &part.byte_positions {
                if span.start == offset {
                    return Ok(index);
                }
                index += 1;
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "смещение не соответствует нормализованному символу",
        ))
    }

    #[cfg(test)]
    pub fn fragment_at_index(
        &self,
        index: usize,
        max_chars: usize,
    ) -> std::io::Result<Option<ExternalText>> {
        self.stream_at_index(index)?.next_fragment(max_chars)
    }
    fn fresh_stream(&self) -> std::io::Result<ExternalTextStream<std::fs::File>> {
        self.fresh_stream_at(0)
    }
    fn fresh_stream_at(&self, offset: usize) -> std::io::Result<ExternalTextStream<std::fs::File>> {
        let file = std::fs::File::open(self.file_id.path())?;
        let mut stream = ExternalTextStream::new(file, self.file_id.clone());
        stream
            .reader
            .get_mut()
            .seek(std::io::SeekFrom::Start(offset as u64))?;
        stream.offset = offset;
        stream.started = offset != 0;
        stream.seen_non_whitespace = offset != 0;
        stream.hash_enabled = false;
        Ok(stream)
    }
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

    #[cfg(test)]
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
    /// Reject non-UTF-8 input instead of silently replacing characters.
    #[cfg(test)]
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

/// Incremental UTF-8 text normalizer. Each returned fragment owns only that
/// fragment's text and source-byte mapping; the reader does not retain prior
/// fragments. Whitespace runs become one space mapped to the complete run.
pub struct ExternalTextStream<R: Read> {
    reader: BufReader<R>,
    file_id: FileId,
    offset: usize,
    pending_space: Option<std::ops::Range<usize>>,
    started: bool,
    eof: bool,
    pending_scalar: Option<(char, std::ops::Range<usize>)>,
    seen_non_whitespace: bool,
    hash_enabled: bool,
}

impl<R: Read> ExternalTextStream<R> {
    pub fn new(reader: R, file_id: FileId) -> Self {
        let mut file_id = file_id;
        file_id.size = 0;
        file_id.content_hash = 0xcbf29ce484222325;
        Self {
            reader: BufReader::new(reader),
            file_id,
            offset: 0,
            pending_space: None,
            started: false,
            eof: false,
            pending_scalar: None,
            seen_non_whitespace: false,
            hash_enabled: true,
        }
    }

    pub fn next_fragment(&mut self, max_chars: usize) -> std::io::Result<Option<ExternalText>> {
        if max_chars == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "размер фрагмента должен быть больше нуля",
            ));
        }
        let mut text = String::new();
        let mut byte_positions = Vec::new();
        while byte_positions.len() < max_chars {
            let Some((ch, span)) = self.read_scalar()? else {
                self.eof = true;
                self.pending_space = None;
                break;
            };
            if !self.started {
                self.started = true;
                if ch == '\u{feff}' {
                    continue;
                }
            }
            if ch == '\0' {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "файл должен быть текстом UTF-8 без нулевых байтов",
                ));
            }
            if ch.is_whitespace() {
                if let Some(range) = self.pending_space.as_mut() {
                    range.end = span.end;
                } else if self.seen_non_whitespace {
                    self.pending_space = Some(span);
                }
                continue;
            }
            if let Some(range) = self.pending_space.take() {
                text.push(' ');
                byte_positions.push(range);
                if byte_positions.len() == max_chars {
                    // Preserve the scalar read ahead after the separator.
                    self.pending_scalar = Some((ch, span));
                    break;
                }
            }
            text.push(ch);
            byte_positions.push(span);
            self.seen_non_whitespace = true;
        }
        if text.is_empty() && self.eof {
            return Ok(None);
        }
        Ok(Some(ExternalText {
            file_id: self.file_id.clone(),
            text,
            byte_positions,
        }))
    }

    fn read_scalar(&mut self) -> std::io::Result<Option<(char, std::ops::Range<usize>)>> {
        if let Some(scalar) = self.pending_scalar.take() {
            return Ok(Some(scalar));
        }
        let mut first = [0u8; 1];
        if self.reader.read(&mut first)? == 0 {
            return Ok(None);
        }
        let width = match first[0] {
            0x00..=0x7f => 1,
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => return Err(invalid_utf8(self.offset)),
        };
        let start = self.offset;
        let mut bytes = [0u8; 4];
        bytes[0] = first[0];
        for slot in bytes.iter_mut().take(width).skip(1) {
            self.reader
                .read_exact(std::slice::from_mut(slot))
                .map_err(|_| invalid_utf8(start))?;
        }
        let ch = std::str::from_utf8(&bytes[..width])
            .ok()
            .and_then(|s| s.chars().next())
            .ok_or_else(|| invalid_utf8(start))?;
        if self.hash_enabled {
            for byte in &bytes[..width] {
                self.file_id.content_hash =
                    (self.file_id.content_hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3);
            }
        }
        self.offset += width;
        self.file_id.size = self.offset as u64;
        Ok(Some((ch, start..self.offset)))
    }

    pub fn file_id(&self) -> &FileId {
        &self.file_id
    }
}

fn invalid_utf8(offset: usize) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("файл должен быть в кодировке UTF-8 (байт {offset})"),
    )
}

#[cfg(test)]
fn utf8_spans(text: &str) -> Vec<std::ops::Range<usize>> {
    text.char_indices()
        .map(|(start, ch)| start..start + ch.len_utf8())
        .collect()
}

#[cfg(test)]
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
    fn eof_position_can_be_restored_but_out_of_range_is_rejected() {
        let path = std::env::temp_dir().join(format!("staminator-eof-{}.txt", std::process::id()));
        std::fs::write(&path, "я b").unwrap();
        let file = ExternalFile::open(&path).unwrap();
        let mut by_index = file.stream_at_index(file.char_count()).unwrap();
        assert!(by_index.next_fragment(4).unwrap().is_none());
        let mut by_byte = file.stream_at_byte(file.file_id().size as usize).unwrap();
        assert!(by_byte.next_fragment(4).unwrap().is_none());
        assert_eq!(
            file.index_at_byte(file.file_id().size as usize).unwrap(),
            file.char_count()
        );
        assert_eq!(
            file.stream_at_index(file.char_count() + 1)
                .err()
                .unwrap()
                .kind(),
            std::io::ErrorKind::InvalidInput
        );
        assert_eq!(
            file.stream_at_byte(file.file_id().size as usize + 1)
                .err()
                .unwrap()
                .kind(),
            std::io::ErrorKind::InvalidInput
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn external_file_indexes_and_streams_without_loading_the_document() {
        let path =
            std::env::temp_dir().join(format!("staminator-stream-{}.txt", std::process::id()));
        std::fs::write(&path, "a  б\nв".as_bytes()).unwrap();
        let file = ExternalFile::open(&path).unwrap();
        assert_eq!(file.char_count(), 5);
        assert_eq!(file.index_at_byte(3).unwrap(), 2);
        let part = file.fragment_at_index(2, 2).unwrap().unwrap();
        assert_eq!(part.text, "б ");
        assert_eq!(file.index_at_byte(file.file_id().size as usize).unwrap(), 5);
        let mut stream = file.stream_at_index(2).unwrap();
        assert_eq!(stream.next_fragment(8).unwrap().unwrap().text, "б в");
        let mut space = file.stream_at_index(1).unwrap();
        let separator = space.next_fragment(1).unwrap().unwrap();
        assert_eq!(separator.text, " ");
        assert_eq!(separator.byte_positions, vec![1..3]);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn stream_at_index_handles_fragment_boundary_and_multibyte_scalars() {
        let path =
            std::env::temp_dir().join(format!("staminator-boundary-{}.txt", std::process::id()));
        let mut content = "x".repeat(4096);
        content.push('я');
        content.push('z');
        std::fs::write(&path, content.as_bytes()).unwrap();
        let file = ExternalFile::open(&path).unwrap();
        let mut at_boundary = file.stream_at_index(4096).unwrap();
        let part = at_boundary.next_fragment(2).unwrap().unwrap();
        assert_eq!(part.text, "яz");
        assert_eq!(part.byte_positions, vec![4096..4098, 4098..4099]);
        let mut after_boundary = file.stream_at_index(4097).unwrap();
        let tail = after_boundary.next_fragment(2).unwrap().unwrap();
        assert_eq!(tail.text, "z");
        assert_eq!(tail.byte_positions, vec![4098..4099]);
        assert_eq!(
            file.index_at_byte(4097).unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn external_file_open_checks_complete_file() {
        let path =
            std::env::temp_dir().join(format!("staminator-invalid-{}.txt", std::process::id()));
        std::fs::write(&path, [b'a', 0xff]).unwrap();
        assert_eq!(
            ExternalFile::open(&path).unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
        let _ = std::fs::remove_file(path);
    }

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

    #[test]
    fn stream_keeps_whitespace_mapping_across_fragment_boundaries() {
        let bytes = "\u{feff}я \r\n  b".as_bytes();
        let mut stream = ExternalTextStream::new(bytes, FileId::new("lesson.txt"));
        let first = stream.next_fragment(2).unwrap().unwrap();
        let second = stream.next_fragment(2).unwrap().unwrap();
        assert_eq!(first.text, "я ");
        assert_eq!(first.byte_positions, vec![3..5, 5..10]);
        assert_eq!(second.text, "b");
        assert_eq!(second.byte_positions, vec![10..11]);
        assert!(stream.next_fragment(2).unwrap().is_none());
        assert_eq!(stream.file_id().size, bytes.len() as u64);
        assert_eq!(
            stream.file_id().content_hash,
            FileId::with_content("lesson.txt", bytes).content_hash
        );
    }

    #[test]
    fn stream_reports_invalid_and_truncated_utf8() {
        for bytes in [&[0xff][..], &[0xe2, 0x82][..]] {
            let mut stream = ExternalTextStream::new(bytes, FileId::new("lesson.txt"));
            let error = stream.next_fragment(4).unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        }
    }

    #[test]
    fn stream_preserves_spaces_when_each_fragment_is_one_character() {
        let mut stream = ExternalTextStream::new(&b"a b"[..], FileId::new("lesson.txt"));
        let fragments = (0..3)
            .map(|_| stream.next_fragment(1).unwrap().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            fragments
                .iter()
                .map(|part| part.text.as_str())
                .collect::<String>(),
            "a b"
        );
        assert_eq!(fragments[1].byte_positions, vec![1..2]);
    }

    #[test]
    fn stream_rejects_zero_sized_fragment_and_nul() {
        let mut empty = ExternalTextStream::new(&b"a"[..], FileId::new("lesson.txt"));
        assert_eq!(
            empty.next_fragment(0).unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput
        );
        let mut nul = ExternalTextStream::new(&b"a\0b"[..], FileId::new("lesson.txt"));
        assert_eq!(
            nul.next_fragment(4).unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
    }
}

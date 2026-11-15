//! Folder scanning, natural sorting and navigation.
//!
//! Only file names are read while scanning, so opening a folder with thousands
//! of photos stays fast. File sizes and timestamps are fetched lazily (for the
//! status bar) or on demand when the user sorts by date or size.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use crate::settings::SortOrder;

/// Extensions that the `image` crate is compiled to decode.
///
/// Several formats are spelled in more than one way (`.jpg`/`.jpeg`/`.jfif`,
/// `.tif`/`.tiff`, and the five portable-map extensions), and animated PNGs are
/// sometimes saved as `.apng`, so every accepted spelling is listed here. This
/// list only decides which files appear in the folder listing: the actual
/// decoder is chosen by inspecting the file contents, so a wrong or unusual
/// extension can never send a file to the wrong decoder.
///
/// `svg`, `svgz` and `psd` are listed as well, but they are decoded by Lumen
/// itself (see `image_loader` and `psd.rs`) because the `image` crate has no
/// decoder for them.
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "jfif", "png", "apng", "gif", "bmp", "webp", "tif", "tiff", "ico", "tga", "dds",
    "hdr", "exr", "pnm", "ppm", "pgm", "pbm", "pam", "qoi", "ff", "svg", "svgz", "psd",
];

/// One image file inside the current folder.
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: PathBuf,
    pub name: String,
}

/// Files of the current folder plus the current position inside that list.
#[derive(Default)]
pub struct Library {
    pub folder: Option<PathBuf>,
    pub files: Vec<FileEntry>,
    pub index: usize,
    sort: SortOrder,
}

impl Library {
    /// Reads `folder` and replaces the current file list.
    pub fn open(&mut self, folder: &Path, sort: SortOrder) -> Result<(), String> {
        let entries = std::fs::read_dir(folder).map_err(|error| error.to_string())?;

        let mut files = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if !is_image(&path) {
                continue;
            }
            let Some(name) = path.file_name().map(|name| name.to_string_lossy().into_owned()) else {
                continue;
            };
            files.push(FileEntry { path, name });
        }

        self.folder = Some(folder.to_path_buf());
        self.files = files;
        self.index = 0;
        self.sort = sort;
        if sort.is_metadata_based() {
            // Sorting by date or size needs one `stat` call per file, which can
            // be slow on network drives; note it in the log so it is diagnosable.
            crate::diag::log("library: sorting by metadata (one stat per file)");
        }
        self.apply_sort();
        Ok(())
    }

    /// Re-sorts the list, keeping the current file selected.
    pub fn set_sort(&mut self, sort: SortOrder) {
        if self.sort == sort {
            return;
        }
        self.sort = sort;
        let current = self.current().map(|entry| entry.path.clone());
        self.apply_sort();
        if let Some(path) = current {
            if let Some(index) = self.files.iter().position(|entry| entry.path == path) {
                self.index = index;
            }
        }
    }

    /// Selects a file by its name, ignoring case.
    pub fn select_by_name(&mut self, name: &str) {
        if let Some(index) = self
            .files
            .iter()
            .position(|entry| entry.name.eq_ignore_ascii_case(name))
        {
            self.index = index;
        }
    }

    pub fn current(&self) -> Option<&FileEntry> {
        self.files.get(self.index)
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Moves by `delta` files, clamped to the ends. Returns `true` if the index
    /// actually changed.
    pub fn step(&mut self, delta: isize) -> bool {
        if self.files.is_empty() {
            return false;
        }
        let last = self.files.len() as isize - 1;
        let next = (self.index as isize + delta).clamp(0, last) as usize;
        if next == self.index {
            return false;
        }
        self.index = next;
        true
    }

    /// Moves by `delta` files, wrapping around at both ends (slideshow).
    pub fn step_wrapped(&mut self, delta: isize) -> bool {
        if self.files.is_empty() {
            return false;
        }
        let count = self.files.len() as isize;
        let next = (self.index as isize + delta).rem_euclid(count) as usize;
        if next == self.index {
            return false;
        }
        self.index = next;
        true
    }

    pub fn go_to(&mut self, index: usize) -> bool {
        if index >= self.files.len() || index == self.index {
            return false;
        }
        self.index = index;
        true
    }

    pub fn first(&mut self) -> bool {
        self.go_to(0)
    }

    pub fn last(&mut self) -> bool {
        self.files.len().checked_sub(1).map(|i| self.go_to(i)).unwrap_or(false)
    }

    /// True when the current file is the last one.
    pub fn at_end(&self) -> bool {
        self.index + 1 >= self.files.len()
    }

    fn apply_sort(&mut self) {
        match self.sort {
            SortOrder::NameAscending => self.files.sort_by(|a, b| natural_cmp(&a.name, &b.name)),
            SortOrder::NameDescending => self.files.sort_by(|a, b| natural_cmp(&b.name, &a.name)),
            SortOrder::NewestFirst => {
                let mut keyed = self.take_with_metadata();
                keyed.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| natural_cmp(&a.entry.name, &b.entry.name)));
                self.files = keyed.into_iter().map(|item| item.entry).collect();
            }
            SortOrder::LargestFirst => {
                let mut keyed = self.take_with_metadata();
                keyed.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| natural_cmp(&a.entry.name, &b.entry.name)));
                self.files = keyed.into_iter().map(|item| item.entry).collect();
            }
        }
    }

    fn take_with_metadata(&mut self) -> Vec<KeyedEntry> {
        std::mem::take(&mut self.files)
            .into_iter()
            .map(|entry| {
                let metadata = std::fs::metadata(&entry.path).ok();
                KeyedEntry {
                    size: metadata.as_ref().map(|m| m.len()).unwrap_or(0),
                    modified: metadata.and_then(|m| m.modified().ok()),
                    entry,
                }
            })
            .collect()
    }
}

struct KeyedEntry {
    size: u64,
    modified: Option<std::time::SystemTime>,
    entry: FileEntry,
}

/// True when the path has one of the supported image extensions.
pub fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            let extension = extension.to_ascii_lowercase();
            SUPPORTED_EXTENSIONS.contains(&extension.as_str())
        })
        .unwrap_or(false)
}

/// Compares two file names the way a human would: `photo2` before `photo10`.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let a = a.to_lowercase();
    let b = b.to_lowercase();
    let mut left = a.chars().peekable();
    let mut right = b.chars().peekable();

    loop {
        match (left.peek().copied(), right.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(lc), Some(rc)) => {
                if lc.is_ascii_digit() && rc.is_ascii_digit() {
                    let ln = take_number(&mut left);
                    let rn = take_number(&mut right);
                    if ln != rn {
                        return ln.cmp(&rn);
                    }
                } else {
                    if lc != rc {
                        return lc.cmp(&rc);
                    }
                    left.next();
                    right.next();
                }
            }
        }
    }
}

fn take_number(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> u128 {
    let mut value: u128 = 0;
    while let Some(digit) = chars.peek().and_then(|c| c.to_digit(10)) {
        value = value.saturating_mul(10).saturating_add(digit as u128);
        chars.next();
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_sort_orders_numbers_numerically() {
        let mut names = vec!["photo10.jpg", "photo2.jpg", "Photo1.jpg"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(names, vec!["Photo1.jpg", "photo2.jpg", "photo10.jpg"]);
    }

    #[test]
    fn image_extensions_are_case_insensitive() {
        assert!(is_image(Path::new("a.JPG")));
        assert!(is_image(Path::new("a.png")));
        assert!(!is_image(Path::new("a.txt")));
        assert!(!is_image(Path::new("noextension")));
    }

    #[test]
    fn every_listed_extension_is_a_real_image_extension() {
        // Decoded by Lumen itself, so the `image` crate does not know them.
        const OWN: &[&str] = &["svg", "svgz", "psd"];
        for extension in SUPPORTED_EXTENSIONS {
            assert!(
                image::ImageFormat::from_extension(extension).is_some() || OWN.contains(extension),
                "`{extension}` is neither an extension the image crate knows nor one Lumen decodes itself"
            );
            assert_eq!(
                *extension,
                extension.to_ascii_lowercase(),
                "extensions are compared in lower case, so `{extension}` must be listed in lower case"
            );
        }
    }

    #[test]
    fn the_extension_list_has_no_duplicates() {
        let mut seen = std::collections::HashSet::new();
        for extension in SUPPORTED_EXTENSIONS {
            assert!(seen.insert(*extension), "`{extension}` is listed twice");
        }
    }
}

use std::fs::DirEntry;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug)]
pub struct Filesystem {
    pub contents: Vec<DirectoryEntry>,
}

#[derive(Debug)]
pub enum DirectoryEntry {
    Folder(Folder),
    File(File),
}

#[derive(Debug)]
pub struct Folder {
    pub name: String,
    pub contents: Vec<DirectoryEntry>,
}

#[derive(Debug)]
pub enum File {
    Direct(PathBuf),
    Virtual {
        name: String,
        offset: u64,
        length: u32,
        checksum: u32,
    },
}

#[derive(Error, Debug)]
pub enum FilesystemError {
    #[error("specified path is not a directory {0}")]
    NotADirectory(PathBuf),
    #[error("specified path is not a file")]
    NotAFile(PathBuf),
    #[error("invalid magic value {0}")]
    InvalidMagicValue(String),
}

impl Filesystem {
    /// Initialises a Shaiya filesystem from an existing archive.
    ///
    /// # Arguments
    /// * `header_path`    - The path to the header.
    pub fn from_archive(header_path: &Path) -> anyhow::Result<Self> {
        let metadata = header_path.metadata()?;
        if !metadata.is_file() {
            return Err(FilesystemError::NotAFile(header_path.into()).into());
        }

        let data = std::fs::read(header_path)?;
        crate::io::read_filesystem(Cursor::new(data.as_slice()))
    }

    /// Opens a Shaiya filesystem from a path found on disk.
    ///
    /// # Arguments
    /// * `path`    - The path to the data folder.
    pub fn from_path(path: &Path) -> anyhow::Result<Self> {
        let metadata = path.metadata()?;
        if !metadata.is_dir() {
            return Err(FilesystemError::NotADirectory(path.into()).into());
        }

        let read = std::fs::read_dir(path)?;
        let contents = read
            .map(|dir| Self::map_directory(&dir.unwrap()).unwrap())
            .collect::<Vec<_>>();

        Ok(Self { contents })
    }

    /// Builds the virtual filesystem to temporary files.
    pub fn build(&self) -> anyhow::Result<(std::fs::File, std::fs::File)> {
        let mut header_file = tempfile::tempfile()?;
        let mut data_file = tempfile::tempfile()?;

        crate::io::build_filesystem(self, &mut header_file, &mut data_file)?;

        Ok((header_file, data_file))
    }

    /// Builds the virtual filesystem, to specified files.
    ///
    /// # Arguments
    /// * `header`  - The destination header file.
    /// * `data`    - The destination data file.
    pub fn build_with_destination(
        &self,
        header: &mut std::fs::File,
        data: &mut std::fs::File,
    ) -> anyhow::Result<()> {
        crate::io::build_filesystem(self, header, data)
    }

    /// Maps an directory entry on disk, do a virtual filesystem entry.
    ///
    /// # Arguments
    /// * `entry`   - The the disk entry.
    fn map_directory(entry: &DirEntry) -> anyhow::Result<DirectoryEntry> {
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            let name: String = entry
                .path()
                .components()
                .last()
                .unwrap()
                .as_os_str()
                .to_string_lossy()
                .into();
            let contents = std::fs::read_dir(entry.path())?
                .map(|entry| Self::map_directory(&entry.unwrap()).unwrap())
                .collect::<Vec<_>>();
            return Ok(DirectoryEntry::Folder(Folder { name, contents }));
        }

        Ok(DirectoryEntry::File(File::Direct(entry.path())))
    }
}

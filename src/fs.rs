use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use std::fs::DirEntry;
use std::io::{Cursor, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug)]
pub struct Filesystem {
    pub contents: Vec<DirectoryEntry>,
    archive_data: Option<PathBuf>,
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
    /// * `header_path` - The path to the header.
    /// * `data_path`   - The path to the data file.
    pub fn from_archive(header_path: &Path, data_path: &Path) -> anyhow::Result<Self> {
        let metadata = header_path.metadata()?;
        if !metadata.is_file() {
            return Err(FilesystemError::NotAFile(header_path.into()).into());
        }

        let metadata = data_path.metadata()?;
        if !metadata.is_file() {
            return Err(FilesystemError::NotAFile(data_path.into()).into());
        }

        let data = std::fs::read(header_path)?;
        let contents = crate::io::read_filesystem(Cursor::new(data.as_slice()))?;
        Ok(Filesystem {
            contents,
            archive_data: Some(data_path.into()),
        })
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

        Ok(Self {
            contents,
            archive_data: None,
        })
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
    pub fn build_with_destination<T: Write, K: Write>(
        &self,
        header: &mut T,
        data: &mut K,
    ) -> anyhow::Result<()> {
        crate::io::build_filesystem(self, header, data)
    }

    /// Extracts a virtual filesystem to disk.
    ///
    /// # Arguments
    /// * `dest`    - The destination path.
    pub fn extract(self, dest: &Path) -> anyhow::Result<()> {
        Self::extract_contents(&self.contents, &self.archive_data, dest)
    }

    /// Extracts directory entries to disk.
    ///
    /// # Arguments
    /// * `contents`        - The entries to extract.
    /// * `archive_data`    - The archive data file, if the filesystem contains virtual files.
    /// * `dest`            - The destination to extract to.
    fn extract_contents(
        contents: &[DirectoryEntry],
        archive_data: &Option<PathBuf>,
        dest: &Path,
    ) -> anyhow::Result<()> {
        std::fs::create_dir_all(dest)?;
        contents.par_iter().map(|entry| {
            match entry {
                DirectoryEntry::File(file) => match file {
                    File::Direct(file_path) => {
                        let src = std::fs::read(&file_path)?;

                        let filename = file_path.file_name().expect("invalid file name");
                        let filepath = dest.join(filename);

                        let mut dest = std::fs::File::create(&filepath)?;
                        dest.write_all(&src)?;
                        Ok(())
                    }
                    File::Virtual {
                        name,
                        offset,
                        length,
                        ..
                    } => match archive_data {
                        Some(data) => {
                            let filepath = dest.join(name);
                            let mut file = std::fs::File::create(&filepath)?;

                            // Allocate data for the virtual file's data.
                            let mut buf: Vec<u8> = vec![0; (*length) as usize];

                            // Load the data from the archive file.
                            let mut data_file = std::fs::File::open(&data)?;
                            data_file.seek(SeekFrom::Start(*offset))?;
                            data_file.read_exact(&mut buf)?;

                            // Write the data to the direct file.
                            file.write_all(&buf)?;
                            Ok(())
                        }
                        None => {
                            panic!("trying to extract a virtual file with no archive data file!")
                        }
                    },
                },
                DirectoryEntry::Folder(folder) => {
                    let subpath = dest.join(&folder.name);
                    Self::extract_contents(&folder.contents, archive_data, &subpath)?;
                    Ok(())
                }
            }
        }).collect::<anyhow::Result<()>>()?;
        Ok(())
    }

    /// Maps an directory entry on disk, to a virtual filesystem entry.
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

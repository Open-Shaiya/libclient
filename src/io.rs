use crate::fs::{DirectoryEntry, File, Filesystem, FilesystemError, Folder};
use byteorder::{LittleEndian, ReadBytesExt};
use bytes::{BufMut, BytesMut};
use crc::{Crc, CRC_32_CKSUM};
use std::fs;
use std::io::{Cursor, Read, Seek, SeekFrom, Write};

/// The magic identifier for the header file.
pub const SAH_HEADER_MAGIC: &str = "SAH";

/// The header format version.
pub const HEADER_VERSION: u32 = 0;

/// The name of the root directory.
pub const ROOT_DIRECTORY_NAME: &str = "data";

/// The default capacity of a data.sah buffer (1mb)
pub const DEFAULT_HEADER_CAPACITY: usize = 1_000_000;

/// The default capacity of a data.saf buffer (2gb)
pub const DEFAULT_DATA_CAPACITY: usize = 2_000_000_000; // 2gb

/// Builds the contents of the filesystem, into a header and data file. This allocates a 2gb buffer
/// for the file data.
///
/// # Arguments
/// * `fs`      - The virtual filesystem.
/// * `header`  - The destination file for the header.
/// * `data`    - The destination buffer to write to.
pub fn build_filesystem<T: Write, K: Write>(
    fs: &Filesystem,
    header: &mut T,
    data: &mut K,
) -> anyhow::Result<()> {
    let mut header_buf = BytesMut::with_capacity(DEFAULT_HEADER_CAPACITY);
    let (total_files, _) = write_contents(&fs.contents, &mut header_buf, data, 0)?;

    let mut out = BytesMut::new();
    out.put_slice(SAH_HEADER_MAGIC.as_bytes());
    out.put_u32_le(HEADER_VERSION);
    out.put_u32_le(total_files);
    out.put_bytes(0, 40); // Unknown, assumed to be padding.
    out.put_length_prefixed_string(ROOT_DIRECTORY_NAME);
    out.put_slice(&header_buf);
    out.put_bytes(0, 8); // According to Parsec, the header should end with 8 null bytes (https://github.com/matigramirez/Parsec/blob/7c2e75f95bb5eaff45e22c2b30481a96a06a3016/src/Parsec/Shaiya/Data/Sah.cs#L183)

    // Write the data to the header, and return the data.saf buffer
    header.write_all(&out)?;
    Ok(())
}

/// Serialize the contents of a directory to the header and data buffer.
///
/// # Arguments
/// * `contents`    - The directory contents.
/// * `header`      - The header destination.
/// * `data`        - The data destination.
fn write_contents<T: Write>(
    contents: &[DirectoryEntry],
    header: &mut BytesMut,
    data: &mut T,
    data_offset: u64,
) -> anyhow::Result<(u32, u64)> {
    let (files, folders): (Vec<_>, Vec<_>) = contents
        .iter()
        .partition(|e| matches!(e, DirectoryEntry::File(_)));
    let dir_file_qty = files.len() as u32;
    let mut total_files = dir_file_qty;
    let mut data_offset = data_offset;
    header.put_u32_le(dir_file_qty);
    for file in files {
        match file {
            DirectoryEntry::File(f) => {
                if let File::Direct(path) = f {
                    let file = fs::File::open(path)?;
                    let metadata = file.metadata()?;
                    let file_length = metadata.len() as u32;
                    let name = path.file_name().unwrap().to_string_lossy().to_string();

                    header.put_length_prefixed_string(&name);
                    header.put_u64_le(data_offset);
                    header.put_u32_le(file_length);

                    data_offset += file_length as u64;

                    let file_data = fs::read(path)?;
                    data.write_all(&file_data)?;

                    let crc: Crc<u32> = Crc::<u32>::new(&CRC_32_CKSUM);
                    header.put_u32_le(crc.checksum(&file_data));
                }
            }
            _ => panic!("folder partitioned as file"),
        }
    }
    header.put_u32_le((folders.len()) as u32);
    for folder in folders {
        match folder {
            DirectoryEntry::Folder(f) => {
                header.put_length_prefixed_string(&f.name);
                let (inner_total_files, inner_data_offset) =
                    write_contents(&f.contents, header, data, data_offset)?;
                total_files += inner_total_files;
                data_offset = inner_data_offset;
            }
            _ => panic!("file partitioned as a folder"),
        }
    }
    Ok((total_files, data_offset))
}

/// Constructs a filesystem from an archive header.
///
/// # Arguments
/// * `header`  - The header buffer.
pub fn read_filesystem(mut header: Cursor<&[u8]>) -> anyhow::Result<Vec<DirectoryEntry>> {
    let magic = header.read_fixed_length_string(3)?;
    if magic != SAH_HEADER_MAGIC {
        return Err(FilesystemError::InvalidMagicValue(magic).into());
    }

    let _header_version = header.read_u32::<LittleEndian>()?;
    let _total_files = header.read_u32::<LittleEndian>()?;
    header.seek(SeekFrom::Current(40))?;
    let _root_directory_name = header.read_length_prefixed_string()?;

    read_contents(&mut header)
}

/// Read the contents of a directory from an archive header.
///
/// # Arguments
/// * `header`  - The archive header.
fn read_contents(header: &mut Cursor<&[u8]>) -> anyhow::Result<Vec<DirectoryEntry>> {
    let mut contents = Vec::with_capacity(256);
    let dir_file_qty = header.read_u32::<LittleEndian>()?;
    for _ in 0..dir_file_qty {
        let name = header.read_length_prefixed_string()?;
        let offset = header.read_u64::<LittleEndian>()?;
        let length = header.read_u32::<LittleEndian>()?;
        let checksum = header.read_u32::<LittleEndian>()?;

        contents.push(DirectoryEntry::File(File::Virtual {
            name,
            offset,
            length,
            checksum,
        }));
    }

    let folder_qty = header.read_u32::<LittleEndian>()?;
    for _ in 0..folder_qty {
        let name = header.read_length_prefixed_string()?;
        let folder_contents = read_contents(header)?;

        contents.push(DirectoryEntry::Folder(Folder {
            name,
            contents: folder_contents,
        }));
    }
    Ok(contents)
}

pub trait ShaiyaWrite {
    /// Writes a null-terminated string, where the string is prefixed
    /// with it's length as a little-endian u32.
    ///
    /// # Arguments
    /// * `string`  - The string to write.
    fn put_length_prefixed_string(&mut self, string: &str);
}

pub trait ShaiyaRead {
    /// Reads a string with a fixed number of bytes.
    ///
    /// # Arguments
    /// * `length`  - The number of bytes to read.
    fn read_fixed_length_string(&mut self, length: usize) -> anyhow::Result<String>;

    /// Reads a null-terminated string, where the string is prefixed
    /// with it's length as a little-endian u32.
    fn read_length_prefixed_string(&mut self) -> anyhow::Result<String>;
}

impl<T> ShaiyaWrite for T
where
    T: BufMut,
{
    fn put_length_prefixed_string(&mut self, string: &str) {
        let bytes = string.as_bytes();
        self.put_u32_le((bytes.len() + 1) as u32);
        self.put_slice(bytes);
        self.put_u8(0);
    }
}

impl<T> ShaiyaRead for T
where
    T: Read,
{
    fn read_fixed_length_string(&mut self, length: usize) -> anyhow::Result<String> {
        let mut string = String::with_capacity(length);
        for _ in 0..length {
            let ch = self.read_u8()? as char;
            if ch != '\0' {
                string.push(ch)
            }
        }
        Ok(string)
    }

    fn read_length_prefixed_string(&mut self) -> anyhow::Result<String> {
        let length = self.read_u32::<LittleEndian>()? as usize;
        self.read_fixed_length_string(length)
    }
}

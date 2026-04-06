#![allow(dead_code)]
use std::ffi::OsStr;
use std::fs;
use std::io::{Error, ErrorKind, Result as IoResult, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use fortanix_vme_abi::fs::{FsEntry, FsOpRequest, FsOpResponse};

const ROOT_INO: u64 = 1;

pub struct VmeFs {
    root_path: PathBuf,
}

impl VmeFs {
    pub fn new(root_path: PathBuf) -> Self {
        Self { root_path }
    }

    pub fn handle_request(&self, request: FsOpRequest) -> IoResult<FsOpResponse> {
        match request {
            FsOpRequest::Create { parent, name, metadata, flags } => {
                let entry = self.create(parent, name, metadata, flags)?;
                Ok(FsOpResponse::GetAttr { entry })
            }
            FsOpRequest::GetAttr { ino } => {
                let entry = self.getattr(ino)?;
                Ok(FsOpResponse::GetAttr { entry })
            }
            FsOpRequest::Lookup { ino, name } => {
                let entry = self.lookup(ino, name)?;
                Ok(FsOpResponse::GetAttr { entry })
            }
            FsOpRequest::Mkdir { ino, name, metadata } => {
                let entry = self.mkdir(ino, name, metadata)?;
                Ok(FsOpResponse::GetAttr { entry })
            }
            FsOpRequest::Read { ino } => {
                let content = self.read(ino)?;
                Ok(FsOpResponse::FileContent { content })
            }
            FsOpRequest::ReadDir { ino, offset } => {
                let entries = self.readdir(ino, offset)?;
                Ok(FsOpResponse::ReadDir { entries })
            }
            FsOpRequest::SetAttr { ino, metadata } => {
                let entry = self.setattr(ino, metadata)?;
                Ok(FsOpResponse::GetAttr { entry })
            }
            FsOpRequest::Write { ino, content, flags } => {
                self.write(ino, content, flags)?;
                Ok(FsOpResponse::Empty)
            }
            _ => {
                // Handle other requests (Create, Mkdir, Read, Write) as needed
                unimplemented!()
            }
        }
    }

    fn create(&self, parent: u64, name: String, meta: Vec<u8>, flags: i32) -> IoResult<FsEntry> {
        let parent_path = self.find_dir_by_ino(parent)?;
        let path = parent_path.join(name);

        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);

        let sync_flags = nix::libc::O_SYNC | nix::libc::O_DSYNC;
        if flags & sync_flags != 0 {
            options.custom_flags(flags & sync_flags);
        }

        match options.open(&path) {
            Ok(_) => {
                if let Err(err) = Self::write_meta_file_for_path(&path, &meta, &options) {
                    let _ = fs::remove_file(path);
                    return Err(err);
                }

                let entry = Self::read_fs_entry(&path)?;
                Ok(entry)
            }
            Err(err) => return Err(err),
        }
    }

    fn read(&self, ino: u64) -> IoResult<Vec<u8>> {
        let path = self.find_dir_by_ino(ino)?;
        fs::read(path)
    }

    fn getattr(&self, ino: u64) -> IoResult<FsEntry> {
        let path = self.find_dir_by_ino(ino)?;
        let entry = Self::read_fs_entry(&path)?;
        Ok(entry)
    }

    /// Locates a directory by given inode and returns `FsEntry` of
    /// the file child to the located directory.
    fn lookup(&self, ino: u64, name: String) -> IoResult<FsEntry> {
        let parent_path = self.find_dir_by_ino(ino)?;
        let path = parent_path.join(&name);
        if !path.exists() {
            return Err(Self::file_not_found_err(format!("File not found {} in requested dir (ino: {})", name, ino)));
        }
        let entry = Self::read_fs_entry(&path)?;
        Ok(entry)
    }

    /// Creates a directory along with its meta file.
    fn mkdir(&self, ino: u64, name: String, meta: Vec<u8>) -> IoResult<FsEntry> {
        let parent_path = self.find_dir_by_ino(ino)?;
        let path = parent_path.join(&name);
        if path.exists() {
            return Err(Self::already_exists_err(path.to_string_lossy().into()));
        }
        fs::create_dir(&path)?;
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        if let Err(err) = Self::write_meta_file_for_path(&path, &meta, &options) {
            let _ = fs::remove_dir(path);
            return Err(err);
        }

        let entry = Self::read_fs_entry(&path)?;
        Ok(entry)
    }

    /// Iterates over files/directories and returns their metadata as a response.
    fn readdir(&self, ino: u64, offset: i64) -> IoResult<Vec<FsEntry>> {
        let path = self.find_dir_by_ino(ino)?;
        assert!(!Self::is_metadata_file(&path), "Metadata files should not be accessed directly.");

        let mut all_entries = Vec::new();
        let dir_entries = fs::read_dir(path)?;

        for dir_entry in dir_entries.flatten() {
            let path = dir_entry.path();
            if Self::is_metadata_file(&path) {
                continue;
            }

            let entry = Self::read_fs_entry(&path)?;
            all_entries.push(entry);
        }

        let offset = offset as usize;
        if offset >= all_entries.len() {
            Ok(Vec::new())
        } else {
            Ok(all_entries.into_iter().skip(offset).collect())
        }
    }

    /// Fetches the related metafile associated with ino and updates metafile.
    fn setattr(&self, ino: u64, metadata: Vec<u8>) -> IoResult<FsEntry> {
        let path = self.find_dir_by_ino(ino)?;
        assert!(!Self::is_metadata_file(&path), "Metadata files should not be accessed directly.");

        // Ensure entry exists
        let _ = Self::read_fs_entry(&path)?;
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);

        // If an application calls fchmod() or fchown() on a file opened with O_SYNC
        // there is no way to know this, so we always set O_SYNC for setattr to ensure
        // metadata updates are properly flushed to disk.
        options.custom_flags(nix::libc::O_SYNC);
        Self::write_meta_file_for_path(&path, &metadata, &options)?;
        let entry = Self::read_fs_entry(&path)?;
        Ok(entry)
    }

    fn write(&self, ino: u64, content: Vec<u8>, flags: i32) -> IoResult<()> {
        let path = self.find_dir_by_ino(ino)?;
        assert!(!Self::is_metadata_file(&path), "Metadata files should not be accessed directly.");

        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);

        let sync_flags = nix::libc::O_SYNC | nix::libc::O_DSYNC;
        if flags & sync_flags != 0 {
            options.custom_flags(flags & sync_flags);
        }

        let mut file = options.open(path)?;
        file.write_all(&content)?;

        Ok(())
    }

    fn is_metadata_file(path: &Path) -> bool {
        path.extension() == Some(OsStr::new("meta"))
    }

    fn read_fs_entry(path: &Path) -> IoResult<FsEntry> {
        let fs_metadata = path.metadata()?;
        let host_metadata = fs_metadata.into();
        let mut meta_path = path.to_path_buf();
        meta_path.set_extension("meta");
        if !meta_path.exists() {
            return Err(Self::file_not_found_err(format!("Metadata file not found {:?}", path)));
        }

        let metadata = fs::read(meta_path)?;

        Ok(FsEntry {
            metadata,
            host_metadata,
        })
    }

    fn find_dir_by_ino(&self, ino: u64) -> IoResult<PathBuf> {
        if ino == ROOT_INO {
            return Ok(self.root_path.clone());
        }

        self.find_dir_by_ino_recursive(&self.root_path, ino)
            .ok_or_else(|| Self::file_not_found_err("Inode not found".to_owned()))
    }

    fn find_dir_by_ino_recursive(&self, path: &Path, ino: u64) -> Option<PathBuf> {
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Ok(metadata) = entry.metadata() {
                    // Check if it is a directory, continue otherwise.
                    if !metadata.is_dir() {
                        continue;
                    }

                    if metadata.ino() == ino {
                        return Some(path);
                    }

                    if let Some(found) = self.find_dir_by_ino_recursive(&path, ino) {
                        return Some(found);
                    }
                }
            }
        }

        None
    }

    fn write_meta_file_for_path(path: &Path, content: &[u8], options: &fs::OpenOptions) -> IoResult<()> {
        let mut meta_path = path.to_path_buf();
        meta_path.set_extension("meta");
        let mut file = options.open(meta_path)?;
        file.write_all(content)?;
        Ok(())
    }

    fn file_not_found_err(msg: String) -> Error {
        Error::new(
            ErrorKind::NotFound,
            msg,
        )
    }

    fn already_exists_err(msg: String) -> Error {
        Error::new(
            ErrorKind::AlreadyExists,
            msg,
        )
    }
}

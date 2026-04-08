#![allow(dead_code)]
use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::os::unix::fs as unix_fs;
use std::path::{Path, PathBuf};

use fortanix_vme_abi::{Error as VmeError, ErrorKind};
use fortanix_vme_abi::fs::{FsEntry, FsOpRequest, FsOpResponse, LinkTarget};

use crate::RunnerError;

const ROOT_INO: u64 = 1;

pub struct VmeFs {
    root_path: PathBuf,
}

impl VmeFs {
    pub fn new(root_path: PathBuf) -> Self {
        Self { root_path }
    }

    pub fn initialize(&self) -> Result<(), RunnerError> {
        if self.root_path.exists() {
            if !self.root_path.is_dir() {
                return Err(RunnerError::FilesystemError(
                    format!("Root path {:?} exists but is not a directory", self.root_path),
                ));
            }
        } else {
            std::fs::create_dir_all(&self.root_path)?;
        }

        Ok(())
    }

    pub fn handle_request(&self, request: FsOpRequest) -> Result<FsOpResponse, VmeError> {
        match request {
            FsOpRequest::Create { parent, name, metadata, flags } => {
                let entry = self.create(parent, name, metadata, flags)?;
                Ok(FsOpResponse::GetAttr { entry })
            }
            FsOpRequest::GetAttr { ino } => {
                let entry = self.getattr(ino)?;
                Ok(FsOpResponse::GetAttr { entry })
            }
            FsOpRequest::InitRoot { metadata } => {
                self.initroot(metadata)?;
                Ok(FsOpResponse::Empty)
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
            FsOpRequest::Rename { parent, name, new_parent, new_name } => {
                self.rename(parent, name, new_parent, new_name)?;
                Ok(FsOpResponse::Empty)
            }
            FsOpRequest::RmDir { ino, name } => {
                self.rmdir(ino, name)?;
                Ok(FsOpResponse::Empty)
            }
            FsOpRequest::SetAttr { ino, metadata } => {
                let entry = self.setattr(ino, metadata)?;
                Ok(FsOpResponse::GetAttr { entry })
            }
            FsOpRequest::Symlink { parent, name, target, metadata } => {
                let entry = self.symlink(parent, name, target, metadata)?;
                Ok(FsOpResponse::GetAttr { entry })
            }
            FsOpRequest::Readlink { ino } => {
                let target = self.readlink(ino)?;
                Ok(FsOpResponse::Readlink { target })
            }
            FsOpRequest::Link { ino, new_parent, new_name, metadata } => {
                let entry = self.link(ino, new_parent, new_name, metadata)?;
                Ok(FsOpResponse::GetAttr { entry })
            }
            FsOpRequest::Unlink { ino, name } => {
                self.unlink(ino, name)?;
                Ok(FsOpResponse::Empty)
            }
            FsOpRequest::Write { ino, content, flags } => {
                self.write(ino, content, flags)?;
                Ok(FsOpResponse::Empty)
            }
        }
    }

    fn create(&self, parent: u64, name: String, meta: Vec<u8>, flags: i32) -> Result<FsEntry, VmeError> {
        let parent_path = self.find_path_by_ino(parent)?;
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
            Err(err) => return Err(err.into()),
        }
    }

    fn read(&self, ino: u64) -> Result<Vec<u8>, VmeError> {
        let path = self.find_path_by_ino(ino)?;
        let read = fs::read(path)?;
        Ok(read)
    }

    fn getattr(&self, ino: u64) -> Result<FsEntry, VmeError> {
        let path = self.find_path_by_ino(ino)?;
        let entry = Self::read_fs_entry(&path)?;
        Ok(entry)
    }

    fn initroot(&self, metadata: Vec<u8>) -> Result<(), VmeError> {
        if let Err(_) = Self::read_fs_entry(&self.root_path) {
            let mut options = fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            Self::write_meta_file_for_path(&self.root_path, &metadata, &options)?;
        }

        Ok(())
    }

    /// Locates a directory by given inode and returns `FsEntry` of
    /// the file child to the located directory.
    fn lookup(&self, ino: u64, name: String) -> Result<FsEntry, VmeError> {
        let parent_path = self.find_path_by_ino(ino)?;
        let path = parent_path.join(&name);
        if !path.exists() {
            return Err(Self::file_not_found_err());
        }
        let entry = Self::read_fs_entry(&path)?;
        Ok(entry)
    }

    /// Creates a directory along with its meta file.
    fn mkdir(&self, ino: u64, name: String, meta: Vec<u8>) -> Result<FsEntry, VmeError> {
        let parent_path = self.find_path_by_ino(ino)?;
        let path = parent_path.join(&name);
        if path.exists() {
            return Err(Self::already_exists_err());
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
    fn readdir(&self, ino: u64, offset: i64) -> Result<Vec<FsEntry>, VmeError> {
        let path = self.find_path_by_ino(ino)?;
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

    fn rename(
        &self,
        parent: u64,
        name: String,
        new_parent: u64,
        new_name: String,
    ) -> Result<(), VmeError> {
        let parent_path = self.find_path_by_ino(parent)?;
        assert!(!Self::is_metadata_file(&parent_path), "Metadata files should not be accessed directly.");

        let new_parent_path = self.find_path_by_ino(new_parent)?;
        assert!(!Self::is_metadata_file(&new_parent_path), "Metadata files should not be accessed directly.");

        let path = parent_path.join(name);
        if !path.exists() {
            log::warn!("Rename src '{:?}' could not be found.", path);
            return Err(Self::file_not_found_err());
        }

        let path_meta = {
            let mut path = path.clone();
            path.set_extension("meta");
            path
        };
        if !path_meta.exists() {
            log::warn!("Rename src meta '{:?}' could not be found.", path_meta);
            return Err(Self::file_not_found_err());
        }

        let new_path = new_parent_path.join(new_name);
        let new_path_meta = {
            let mut path = new_path.clone();
            path.set_extension("meta");
            path
        };

        fs::rename(path, new_path)?;
        fs::rename(path_meta, new_path_meta)?;

        Ok(())
    }

    fn rmdir(&self, ino: u64, name: String) -> Result<(), VmeError> {
        let parent = self.find_path_by_ino(ino)?;
        assert!(!Self::is_metadata_file(&parent), "Metadata files should not be accessed directly.");

        let path = parent.join(name);
        if !path.exists() {
            return Err(Self::file_not_found_err());
        }
        if !path.metadata()?.is_dir() {
            return Err(Self::not_directory_err());
        }

        let meta = {
            let mut path = path.clone();
            path.set_extension("meta");
            path
        };
        if !meta.exists() {
            return Err(Self::file_not_found_err());
        }

        fs::remove_dir(path)?;
        fs::remove_file(meta)?;

        Ok(())
    }

    /// Fetches the related metafile associated with ino and updates metafile.
    fn setattr(&self, ino: u64, metadata: Vec<u8>) -> Result<FsEntry, VmeError> {
        let path = self.find_path_by_ino(ino)?;
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

    fn unlink(&self, ino: u64, name: String) -> Result<(), VmeError> {
        let parent = self.find_path_by_ino(ino)?;
        assert!(!Self::is_metadata_file(&parent), "Metadata files should not be accessed directly.");

        let path = parent.join(name);
        if !path.exists() {
            return Err(Self::file_not_found_err());
        }
        if path.metadata()?.is_dir() {
            return Err(Self::is_directory_err());
        }

        let meta = {
            let mut path = path.clone();
            path.set_extension("meta");
            path
        };
        if !meta.exists() {
            return Err(Self::file_not_found_err());
        }

        fs::remove_file(path)?;
        fs::remove_file(meta)?;

        Ok(())
    }

    fn write(&self, ino: u64, content: Vec<u8>, flags: i32) -> Result<(), VmeError> {
        let path = self.find_path_by_ino(ino)?;
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

    fn read_fs_entry(path: &Path) -> Result<FsEntry, VmeError> {
        let fs_metadata = path.metadata()?;
        let host_metadata = fs_metadata.into();
        let mut meta_path = path.to_path_buf();
        meta_path.set_extension("meta");
        if !meta_path.exists() {
            return Err(Self::file_not_found_err());
        }

        let metadata = fs::read(meta_path)?;

        Ok(FsEntry {
            metadata,
            host_metadata,
        })
    }

    fn link(&self, ino: u64, new_parent: u64, new_name: String, meta: Vec<u8>) -> Result<FsEntry, VmeError> {
        let path = self.find_path_by_ino(ino)?;
        let parent_path = self.find_path_by_ino(new_parent)?;
        let new_path = parent_path.join(new_name);
        fs::hard_link(path, &new_path)?;
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        Self::write_meta_file_for_path(&new_path, &meta, &options)?;
        Self::read_fs_entry(&new_path)
    }

    fn readlink(&self, ino: u64) -> Result<Vec<u8>, VmeError> {
        let path = self.find_path_by_ino(ino)?;
        eprintln!("Reading link: {:?}", path);
        let target = fs::read(path)?;
        Ok(target)
    }

    fn symlink(&self, parent: u64, name: String, target: LinkTarget, meta: Vec<u8>) -> Result<FsEntry, VmeError> {
        let parent_path = self.find_path_by_ino(parent)?;
        let path = parent_path.join(name);
        let target = match target {
            LinkTarget::Absolute(target) => Path::new(&target).to_path_buf(),
            LinkTarget::Relative(target) => self.root_path.join(target),
        };
        unix_fs::symlink(target, &path)?;
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        Self::write_meta_file_for_path(&path, &meta, &options)?;
        Self::read_fs_entry(&path)
    }

    fn find_path_by_ino(&self, ino: u64) -> Result<PathBuf, VmeError> {
        if ino == ROOT_INO {
            return Ok(self.root_path.clone());
        }

        self.find_path_by_ino_recursive(&self.root_path, ino)
            .ok_or_else(|| Self::file_not_found_err())
    }

    fn find_path_by_ino_recursive(&self, path: &Path, ino: u64) -> Option<PathBuf> {
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Ok(metadata) = entry.metadata() {
                    if metadata.ino() == ino {
                        return Some(path);
                    }

                    // DFS for directories
                    if metadata.is_dir() {
                        if let Some(found) = self.find_path_by_ino_recursive(&path, ino) {
                            return Some(found);
                        }
                    }
                }
            }
        }

        None
    }

    fn write_meta_file_for_path(path: &Path, content: &[u8], options: &fs::OpenOptions) -> Result<(), VmeError> {
        let mut meta_path = path.to_path_buf();
        meta_path.set_extension("meta");
        let mut file = options.open(meta_path)?;
        file.write_all(content)?;
        Ok(())
    }

    fn file_not_found_err() -> VmeError {
        VmeError::Command(ErrorKind::NotFound)
    }

    fn not_directory_err() -> VmeError {
        VmeError::Command(ErrorKind::NotADirectory)
    }

    fn is_directory_err() -> VmeError {
        VmeError::Command(ErrorKind::IsADirectory)
    }

    fn not_empty_err() -> VmeError {
        VmeError::Command(ErrorKind::DirectoryNotEmpty)
    }

    fn already_exists_err() -> VmeError {
        VmeError::Command(ErrorKind::AlreadyExists)
    }
}

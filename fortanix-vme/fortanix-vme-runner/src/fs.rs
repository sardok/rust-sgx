#![allow(dead_code)]
use std::ffi::OsStr;
use std::fs;
use std::io::{Error, ErrorKind, Result as IoResult};
use std::os::unix::fs::MetadataExt;
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
            FsOpRequest::GetAttr { ino } => {
                let path = self.find_dir_by_ino(ino)?;
                let entry = Self::read_fs_entry(&path)?;
                Ok(FsOpResponse::GetAttr { entry })
            }
            FsOpRequest::ReadDir { ino } => {
                let entries = self.read_dir(ino)?;
                Ok(FsOpResponse::ReadDir { entries })
            }
            _ => {
                // Handle other requests (Create, Mkdir, Read, Write) as needed
                unimplemented!()
            }
        }
    }

    /// Iterates over files/directories and returns their metadata as a response.
    fn read_dir(&self, ino: u64) -> IoResult<Vec<FsEntry>> {
        let path = self.find_dir_by_ino(ino)?;
        assert!(!Self::is_metadata_file(&path), "Metadata files should not be listed");

        let mut entries = Vec::new();
        let dir_entries = fs::read_dir(path)?;

        for dir_entry in dir_entries.flatten() {
            let path = dir_entry.path();
            if Self::is_metadata_file(&path) {
                continue;
            }

            let entry = Self::read_fs_entry(&path)?;
            entries.push(entry);
        }

        Ok(entries)
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
            return Err(Error::new(
                ErrorKind::NotFound,
                format!("Metadata file not found for {:?}", path),
            ));
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
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "Inode not found"))
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
}

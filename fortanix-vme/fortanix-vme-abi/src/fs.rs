use std::vec::Vec;
use std::fs::Metadata;
use std::os::unix::fs::MetadataExt;
use serde::{Deserialize, Serialize};


#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum FsOpRequest {
    Create,
    GetAttr {
        ino: u64,
    },
    Mkdir,
    Read,
    ReadDir {
        ino: u64,
    },
    SetAttr {
        ino: u64,
        metadata: Vec<u8>,
    },
    Write,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum FileType {
    RegularFile,
    Directory,
    Symlink,
}

impl From<Metadata> for HostMetadata {
    fn from(metadata: Metadata) -> Self {
        let kind = if metadata.is_file() {
            FileType::RegularFile
        } else if metadata.is_dir() {
            FileType::Directory
        } else if metadata.is_symlink() {
            FileType::Symlink
        } else {
            panic!("Unsupported file type");
        };

        HostMetadata {
            ino: metadata.ino(),
            kind,
        }
    }
}

/// This struct contains non-encrypted metadata about a file/directory entry
/// to be used by enclave fuse implementation as is.
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct HostMetadata {
    pub ino: u64,
    pub kind: FileType,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct FsEntry {
    pub metadata: Vec<u8>,
    pub host_metadata: HostMetadata,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum FsOpResponse {
    GetAttr {
        entry: FsEntry,
    },
    ReadDir {
        entries: Vec<FsEntry>,
    }
}

use std::fs::Metadata;
use std::os::unix::fs::MetadataExt;
use std::string::String;
use std::vec::Vec;
use serde::{Deserialize, Serialize};


#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum FsOpRequest {
    Create {
        // inode of parent
        parent: u64,
        name: String,
        metadata: Vec<u8>,
    },
    GetAttr {
        ino: u64,
    },
    Lookup {
        ino: u64,
        name: String,
    },
    Mkdir {
        // inode of parent
        ino: u64,
        name: String,
        metadata: Vec<u8>,
    },
    Read {
        ino: u64,
    },
    ReadDir {
        ino: u64,
    },
    RmDir {
        ino: u64,
        name: String,
    },
    SetAttr {
        ino: u64,
        metadata: Vec<u8>,
    },
    Unlink {
        ino: u64,
        name: String,
    },
    Write {
        ino: u64,
        content: Vec<u8>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
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
            blocks: metadata.blocks(),
            ino: metadata.ino(),
            kind,
            nlink: metadata.nlink() as u32,
            rdev: metadata.rdev() as u32,
            atime: metadata.atime() as u64,
            mtime: metadata.mtime() as u64,
            ctime: metadata.ctime() as u64,
        }
    }
}

/// This struct contains non-encrypted metadata about a file/directory entry
/// to be used by enclave fuse implementation as is.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct HostMetadata {
    pub blocks: u64,
    pub ino: u64,
    pub kind: FileType,
    pub nlink: u32,
    pub rdev: u32,
    pub atime: u64,
    pub mtime: u64,
    pub ctime: u64,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct FsEntry {
    pub metadata: Vec<u8>,
    pub host_metadata: HostMetadata,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum FsOpResponse {
    Empty,
    GetAttr {
        entry: FsEntry,
    },
    FileContent {
        content: Vec<u8>,
    },
    ReadDir {
        entries: Vec<FsEntry>,
    },
}

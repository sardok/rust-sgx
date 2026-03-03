use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
pub enum FsOp {
    Create,
    Mkdir,
    Read,
    ReadDir,
    Write,
}

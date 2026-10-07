/// One source file. The engine's VFS hands these out after canonicalizing
/// the path, symlinks resolved, so each file has exactly one (R7); nothing
/// else creates them except tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileId(u32);

impl FileId {
    pub const fn from_raw(raw: u32) -> FileId {
        FileId(raw)
    }

    pub const fn raw(self) -> u32 {
        self.0
    }
}

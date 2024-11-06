//! File trait & inode(dir, file, pipe, stdin, stdout)

mod inode;
mod stdio;

use core::any::Any;

use crate::mm::UserBuffer;

/// Implement in [CH6]
pub trait AToAny: 'static {
    /// TODO
    fn as_any(&self) -> &dyn Any;
}

/// Implement in [CH6]
impl<T: 'static> AToAny for T {
    /// TODO
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Implement in [CH6]
/// Trait File for all file types
pub trait File: Send + Sync + AToAny {
    /// The file readable?
    fn readable(&self) -> bool;
    /// The file writable?
    fn writable(&self) -> bool;
    /// Read from the file to buf, return the number of bytes read
    /// 从文件(即`IO`资源)中读取数据放到缓冲区中,最多将缓冲区填满(即读取缓冲区的长度那么多字节),并返回实际读取的字节数
    fn read(&self, buf: UserBuffer) -> usize;
    /// Write to the file from buf, return the number of bytes written
    /// 将缓冲区中的数据写入文件,最多将缓冲区中的数据全部写入,并返回直接写入的字节数
    fn write(&self, buf: UserBuffer) -> usize;
}

/// The stat of a inode
/// Change in [CH6]
/// Let `pad` from private to public
#[repr(C)]
#[derive(Debug)]
pub struct Stat {
    /// ID of device containing file
    pub dev: u64,
    /// inode number
    pub ino: u64,
    /// file type and mode
    pub mode: StatMode,
    /// number of hard links
    pub nlink: u32,
    /// unused pad
    pub pad: [u64; 7],
}

bitflags! {
    /// The mode of a inode
    /// whether a directory or a file
    pub struct StatMode: u32 {
        /// null
        const NULL  = 0;
        /// directory
        const DIR   = 0o040000;
        /// ordinary regular file
        const FILE  = 0o100000;
    }
}

/// Change in [CH6]
pub use inode::{list_apps, open_file, OSInode, OpenFlags, ROOT_INODE};
pub use stdio::{Stdin, Stdout};

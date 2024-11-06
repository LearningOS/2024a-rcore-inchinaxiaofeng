//! `Arc<Inode>` -> `OSInodeInner`: In order to open files concurrently
//! we need to wrap `Inode` into `Arc`,but `Mutex` in `Inode` prevents
//! file systems from being accessed simultaneously
//!
//! `UPSafeCell<OSInodeInner>` -> `OSInode`: for static `ROOT_INODE`,we
//! need to wrap `OSInodeInner` into `UPSafeCell`
use super::File;
use crate::drivers::BLOCK_DEVICE;
use crate::mm::UserBuffer;
use crate::sync::UPSafeCell;
use alloc::sync::Arc;
use alloc::vec::Vec;
use bitflags::*;
use easy_fs::{EasyFileSystem, Inode};
use lazy_static::*;

/// Inode in memory
/// A wrapper around an `filesystem` inode
/// to implement File trait atop
/// 将Inode进一步封装为OSInode
/// Change in [CH6], let inner from private to public
pub struct OSInode {
    readable: bool,
    writable: bool,
    /// Todo
    pub inner: UPSafeCell<OSInodeInner>,
}

/// The OS inode inner in 'UPSafeCell'
/// Change in [CH6], let inode from private to public
pub struct OSInodeInner {
    offset: usize,
    pub inode: Arc<Inode>,
}

impl OSInode {
    /// Create a new inode in memory
    pub fn new(readable: bool, writable: bool, inode: Arc<Inode>) -> Self {
        Self {
            readable,
            writable,
            inner: unsafe { UPSafeCell::new(OSInodeInner { offset: 0, inode }) },
        }
    }
    /// Read all data from the inode
    pub fn read_all(&self) -> Vec<u8> {
        let mut inner = self.inner.exclusive_access();
        let mut buffer = [0u8; 512];
        let mut v: Vec<u8> = Vec::new();
        loop {
            let len = inner.inode.read_at(inner.offset, &mut buffer);
            if len == 0 {
                break;
            }
            inner.offset += len;
            v.extend_from_slice(&buffer[..len]);
        }
        v
    }

    /// Implement in [CH6]
    /// get current node id
    pub fn get_inode_id(&self) -> u64 {
        let inner = self.inner.exclusive_access();
        inner.inode.block_id as u64
    }
    /// Implement in [CH6]
    /// get inode 'block_id' and 'block_offset'
    pub fn get_inode_pos(&self) -> (usize, usize) {
        let inner = self.inner.exclusive_access();
        (inner.inode.block_id, inner.inode.block_offset)
    }
}

lazy_static! {
    /// Todo
    pub static ref ROOT_INODE: Arc<Inode> = {
        let efs = EasyFileSystem::open(BLOCK_DEVICE.clone());
        Arc::new(EasyFileSystem::root_inode(&efs))
    };
}

/// List all apps in the root directory
/// 这之后就可以使用根目录的`inode ROOT_INODE`，在内核中调用`easy-fs`的相关接口了。
/// 例如，在文件系统初始化完毕之后，调用`list_apps`函数来打印所有可用应用的文件名
pub fn list_apps() {
    println!("/**** APPS ****");
    for app in ROOT_INODE.ls() {
        println!("{}", app);
    }
    println!("**************/");
}

bitflags! {
    ///  The flags argument to the open() system call is constructed by `ORing` together zero or more of the following values:
    pub struct OpenFlags: u32 {
        /// Ready only
        const RDONLY = 0;
        /// Write only
        const WRONLY = 1 << 0;
        /// Read and write
        const RDWR = 1 << 1;
        /// Create new file
        const CREATE = 1 << 9;
        /// Truncate file size to 0
        const TRUNC = 1 << 10;
    }
}

impl OpenFlags {
    /// Do not check validity for simplicity
    /// Return (readable, writable)
    /// 它的 read_write 方法可以根据标志的情况返回要打开的文件是否允许读写。
    /// 简单起见，这里假设标志自身一定合法。
    pub fn read_write(&self) -> (bool, bool) {
        if self.is_empty() {
            (true, false)
        } else if self.contains(Self::WRONLY) {
            (false, true)
        } else {
            (true, true)
        }
    }
}

/// Open a file
/// 这里主要是实现了`OpenFlags`各标志位的语义。
/// 例如只有`flags`参数包含`CREATE`标志位才允许创建文件；
/// 而如果文件已经存在，则清空文件的内容。
pub fn open_file(name: &str, flags: OpenFlags) -> Option<Arc<OSInode>> {
    let (readable, writable) = flags.read_write();
    if flags.contains(OpenFlags::CREATE) {
        if let Some(inode) = ROOT_INODE.find(name) {
            // Clear size
            inode.clear();
            Some(Arc::new(OSInode::new(readable, writable, inode)))
        } else {
            // Create file
            ROOT_INODE
                .create(name)
                .map(|inode| Arc::new(OSInode::new(readable, writable, inode)))
        }
    } else {
        ROOT_INODE.find(name).map(|inode| {
            if flags.contains(OpenFlags::TRUNC) {
                inode.clear();
            }
            Arc::new(OSInode::new(readable, writable, inode))
        })
    }
}

impl File for OSInode {
    /// 是否是可读的
    fn readable(&self) -> bool {
        self.readable
    }
    /// 是否是可写的
    fn writable(&self) -> bool {
        self.writable
    }
    /// 只需遍历`UserBuffer`中的每个缓冲区片段，调用`Inode`写好的`read`接口就好了
    fn read(&self, mut buf: UserBuffer) -> usize {
        let mut inner = self.inner.exclusive_access();
        let mut total_read_size = 0usize;
        for slice in buf.buffers.iter_mut() {
            let read_size = inner.inode.read_at(inner.offset, *slice);
            if read_size == 0 {
                break;
            }
            inner.offset += read_size;
            total_read_size += read_size;
        }
        total_read_size
    }
    /// 只需遍历`UserBuffer`中的每个缓冲区片段，调用`Inode`写好的`write_at`接口就好了
    fn write(&self, buf: UserBuffer) -> usize {
        let mut inner = self.inner.exclusive_access();
        let mut total_write_size = 0usize;
        for slice in buf.buffers.iter() {
            let write_size = inner.inode.write_at(inner.offset, *slice);
            assert_eq!(write_size, slice.len());
            inner.offset += write_size;
            total_write_size += write_size;
        }
        total_write_size
    }
}

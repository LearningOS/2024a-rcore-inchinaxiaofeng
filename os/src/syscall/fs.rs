//! `File and filesystem-related syscalls`

use crate::fs::{open_file, OSInode, OpenFlags, Stat, StatMode, ROOT_INODE};
use crate::mm::{translated_byte_buffer, translated_str, UserBuffer};
use crate::task::{current_task, current_user_token};
use core::any::Any;

/// 让其更有普适性
pub fn sys_write(fd: usize, buf: *const u8, len: usize) -> isize {
    trace!("kernel:pid[{}] sys_write", current_task().unwrap().pid.0);
    let token = current_user_token();
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if let Some(file) = &inner.fd_table[fd] {
        if !file.writable() {
            return -1;
        }
        let file = file.clone();
        // Release current task `TCB` manually to avoid multi-borrow
        drop(inner);
        file.write(UserBuffer::new(translated_byte_buffer(token, buf, len))) as isize
    } else {
        -1
    }
}

/// 让其更有普适性
pub fn sys_read(fd: usize, buf: *const u8, len: usize) -> isize {
    trace!("kernel:pid[{}] sys_read", current_task().unwrap().pid.0);
    let token = current_user_token();
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if let Some(file) = &inner.fd_table[fd] {
        let file = file.clone();
        if !file.readable() {
            return -1;
        }
        // Release current task `TCB` manually to avoid multi-borrow
        drop(inner);
        trace!("kernel: sys_read .. file.read");
        file.read(UserBuffer::new(translated_byte_buffer(token, buf, len))) as isize
    } else {
        -1
    }
}

/// **功能**：打开一个常规文件，并返回可以访问它的文件描述符。
/// **参数**：`path`描述要打开的文件的文件名（简单起见，文件系统不需要支持目录，所有的文件都放在根目录 / 下），
/// `flags`描述打开文件的标志，具体含义下面给出。
/// `dirfd`和`mode`仅用于保证兼容性，忽略。
/// 返回值：如果出现了错误则返回 -1，否则返回打开常规文件的文件描述符。可能的错误原因是：文件不存在。
/// `syscall ID`：56
///
/// 目前我们的内核支持以下几种标志（多种不同标志可能共存）：
/// * 如果`flags`为`0`，则表示以只读模式`RDONLY`打开；
/// * 如果`flags`第`0`位被设置（0x001），表示以只写模式`WRONLY`打开；
/// * 如果`flags`第`1`位被设置（0x002），表示既可读又可写`RDWR`；
/// * 如果`flags`第`9`位被设置（0x200），表示允许创建文件`CREATE`，在找不到该文件的时候应创建文件；如果该文件已经存在则应该将该文件的大小归零；
/// * 如果`flags`第`10`位被设置（0x400），则在打开文件的时候应该清空文件的内容并将该文件的大小归零，也即`TRUNC`。
pub fn sys_open(path: *const u8, flags: u32) -> isize {
    trace!("kernel:pid[{}] sys_open", current_task().unwrap().pid.0);
    let task = current_task().unwrap();
    let token = current_user_token();
    let path = translated_str(token, path);
    if let Some(inode) = open_file(path.as_str(), OpenFlags::from_bits(flags).unwrap()) {
        let mut inner = task.inner_exclusive_access();
        let fd = inner.alloc_fd();
        inner.fd_table[fd] = Some(inode);
        fd as isize
    } else {
        -1
    }
}

pub fn sys_close(fd: usize) -> isize {
    trace!("kernel:pid[{}] sys_close", current_task().unwrap().pid.0);
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    if fd >= inner.fd_table.len() {
        return -1;
    }
    if inner.fd_table[fd].is_none() {
        return -1;
    }
    inner.fd_table[fd].take();
    0
}

/// YOUR JOB: Implement fstat.
/// Implement in [CH6]
pub fn sys_fstat(fd: usize, st: *mut Stat) -> isize {
    trace!(
        "kernel:pid[{}] sys_fstat NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    let task = current_task().unwrap();
    let inner = task.inner_exclusive_access();

    if fd >= inner.fd_table.len() {
        return -1;
    }
    if inner.fd_table[fd].is_none() {
        return -1;
    }

    let ino: u64;
    let nlink: u32;

    //    let mut ino = 0 as u64;
    //    let mut nlink = 0 as u32;
    if let Some(file_node) = &inner.fd_table[fd] {
        let any: &dyn Any = file_node.as_any();
        let os_node = any.downcast_ref::<OSInode>().unwrap();
        ino = os_node.get_inode_id();
        let (block_id, block_offset) = os_node.get_inode_pos();
        nlink = ROOT_INODE.get_link_num(block_id, block_offset);
    } else {
        return -1;
    }

    let stat = &Stat {
        dev: 0,
        ino: ino,
        mode: StatMode::FILE,
        nlink: nlink,
        pad: [0; 7],
    };

    // Copy data from kernel space to user space
    let token = inner.get_user_token();
    let st = translated_byte_buffer(token, st as *const u8, core::mem::size_of::<Stat>());
    let stat_ptr = stat as *const _ as *const u8;
    for (idx, byte) in st.into_iter().enumerate() {
        unsafe {
            byte.copy_from_slice(core::slice::from_raw_parts(
                stat_ptr.wrapping_byte_add(idx),
                byte.len(),
            ));
        }
    }
    0
}

/// Implement in [CH6]
pub fn sys_linkat(old_name: *const u8, new_name: *const u8) -> isize {
    trace!(
        "kernel:pid[{}] sys_linkat NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    let token = current_user_token();
    let old = translated_str(token, old_name);
    let new = translated_str(token, new_name);
    println!("link {} to {}", new, old);
    if old.as_str() != new.as_str() {
        if let Some(_) = ROOT_INODE.link(old.as_str(), new.as_str()) {
            return 0;
        }
    }
    -1
}

/// YOUR JOB: Implement unlinkat.
pub fn sys_unlinkat(name: *const u8) -> isize {
    trace!(
        "kernel:pid[{}] sys_unlinkat NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    let token = current_user_token();
    let name = translated_str(token, name);
    if let Some(inode) = ROOT_INODE.find(name.as_str()) {
        if ROOT_INODE.get_link_num(inode.block_id, inode.block_offset) == 1 {
            // Clear data if only one link exists
            inode.clear();
        }
        return ROOT_INODE.unlink(name.as_str());
    }
    -1
}

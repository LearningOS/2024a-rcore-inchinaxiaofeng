//! Process management syscalls
//!
use alloc::sync::Arc;

use crate::{
    config::{MAXVA, MAX_SYSCALL_NUM, PAGE_SIZE, TRAP_CONTEXT_BASE},
    fs::{open_file, File, OpenFlags},
    mm::{
        translated_byte_buffer, translated_refmut, translated_str, MapPermission, MemorySet,
        VPNRange, VirtAddr, KERNEL_SPACE,
    },
    sync::UPSafeCell,
    task::{
        add_task, create_new_map_area, current_task, current_user_token, exit_current_and_run_next,
        get_current_task_page_table, get_current_task_status, get_current_task_syscall_times,
        kstack_alloc, pid_alloc, suspend_current_and_run_next, unmap_consecutive_area, TaskContext,
        TaskControlBlock, TaskControlBlockInner, TaskStatus,
    },
    timer::{get_time_ms, get_time_us},
    trap::{trap_handler, TrapContext},
};
use alloc::vec::Vec;

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// Task information
#[allow(dead_code)]
pub struct TaskInfo {
    /// Task status in it's life cycle
    status: TaskStatus,
    /// The numbers of syscall called by task
    syscall_times: [u32; MAX_SYSCALL_NUM],
    /// Total running time of task
    time: usize,
}

pub fn sys_exit(exit_code: i32) -> ! {
    trace!("kernel:pid[{}] sys_exit", current_task().unwrap().pid.0);
    exit_current_and_run_next(exit_code);
    panic!("Unreachable in sys_exit!");
}

pub fn sys_yield() -> isize {
    //trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

pub fn sys_getpid() -> isize {
    trace!("kernel: sys_getpid pid:{}", current_task().unwrap().pid.0);
    current_task().unwrap().pid.0 as isize
}

pub fn sys_fork() -> isize {
    trace!("kernel:pid[{}] sys_fork", current_task().unwrap().pid.0);
    let current_task = current_task().unwrap();
    let new_task = current_task.fork();
    let new_pid = new_task.pid.0;
    // modify trap context of new_task, because it returns immediately after switching
    let trap_cx = new_task.inner_exclusive_access().get_trap_cx();
    // we do not have to move to next instruction since we have done it before
    // for child process, fork returns 0
    trap_cx.x[10] = 0;
    // add new task to scheduler
    add_task(new_task);
    new_pid as isize
}

pub fn sys_exec(path: *const u8) -> isize {
    trace!("kernel:pid[{}] sys_exec", current_task().unwrap().pid.0);
    let token = current_user_token();
    let path = translated_str(token, path);
    if let Some(app_inode) = open_file(path.as_str(), OpenFlags::RDONLY) {
        let all_data = app_inode.read_all();
        let task = current_task().unwrap();
        task.exec(all_data.as_slice());
        0
    } else {
        -1
    }
}

/// If there is not a child process whose pid is same as given, return -1.
/// Else if there is a child process but it is still running, return -2.
pub fn sys_waitpid(pid: isize, exit_code_ptr: *mut i32) -> isize {
    //trace!("kernel: sys_waitpid");
    let task = current_task().unwrap();
    // find a child process

    // ---- access current PCB exclusively
    let mut inner = task.inner_exclusive_access();
    if !inner
        .children
        .iter()
        .any(|p| pid == -1 || pid as usize == p.getpid())
    {
        return -1;
        // ---- release current PCB
    }
    let pair = inner.children.iter().enumerate().find(|(_, p)| {
        // ++++ temporarily access child PCB exclusively
        p.inner_exclusive_access().is_zombie() && (pid == -1 || pid as usize == p.getpid())
        // ++++ release child PCB
    });
    if let Some((idx, _)) = pair {
        let child = inner.children.remove(idx);
        // confirm that child will be deallocated after being removed from children list
        assert_eq!(Arc::strong_count(&child), 1);
        let found_pid = child.getpid();
        // ++++ temporarily access child PCB exclusively
        let exit_code = child.inner_exclusive_access().exit_code;
        // ++++ release child PCB
        *translated_refmut(inner.memory_set.token(), exit_code_ptr) = exit_code;
        found_pid as isize
    } else {
        -2
    }
    // ---- release current PCB automatically
}

/// Implement in [CH3], re implement in [CH6]
/// Implement by myself
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!(
        "kernel:pid[{}] sys_get_time NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    let us = get_time_us();
    let dst_vec = translated_byte_buffer(
        current_user_token(),
        ts as *const u8,
        core::mem::size_of::<TimeVal>(),
    );
    let ref time_val = TimeVal {
        sec: us / 1_000_000,
        usec: us % 1_000_000,
    };
    let src_ptr = time_val as *const TimeVal;
    for (idx, dst) in dst_vec.into_iter().enumerate() {
        let unit_len = dst.len();
        unsafe {
            dst.copy_from_slice(core::slice::from_raw_parts(
                src_ptr.wrapping_byte_add(idx * unit_len) as *const u8,
                unit_len,
            ));
        }
    }
    0
}

/// Implement in [CH3], re implement in [CH5] We re implement this function use the function as follow:
/// * `get_current_task_status()`
/// * `get_current_task_syscall_times()`
/// * `get_current_task_time_cost()`
pub fn sys_task_info(ti: *mut TaskInfo) -> isize {
    trace!(
        "kernel:pid[{}] sys_task_info NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    let dst_vec = translated_byte_buffer(
        current_user_token(),
        ti as *const u8,
        core::mem::size_of::<TaskInfo>(),
    );
    let ref task_info = TaskInfo {
        status: get_current_task_status(),
        syscall_times: get_current_task_syscall_times(),
        time: get_time_ms(),
    };

    let src_ptr = task_info as *const TaskInfo;
    for (idx, dst) in dst_vec.into_iter().enumerate() {
        let unit_len = dst.len();
        unsafe {
            dst.copy_from_slice(core::slice::from_raw_parts(
                src_ptr.wrapping_byte_add(idx * unit_len) as *const u8,
                unit_len,
            ));
        }
    }
    0
}
/// Implement in [CH5], function `mmap()`.
/// `Mmap` the mapped virtual address
pub fn sys_mmap(start: usize, len: usize, port: usize) -> isize {
    trace!(
        "kernel:pid[{}] sys_mmap NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    if start % PAGE_SIZE != 0 /* Start need to be page aligned */ ||
        port & !0x7 != 0 /* Other bits of needs to be zero */ ||
        port & 0x7 == 0 /* No permission set, meaningless */ ||
        start >= MAXVA
    /* Mapping range should be a legal address */
    {
        return -1;
    }

    // Check the range [start, start + len)
    let start_vpn = VirtAddr::from(start).floor();
    let end_vpn = VirtAddr::from(start + len).ceil();
    let vpns = VPNRange::new(start_vpn, end_vpn);
    for vpn in vpns {
        if let Some(pte) = get_current_task_page_table(vpn) {
            // We find a pte that has been mapped
            if pte.is_valid() {
                return -1;
            }
        }
    }

    // All `ptes` in range has pass the test
    create_new_map_area(
        start_vpn.into(),
        end_vpn.into(),
        MapPermission::from_bits_truncate((port << 1) as u8) | MapPermission::U,
    );
    0
}

/// Implement in [CH5]
/// `Munmap` the mapped virtual address
pub fn sys_munmap(start: usize, len: usize) -> isize {
    trace!(
        "kernel:pid[{}] sys_munmap NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    if start >= MAXVA || start & PAGE_SIZE != 0 {
        return -1;
    }
    // Avoid undefined situation
    let mut mlen = len;
    if start > MAXVA - len {
        mlen = MAXVA - start;
    }
    unmap_consecutive_area(start, mlen)
}

/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel:pid[{}] sys_sbrk", current_task().unwrap().pid.0);
    if let Some(old_brk) = current_task().unwrap().change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}

/// Implement in [CH5], re implement in [CH6]
/// `syscall ID`: 400
/// **Function:** Create a new child process and execute the target program.
/// **Description:** Returns the child process ID upon success, or -1 otherwise.
/// **Possible Errors:**
/// * Invalid file name.
/// * Process pool full/insufficient memory/resources error.
pub fn sys_spawn(path: *const u8) -> isize {
    let task = current_task().unwrap();
    let mut parent_inner = task.inner_exclusive_access();
    let token = parent_inner.memory_set.token();
    let path = translated_str(token, path);
    if let Some(app_inode) = open_file(path.as_str(), OpenFlags::RDONLY) {
        let all_data = app_inode.read_all();
        let (memory_set, user_sp, entry_point) = MemorySet::from_elf(all_data.as_slice());
        let trap_cx_ppn = memory_set
            .translate(VirtAddr::from(TRAP_CONTEXT_BASE).into())
            .unwrap()
            .ppn();

        // Alloc a pid and a kernel stack in kernel space
        let pid_handle = pid_alloc();
        let kernel_stack = kstack_alloc();
        let kernel_stack_top = kernel_stack.get_top();
        let mut new_fd_table: Vec<Option<Arc<dyn File + Send + Sync>>> = Vec::new();
        for fd in parent_inner.fd_table.iter() {
            if let Some(file) = fd {
                new_fd_table.push(Some(file.clone()));
            } else {
                new_fd_table.push(None);
            }
        }
        let task_control_block = Arc::new(TaskControlBlock {
            pid: pid_handle,
            kernel_stack,
            inner: unsafe {
                UPSafeCell::new(TaskControlBlockInner {
                    trap_cx_ppn,
                    base_size: parent_inner.base_size,
                    task_cx: TaskContext::goto_trap_return(kernel_stack_top),
                    task_status: TaskStatus::Ready,
                    memory_set,
                    parent: Some(Arc::downgrade(&task)),
                    children: Vec::new(),
                    exit_code: 0,
                    fd_table: new_fd_table,
                    heap_bottom: parent_inner.heap_bottom,
                    program_brk: parent_inner.program_brk,
                    syscall_times: [0; MAX_SYSCALL_NUM],
                    user_time: 0,
                    kernel_time: 0,
                    checkpoint: get_time_ms(),
                    stride: 0,
                    priority: 16,
                })
            },
        });

        // Add child
        parent_inner.children.push(task_control_block.clone());
        // Prepare TrapContext in user space
        let trap_cx = task_control_block.inner_exclusive_access().get_trap_cx();
        *trap_cx = TrapContext::app_init_context(
            entry_point,
            user_sp,
            KERNEL_SPACE.exclusive_access().token(),
            kernel_stack_top,
            trap_handler as usize,
        );

        let pid = task_control_block.pid.0 as isize;
        add_task(task_control_block);
        pid
    } else {
        return -1;
    }
}

/// `syscall ID:` 140
/// Set the current process priority to `prio`
/// **Parameter**: `prio` is the process priority, must be `prio >= 2`
/// **Return value**: Returns `prio` if the input is valid; otherwise, `returns -1`.
/// Implement in [CH5]
pub fn sys_set_priority(prio: isize) -> isize {
    trace!(
        "kernel:pid[{}] sys_set_priority NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    if prio <= 1 {
        return -1;
    }
    let task = current_task().unwrap();
    task.inner.exclusive_access().set_priority(prio as u64);
    prio
}

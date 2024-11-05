//! Process management `syscalls`

use crate::{
    config::{MAXVA, MAX_SYSCALL_NUM, PAGE_SIZE, TRAP_CONTEXT_BASE},
    loader::get_app_data_by_name,
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
use alloc::sync::Arc;
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
    /// The numbers of `syscall` called by task
    syscall_times: [u32; MAX_SYSCALL_NUM],
    /// Total running time of task
    time: usize,
}

// NOTE: 当应用调用`sys_exit`系统调用主动退出，
// 会在内核中调用`exit_current_and_run_next`函数
/// task exits and submit an exit code
pub fn sys_exit(exit_code: i32) -> ! {
    trace!("kernel:pid[{}] sys_exit", current_task().unwrap().pid.0);
    exit_current_and_run_next(exit_code);
    panic!("Unreachable in sys_exit!");
}

// NOTE: 使用task模块的suspend_current_and_run_next函数去，
// 暂停当前任务，并切换到下一个任务
/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel:pid[{}] sys_yield", current_task().unwrap().pid.0);
    suspend_current_and_run_next();
    0
}

pub fn sys_getpid() -> isize {
    trace!("kernel: sys_getpid pid:{}", current_task().unwrap().pid.0);
    current_task().unwrap().pid.0 as isize
}

// NOTE: 调用`sys_fork`之前，我们已经将当前进程Trap上下文的`sepc`向后移动了4字节，
// 使得它返回到用户态之后，从`ecall`下一条指令开始执行。。
// 之后，当我们复制地址空间时，子进程地址空间Trap上下文的`sepc`也是移动之后的值，
// 我们无需再进行修改
/// 功能：由当前进程 fork 出一个子进程。
/// 返回值：对于子进程返回 0，对于当前进程则返回子进程的 PID 。
/// `syscall ID`：220
pub fn sys_fork() -> isize {
    trace!("kernel:pid[{}] sys_fork", current_task().unwrap().pid.0);
    let current_task = current_task().unwrap();
    let new_task = current_task.fork();
    let new_pid = new_task.pid.0;
    // NOTE: 将子进程的Trap上下文中用来存放系统调用返回值的a0寄存器修改为0
    // 而父进程系统调用返回值会在`syscall`返回之后再设置为`sys_fork`的返回值。
    // 这样就做到，父进程`fork`返回值为子进程的pid，子进程返回0
    // modify trap context of new_task, because it returns immediately after switching
    let trap_cx = new_task.inner_exclusive_access().get_trap_cx();
    // We do not have to move to next instruction since we have done it before
    // for child process, fork returns 0
    trap_cx.x[10] = 0;
    // Add new task to scheduler
    add_task(new_task);
    new_pid as isize
}

/// 功能：将当前进程的地址空间清空并加载一个特定的可执行文件，返回用户态后开始它的执行。
/// 参数：字符串 path 给出了要加载的可执行文件的名字；
/// 返回值：如果出错的话（如找不到名字相符的可执行文件）则返回 -1，否则不应该返回。
/// 注意：path 必须以 "\0" 结尾，否则内核将无法确定其长度
/// `syscall ID`：221
pub fn sys_exec(path: *const u8) -> isize {
    trace!("kernel:pid[{}] sys_exec", current_task().unwrap().pid.0);
    let token = current_user_token();
    // NOTE: 调用`translated_str`找到要执行的应用名，
    let path = translated_str(token, path);
    // NOTE: 试图从应用加载器提供的`get_app_data_by_name`接口中获取对应的ELF数据
    // 如果找到，就调用`TaskControlBlock::exec`替换地址空间
    if let Some(data) = get_app_data_by_name(path.as_str()) {
        let task = current_task().unwrap();
        task.exec(data);
        0
    } else {
        -1
    }
}

// NOTE: 是一个立即返回的系统调用，它的返回值语义是：
// 如果当前的进程不存在一个符合要求的子进程，则返回-1；
// 如果至少存在一个，但是其中没有僵尸进程（也即仍未退出）则返回-2；
// 如果都不是的话，则可以正常返回并返回回收子进程的pid。
// 但在编写应用的开发者看来，`wait/waitpid`两个辅助函数都必须能够返回一个有意义的结果，
// 要么是-1，要么是一个正数PID，是不存在-2这种通过等待即可消除的中间结果的。
// 等待的过程由用户库`user_lib`完成。
/// 功能：当前进程等待一个子进程变为僵尸进程，回收其全部资源并收集其返回值。
/// 参数：pid 表示要等待的子进程的进程 ID，如果为 -1 的话表示等待任意一个子进程；
/// exit_code 表示保存子进程返回值的地址，如果这个地址为 0 的话表示不必保存。
/// 返回值：如果要等待的子进程不存在则返回 -1；否则如果要等待的子进程均未结束则返回 -2；
/// 否则返回结束的子进程的进程 ID。
/// `syscall ID`：260
/// If there is not a child process whose pid is same as given, return -1.
/// Else if there is a child process but it is still running, return -2.
pub fn sys_waitpid(pid: isize, exit_code_ptr: *mut i32) -> isize {
    trace!(
        "kernel::pid[{}] sys_waitpid [{}]",
        current_task().unwrap().pid.0,
        pid
    );
    let task = current_task().unwrap();
    // Find a child process

    // NOTE: 判断是否会返回-1，这取决于当前进程是否有一个符合要求的子进程。
    // 当传入的pid为-1的时候，任何一个子进程都算是符合要求；
    // 但pid不为-1的时候，则只有PID恰好与`pid`相同的子进程才符合条件
    // ---- access current PCB exclusively
    let mut inner = task.inner_exclusive_access();
    if !inner
        .children
        .iter()
        .any(|p| pid == -1 || pid as usize == p.getpid())
    {
        return -1;
        // ---- Release current PCB
    }
    let pair = inner.children.iter().enumerate().find(|(_, p)| {
        // ++++ temporarily access child PCB exclusively
        p.inner_exclusive_access().is_zombie() && (pid == -1 || pid as usize == p.getpid())
        // ++++ release child PCB
    });
    if let Some((idx, _)) = pair {
        let child = inner.children.remove(idx); // Confirm that child will be deallocated after being removed from children list assert_eq!(Arc::strong_count(&child), 1);
        let found_pid = child.getpid();
        // ++++ temporarily access child PCB exclusively
        let exit_code = child.inner_exclusive_access().exit_code;
        // ++++ release child PCB
        *translated_refmut(inner.memory_set.token(), exit_code_ptr) = exit_code;
        found_pid as isize
    } else {
        // NOTE: 找不到僵尸进程，返回-2
        -2
    }
    // ---- Release current PCB automatically
}

/// Implement in [CH3], re implement in [CH5]
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

/// Change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel:pid[{}] sys_sbrk", current_task().unwrap().pid.0);
    if let Some(old_brk) = current_task().unwrap().change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}

/// Implement in [CH5]
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
    if let Some(elf_data) = get_app_data_by_name(path.as_str()) {
        let (memory_set, user_sp, entry_point) = MemorySet::from_elf(elf_data);
        let trap_cx_ppn = memory_set
            .translate(VirtAddr::from(TRAP_CONTEXT_BASE).into())
            .unwrap()
            .ppn();
        // Alloc a pid and a kernel stack in kernel space
        let pid_handle = pid_alloc();
        let kernel_stack = kstack_alloc();
        let kernel_stack_top = kernel_stack.get_top();
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
pub fn sys_set_priority(prio: isize) -> isize {
    trace!(
        "kernel:pid[{}] sys_set_priority NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    // Must be `prio >= 2`
    if prio <= 1 {
        return -1;
    }
    let task = current_task().unwrap();
    task.inner.exclusive_access().set_priority(prio as u64);
    prio
}

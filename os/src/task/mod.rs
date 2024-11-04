//! Task management implementation
//!
//! Everything about task management, like starting and switching tasks is
//! implemented here.
//!
//! A single global instance of [`TaskManager`] called `TASK_MANAGER` controls
//! all the tasks in the whole operating system.
//!
//! A single global instance of [`Processor`] called `PROCESSOR` monitors running
//! task(s) for each core.
//!
//! A single global instance of `PID_ALLOCATOR` allocates pid for user apps.
//!
//! Be careful when you see `__switch` ASM function in `switch.S`. Control flow around this function
//! might not be what you expect.
mod context;
mod id;
mod manager;
mod processor;
mod switch;
#[allow(clippy::module_inception)]
mod task;

use crate::{
    loader::get_app_data_by_name,
    mm::{MapPermission, PageTableEntry, VPNRange, VirtAddr, VirtPageNum},
};
use alloc::sync::Arc;
use lazy_static::*;
pub use manager::{fetch_task, TaskManager};
use switch::__switch;
pub use task::{TaskControlBlock, TaskControlBlockInner, TaskStatus};

pub use context::TaskContext;
pub use id::{kstack_alloc, pid_alloc, KernelStack, PidHandle};
pub use manager::add_task;
pub use processor::{
    current_task, current_trap_cx, current_user_token, run_tasks, schedule, take_current_task,
    Processor,
};

use crate::config::MAX_SYSCALL_NUM;

// NOTE: 进行了修改
/// Add kernel cost time update logic for [CH5] use `update_checkpoint()`
/// Suspend the current 'Running' task and run the next task in task list.
pub fn suspend_current_and_run_next() {
    // NOTE: 首先通过`task_current_task`来取出当前执行的任务，
    // There must be an application running.
    let task = take_current_task().unwrap();

    // NOTE: 修改进程控制块内的状态
    // ---- access current TCB exclusively
    let mut task_inner = task.inner_exclusive_access();
    let task_cx_ptr = &mut task_inner.task_cx as *mut TaskContext;
    // Change status to Ready
    task_inner.task_status = TaskStatus::Ready;
    // Implement for [CH5]
    // Update kernel cost time
    task_inner.kernel_time += task_inner.update_checkpoint();

    drop(task_inner);
    // ---- release current PCB

    // NOTE: 将任务放入任务管理器的队尾
    // push back to ready queue.
    add_task(task);
    // NOTE: 触发调度并切换任务
    // jump to scheduling cycle
    schedule(task_cx_ptr);
}

/// pid of usertests app in make run TEST=1
pub const IDLE_PID: usize = 0;

/// Add kernel cost time update logic for [CH5] use `update_checkpoint()`
/// Exit the current 'Running' task and run the next task in task list.
pub fn exit_current_and_run_next(exit_code: i32) {
    // NOTE: 调用`take_current_task`来将当前进程控制块从处理器监控`PROCESSOR`中取出，
    // 而不只是得到一份拷贝，这是为了正确维护进程控制块的引用计数
    // take from Processor
    let task = take_current_task().unwrap();

    let pid = task.getpid();
    if pid == IDLE_PID {
        println!(
            "[kernel] Idle process exit with exit_code {} ...",
            exit_code
        );
        panic!("All applications completed!");
    }

    // **** access current TCB exclusively
    let mut inner = task.inner_exclusive_access();
    // NOTE: 将进程控制块中的状态修改为`TaskStatus::Zombie`即僵尸进程
    // Change status to Zombie
    inner.task_status = TaskStatus::Zombie;
    // NOTE: 将传入的退出码`exit_code`写入进程控制块中，后续父进程在`waitpid`的时候可以收集
    // Record exit code
    inner.exit_code = exit_code;
    // do not move to its parent but under initproc

    // ++++++ access initproc TCB exclusively
    {
        // NOTE: 将当前进程的所有子进程挂在初始进程`initproc`下面
        let mut initproc_inner = INITPROC.inner_exclusive_access();
        for child in inner.children.iter() {
            child.inner_exclusive_access().parent = Some(Arc::downgrade(&INITPROC));
            initproc_inner.children.push(child.clone());
        }
    }
    // ++++++ release parent PCB

    // NOTE: 当前进程的子向量清空
    inner.children.clear();
    // NOTE: 对于当前进程占用的资源进行早期回收。
    // `MemorySet::recycle_data_pages`只是将地址空间中的逻辑段列表`areas`清空，
    // 这会导致应用地址空间的所有数据被存放在的物理页帧被回收，
    // 而用来存放页表的那些物理页帧此时则不会被回收
    // deallocate user space
    inner.memory_set.recycle_data_pages();

    // Implement for [CH5]
    // Update kernel time cost
    inner.kernel_time += inner.update_checkpoint();

    drop(inner);
    // **** release current PCB
    // drop task manually to maintain rc correctly
    drop(task);
    // we do not have to save task context
    let mut _unused = TaskContext::zero_init();
    // NOTE:
    // 调用`schedule`出发调度及任务切换，由于再也不会回到个该进程的执行过程，
    // 因此需要关心任务的上下文切换
    schedule(&mut _unused as *mut _);
}

lazy_static! {
    // NOTE: 初始进程的进程控制块
    /// Creation of initial process
    ///
    /// the name "initproc" may be changed to any other app name like "usertests",
    /// but we have user_shell, so we don't need to change it.
    // NOTE: 调用`TaskControlBlock::new`来创建一个进程控制块，
    // 其需要传入ELF可执行文件的数据切片作为参数，
    // 这个参数需要通过加载器`loader`子模块提供的`get_app_data_by_name`接口查找`initproc`的ELF数据来获得
    pub static ref INITPROC: Arc<TaskControlBlock> = Arc::new(TaskControlBlock::new(
        get_app_data_by_name("ch5b_initproc").unwrap()
    ));
}

// NOTE: 初始化`INITPROC`之后，
// 则在这个函数中可以调用`task`的任务管理器`manager`子模块提供的`add_task`接口将其加入到任务管理器
///Add init process to the manager
pub fn add_initproc() {
    add_task(INITPROC.clone());
}

/// Implement in [CH5]
/// Count for time
///
pub fn user_time_start() {
    let task = current_task().unwrap();
    let mut task_inner = task.inner_exclusive_access();
    task_inner.kernel_time += task_inner.update_checkpoint();
}

/// Implement in [CH5]
/// Count for time
pub fn user_time_end() {
    let task = current_task().unwrap();
    let mut task_inner = task.inner_exclusive_access();
    task_inner.user_time += task_inner.update_checkpoint();
}

/// Implement in [CH3], re implement in [CH5] and split as 3 part
/// TaskControlBlock in chapter4 contains `MemorySet` and other fields
/// which cannot derive 'Clone' and 'Copy' traits. Therefore, we need to
/// split the variables into separate parts
pub fn get_current_task_status() -> TaskStatus {
    let task = current_task().unwrap();
    let task_inner = task.inner_exclusive_access();
    task_inner.get_status()
}

/// Implement in [CH3], re implement in [CH5] and split as 3 part
pub fn get_current_task_syscall_times() -> [u32; MAX_SYSCALL_NUM] {
    let task = current_task().unwrap();
    let task_inner = task.inner_exclusive_access();
    task_inner.syscall_times
}

/// Implement in [CH3], re implement in [CH5], but change the function name
/// *Old function name*: `current_task_do_syscall()`
/// When the system is dispatched, you'll need to call this function every time.
pub fn update_current_task_times(syscall_id: usize) {
    let task = current_task().unwrap();
    let mut task_inner = task.inner_exclusive_access();
    task_inner.syscall_times[syscall_id] += 1;
}

/// Implement in [CH5]
/// Count task time, which is kernel time + user time
pub fn get_current_task_time_cost() -> usize {
    let task = current_task().unwrap();
    let task_inner = task.inner_exclusive_access();
    task_inner.user_time + task_inner.kernel_time
}

/// Implement in [CH5]
/// Get PageTableEntry by a vpn
pub fn get_current_task_page_table(vpn: VirtPageNum) -> Option<PageTableEntry> {
    let task = current_task().unwrap();
    let task_inner = task.inner_exclusive_access();
    task_inner.memory_set.translate(vpn)
}

/// Implement in [CH5]
pub fn create_new_map_area(start_va: VirtAddr, end_va: VirtAddr, perm: MapPermission) {
    let task = current_task().unwrap();
    let mut task_inner = task.inner_exclusive_access();
    task_inner
        .memory_set
        .insert_framed_area(start_va, end_va, perm);
}

/// Implement in [CH5]
pub fn unmap_consecutive_area(start: usize, len: usize) -> isize {
    let task = current_task().unwrap();
    let mut task_inner = task.inner_exclusive_access();
    let start_vpn = VirtAddr::from(start).floor();
    let end_vpn = VirtAddr::from(start + len).ceil();
    let vpns = VPNRange::new(start_vpn, end_vpn);
    for vpn in vpns {
        if let Some(pte) = task_inner.memory_set.translate(vpn) {
            if !pte.is_valid() {
                return -1;
            }
            task_inner.memory_set.get_page_table().unmap(vpn);
        } else {
            // Also unmapped if no PTE found
            return -1;
        }
    }
    0
}

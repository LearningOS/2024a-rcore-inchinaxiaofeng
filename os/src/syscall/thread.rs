use crate::{
    mm::kernel_token,
    task::{add_task, current_task, TaskControlBlock},
    trap::{trap_handler, TrapContext},
};
use alloc::sync::Arc;

/// Thread create `syscall`
pub fn sys_thread_create(entry: usize, arg: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_thread_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    // 找到当前正在执行的线程`task`和此线程所属的进程`process`
    let task = current_task().unwrap();
    let process = task.process.upgrade().unwrap();
    // Create a new thread
    // 调用`TaskControlBlock::new`方法，创建`new_task`，在创建过程中，
    // 建立与`process`所属的关系，分配了线程用户态栈、内核态栈、用于异常/中断的跳板页
    let new_task = Arc::new(TaskControlBlock::new(
        Arc::clone(&process),
        task.inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .ustack_base,
        true,
    ));
    // Add new task to scheduler
    // 把线程挂到调度队列中
    add_task(Arc::clone(&new_task));
    let new_task_inner = new_task.inner_exclusive_access();
    let new_task_res = new_task_inner.res.as_ref().unwrap();
    let new_task_tid = new_task_res.tid;
    let mut process_inner = process.inner_exclusive_access();
    // Add new thread to current process
    // 把线程接入到所属进程的线程列表`tasks`中
    let tasks = &mut process_inner.tasks;
    while tasks.len() < new_task_tid + 1 {
        tasks.push(None);
    }
    tasks[new_task_tid] = Some(Arc::clone(&new_task));
    let new_task_trap_cx = new_task_inner.get_trap_cx();
    // 初始化位于该线程在用户态地址空间中的Trap上下文：设置线程的函数入口点和用户栈，
    // 使得第一次进入用户态时能从线程起始位置开始正确执行；
    // 设置好内核栈和陷入函数指针`trap_handler`，保证在Trap的时候用户态的线程能正确进入内核态。
    *new_task_trap_cx = TrapContext::app_init_context(
        entry,
        new_task_res.ustack_top(),
        kernel_token(),
        new_task.kstack.get_top(),
        trap_handler as usize,
    );
    (*new_task_trap_cx).x[10] = arg;
    new_task_tid as isize
}

/// Get current thread id `syscall`
pub fn sys_gettid() -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_gettid",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    current_task()
        .unwrap()
        .inner_exclusive_access()
        .res
        .as_ref()
        .unwrap()
        .tid as isize
}

/// Wait for a thread to exit `syscall`
///
/// Thread does not exist, return -1
/// thread has not exited yet, return -2
/// otherwise, return thread's exit code
pub fn sys_waittid(tid: usize) -> i32 {
    trace!(
        "kernel:pid[{}] tid[{}] sys_waittid",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let task = current_task().unwrap();
    let process = task.process.upgrade().unwrap();
    let task_inner = task.inner_exclusive_access();
    let mut process_inner = process.inner_exclusive_access();
    // A thread cannot wait for itself
    // 如果是线程等待自己，返回错误
    if task_inner.res.as_ref().unwrap().tid == tid {
        return -1;
    }
    // 如果找到`tid`对应的退出线程，则收集该退出线程的退出码`exit_tid`，否则返回错误（退出线程不存在）。
    let mut exit_code: Option<i32> = None;
    let waited_task = process_inner.tasks[tid].as_ref();
    if let Some(waited_task) = waited_task {
        if let Some(waited_exit_code) = waited_task.inner_exclusive_access().exit_code {
            exit_code = Some(waited_exit_code);
        }
    } else {
        // Waited thread does not exist
        return -1;
    }
    // 如果退出码存在，则清空进程中对应此退出线程的线程控制块（至此，线程所占用资源算是全部清空了）
    // 否则返回错误（线程还没退出）
    if let Some(exit_code) = exit_code {
        // `dealloc` the exited thread
        process_inner.tasks[tid] = None;
        exit_code
    } else {
        // Waited thread has not exited
        -2
    }
}

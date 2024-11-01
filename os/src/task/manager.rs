//!Implementation of [`TaskManager`]
use super::TaskControlBlock;
use crate::sync::UPSafeCell;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use lazy_static::*;
///A array of `TaskControlBlock` that is thread-safe
pub struct TaskManager {
    // NOTE: 将所有的任务控制块用引用计数`Arc`智能指针包裹后放在一个双端队列`VecDeque`中
    // 使用智能指针的原因在于，
    // 1. 任务控制块经常需要被放入/取出，
    //  如果直接移动任务控制块自身将会带来大量的数据拷贝开销，
    //  而对于智能指针进行移动则没有多少开销。
    // 2. 允许任务控制块的共享引用在某些情况下能够让我们的实现更加方便。
    ready_queue: VecDeque<Arc<TaskControlBlock>>,
}

// NOTE: 在这里，add和fetch组合形成了最简单的RR算法
/// A simple FIFO scheduler.
impl TaskManager {
    ///Creat an empty TaskManager
    pub fn new() -> Self {
        Self {
            ready_queue: VecDeque::new(),
        }
    }
    // NOTE: 将一个任务加入队尾
    /// Add process back to ready queue
    pub fn add(&mut self, task: Arc<TaskControlBlock>) {
        self.ready_queue.push_back(task);
    }
    // NOTE: 从队头中取出一个任务来执行
    /// Take a process out of the ready queue
    pub fn fetch(&mut self) -> Option<Arc<TaskControlBlock>> {
        self.ready_queue.pop_front()
    }
}

lazy_static! {
    /// TASK_MANAGER instance through lazy_static!
    pub static ref TASK_MANAGER: UPSafeCell<TaskManager> =
        unsafe { UPSafeCell::new(TaskManager::new()) };
}

// NOTE: 给内核其他的子模块提供的函数
/// Add process to ready queue
pub fn add_task(task: Arc<TaskControlBlock>) {
    //trace!("kernel: TaskManager::add_task");
    TASK_MANAGER.exclusive_access().add(task);
}

// NOTE: 给内核其他的子模块提供的函数
/// Take a process out of the ready queue
pub fn fetch_task() -> Option<Arc<TaskControlBlock>> {
    //trace!("kernel: TaskManager::fetch_task");
    TASK_MANAGER.exclusive_access().fetch()
}

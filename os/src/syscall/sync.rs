use core::usize;

use crate::config::TOTAL_AVAILABLE;
use crate::sync::{Condvar, Mutex, MutexBlocking, MutexSpin, Semaphore};
use crate::task::{block_current_and_run_next, current_process, current_task, ProcessControlBlock};
use crate::timer::{add_timer, get_time_ms};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

/// Sleep `syscall`
pub fn sys_sleep(ms: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_sleep",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let expire_ms = get_time_ms() + ms;
    let task = current_task().unwrap();
    add_timer(expire_ms, task);
    block_current_and_run_next();
    0
}
/// Mutex create `syscall`
pub fn sys_mutex_create(blocking: bool) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mutex: Option<Arc<dyn Mutex>> = if !blocking {
        Some(Arc::new(MutexSpin::new()))
    } else {
        // 如果向量中有空的元素，就在这个空元素的位置创建一个可睡眠的互斥锁；
        Some(Arc::new(MutexBlocking::new()))
    };
    let mut process_inner = process.inner_exclusive_access();
    if let Some(id) = process_inner
        .mutex_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.mutex_list[id] = mutex;
        id as isize
    } else {
        // 如果向量满了，就在向量中添加新的可睡眠的互斥锁；
        process_inner.mutex_list.push(mutex);
        process_inner.mutex_list.len() as isize - 1
    }
}
/// Mutex lock `syscall`
pub fn sys_mutex_lock(mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_lock",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());

    // Deadlock detection
    if process_inner.deadlock_detection_enabled {
        // Implement deadlock detection check here
        if deadlock_detected(&process) {
            return -0xDEAD; // Deadlock detected
        }
    }
    drop(process_inner);
    drop(process);
    // 调用ID为`mutex_id`的互斥锁`mutex`的`lock`方法
    mutex.lock();
    0
}
/// Mutex unlock `syscall`
pub fn sys_mutex_unlock(mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_unlock",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    drop(process_inner);
    drop(process);
    // 调用ID为`mutex_id`的互斥锁`mutex`的`unlock`方法
    mutex.unlock();
    0
}
/// Semaphore create `syscall`
pub fn sys_semaphore_create(res_count: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let id = if let Some(id) = process_inner
        .semaphore_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.semaphore_list[id] = Some(Arc::new(Semaphore::new(res_count)));
        id
    } else {
        process_inner
            .semaphore_list
            .push(Some(Arc::new(Semaphore::new(res_count))));
        process_inner.semaphore_list.len() - 1
    };
    id as isize
}
/// Semaphore up `syscall`
pub fn sys_semaphore_up(sem_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_up",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let sem = Arc::clone(process_inner.semaphore_list[sem_id].as_ref().unwrap());
    drop(process_inner);
    sem.up();
    0
}
/// Semaphore down `syscall`
pub fn sys_semaphore_down(sem_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_down",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let sem = Arc::clone(process_inner.semaphore_list[sem_id].as_ref().unwrap());

    // Deadlock detection
    if process_inner.deadlock_detection_enabled {
        // Implement deadlock detection check here
        if deadlock_detected(&process) {
            return -0xDEAD; // Deadlock detected
        }
    }
    drop(process_inner);
    sem.down();
    0
}
/// Condvar create `syscall`
pub fn sys_condvar_create() -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let id = if let Some(id) = process_inner
        .condvar_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.condvar_list[id] = Some(Arc::new(Condvar::new()));
        id
    } else {
        process_inner
            .condvar_list
            .push(Some(Arc::new(Condvar::new())));
        process_inner.condvar_list.len() - 1
    };
    id as isize
}
/// Condvar signal `syscall`
pub fn sys_condvar_signal(condvar_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_signal",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let condvar = Arc::clone(process_inner.condvar_list[condvar_id].as_ref().unwrap());
    drop(process_inner);
    condvar.signal();
    0
}
/// Condvar wait `syscall`
pub fn sys_condvar_wait(condvar_id: usize, mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_wait",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let condvar = Arc::clone(process_inner.condvar_list[condvar_id].as_ref().unwrap());
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    drop(process_inner);
    condvar.wait(mutex);
    0
}
/// Enable deadlock detection `syscall`
///
/// YOUR JOB: Implement deadlock detection, but might not all in this syscall
/// Implement in [CH8]
pub fn sys_enable_deadlock_detect(enabled: usize) -> isize {
    trace!("kernel: sys_enable_deadlock_detect NOT IMPLEMENTED");

    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();

    // Check for valid input
    if enabled != 0 && enabled != 1 {
        return -1; // Invalid parameter
    }

    // Set the deadlock detection state
    process_inner.deadlock_detection_enabled = enabled == 1;
    0
}

/// Define Resource management structures
pub struct ResourceManager {
    allocation: Vec<Vec<usize>>,
    max: Vec<Vec<usize>>,
    available: Vec<usize>,
    _num_processes: usize,
    _num_resources: usize,
}

impl ResourceManager {
    pub fn new(num_processes: usize, num_resources: usize) -> Self {
        Self {
            allocation: vec![vec![0; num_resources]; num_processes],
            max: vec![vec![0; num_resources]; num_processes],
            available: vec![0; num_resources],
            _num_processes: num_processes,
            _num_resources: num_resources,
        }
    }

    pub fn update_allocation(&mut self, process_id: usize, resources: Vec<usize>) {
        self.allocation[process_id] = resources;
    }

    pub fn update_max(&mut self, process_id: usize, max_resource: Vec<usize>) {
        self.max[process_id] = max_resource;
    }

    pub fn set_available(&mut self, available: Vec<usize>) {
        self.available = available;
    }
}

/// Implement in [CH8]
fn deadlock_detected(process: &Arc<ProcessControlBlock>) -> bool {
    let process_inner = process.inner_exclusive_access();
    let num_processes = process_inner.num_processes;
    let num_resources = process_inner.num_resources;

    let mut resource_manager = ResourceManager::new(num_processes, num_resources);

    // Populate resource_manager with current allocation and max
    for i in 0..num_processes {
        resource_manager.update_allocation(i, process_inner.allocation[i].clone());
        resource_manager.update_max(i, process_inner.max[i].clone());
    }

    // Calculate the Need matrix
    let need: Vec<Vec<usize>> = resource_manager
        .allocation
        .iter()
        .zip(resource_manager.max.iter())
        .map(|(alloc, max)| max.iter().zip(alloc.iter()).map(|(m, a)| m - a).collect())
        .collect();

    // Update available resources based on current allocations
    let mut available = vec![0; num_resources];
    for j in 0..num_resources {
        available[j] = TOTAL_AVAILABLE[j];
        for i in 0..num_processes {
            available[j] -= resource_manager.allocation[i][j];
        }
    }
    resource_manager.set_available(available);

    // Work array represents the resources available to complete processes
    let mut work = resource_manager.available.clone();
    let mut finish = vec![false; num_processes];

    loop {
        let mut made_progress = false;
        for i in 0..num_processes {
            if !finish[i] && need[i].iter().zip(work.iter()).all(|(n, w)| n <= w) {
                // Process i can finish
                for j in 0..num_resources {
                    work[j] += resource_manager.allocation[i][j];
                }
                finish[i] = true; // Mark process as finished
                made_progress = true;
            }
        }
        // If no process is made, we're in a deadlock
        if !made_progress {
            return true; // Deadlock detected
        }
        // Check if all processes are finished
        if finish.iter().all(|&f| f) {
            return false; // No deadlock
        }
    }
}

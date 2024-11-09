use core::usize;

use crate::sync::{Condvar, Mutex, MutexBlocking, MutexSpin, Semaphore};
use crate::task::{block_current_and_run_next, current_process, current_task};
use crate::timer::{add_timer, get_time_ms};
use alloc::sync::Arc;
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
    let tid = current_task()
        .unwrap()
        .inner_exclusive_access()
        .res
        .as_ref()
        .unwrap()
        .tid;
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
    let id = if let Some(id) = process_inner
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
    };
    drop(process_inner);
    drop(process);
    expand(0);
    current_process().inner_exclusive_access().available[0][id as usize] = 1;
    current_process().inner_exclusive_access().allocation[0][tid][id as usize] = 0;
    id
}
/// Mutex lock `syscall`
pub fn sys_mutex_lock(mutex_id: usize) -> isize {
    let tid = current_task()
        .unwrap()
        .inner_exclusive_access()
        .res
        .as_ref()
        .unwrap()
        .tid;

    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_lock",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        tid
    );
    expand(0);
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    drop(process_inner);
    drop(process);
    match deadlock_detected(0, mutex_id, tid) {
        false => -0xDEAD,
        true => {
            mutex.lock(tid, mutex_id);
            0
        }
    }
}
/// Mutex unlock `syscall`
pub fn sys_mutex_unlock(mutex_id: usize) -> isize {
    let tid = current_task()
        .unwrap()
        .inner_exclusive_access()
        .res
        .as_ref()
        .unwrap()
        .tid;
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_unlock",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        tid
    );
    expand(0);
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    process_inner.allocation[0][tid][mutex_id] -= 1;
    process_inner.available[0][mutex_id] += 1;
    drop(process_inner);
    drop(process);
    // 调用ID为`mutex_id`的互斥锁`mutex`的`unlock`方法
    mutex.unlock();
    0
}
/// Semaphore create `syscall`
pub fn sys_semaphore_create(res_count: usize) -> isize {
    let tid = current_task()
        .unwrap()
        .inner_exclusive_access()
        .res
        .as_ref()
        .unwrap()
        .tid;
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        tid
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
    drop(process_inner);
    drop(process);
    expand(1);
    current_process().inner_exclusive_access().available[1][id] = res_count as isize;
    current_process().inner_exclusive_access().allocation[1][tid][id] = 0;
    id as isize
}
/// Semaphore up `syscall`
pub fn sys_semaphore_up(sem_id: usize) -> isize {
    let tid = current_task()
        .unwrap()
        .inner_exclusive_access()
        .res
        .as_ref()
        .unwrap()
        .tid;
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_up",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let sem = Arc::clone(process_inner.semaphore_list[sem_id].as_ref().unwrap());
    process_inner.allocation[1][tid][sem_id] -= 1;
    process_inner.available[1][sem_id] += 1;
    drop(process_inner);
    sem.up();
    0
}
/// Semaphore down `syscall`
pub fn sys_semaphore_down(sem_id: usize) -> isize {
    let tid = current_task()
        .unwrap()
        .inner_exclusive_access()
        .res
        .as_ref()
        .unwrap()
        .tid;
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_down",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        tid
    );
    expand(1);
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let sem = Arc::clone(process_inner.semaphore_list[sem_id].as_ref().unwrap());
    drop(process_inner);
    match deadlock_detected(1, sem_id, tid) {
        false => -0xdead,
        true => {
            sem.down(tid, sem_id);
            0
        }
    }
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
    match enabled {
        //off
        0 => {
            current_process()
                .inner_exclusive_access()
                .deadlock_detection_enabled = false;
            0
        }
        //on
        1 => {
            current_process()
                .inner_exclusive_access()
                .deadlock_detection_enabled = true;
            0
        }
        _ => -1,
    }
}

/// Implement in [CH8]
fn deadlock_detected(i: usize, id: usize, tid: usize) -> bool {
    let process = current_process();
    let p_inner = process.inner_exclusive_access();
    if p_inner.deadlock_detection_enabled == false {
        return true;
    }
    let mut available = p_inner.available[i].clone();
    let allocation = p_inner.allocation[i].clone();
    let mut need = p_inner.need[i].clone();
    drop(p_inner);
    drop(process);
    need[tid][id] += 1;
    let mut used = Vec::new();
    while used.len() < allocation.len() {
        used.push(false);
    }
    let mut flag = true;
    while flag {
        flag = false;
        for i in 0..need.len() {
            if used[i] {
                continue;
            }
            let mut flag1 = true;
            for j in 0..need[i].len() {
                if available[j] - need[i][j] < 0 {
                    flag1 = false;
                }
            }
            if flag1 {
                for j in 0..allocation[i].len() {
                    available[j] += allocation[i][j];
                }
                flag = true;
                used[i] = true;
            }
        }
    }
    for i in used {
        if i == false {
            return false;
        }
    }
    true
}

///
fn expand(i: usize) {
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let id = match i {
        0 => process_inner.mutex_list.len(),
        _ => process_inner.semaphore_list.len(),
    };
    let tid = process_inner.tasks.len();
    while process_inner.available[i].len() <= id {
        process_inner.available[i].push(0);
    }
    while process_inner.allocation[i].len() <= tid {
        process_inner.allocation[i].push(Vec::new());
        process_inner.need[i].push(Vec::new());
    }
    process_inner.allocation[i].iter_mut().for_each(|i| {
        while i.len() <= id {
            i.push(0);
        }
    });
    process_inner.need[i].iter_mut().for_each(|i| {
        while i.len() <= id {
            i.push(0);
        }
    });
}

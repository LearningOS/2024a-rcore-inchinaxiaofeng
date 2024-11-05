//! Task pid implementation.
//!
//! Assign PID to the process here. At the same time, the position of the application KernelStack
//! is determined according to the PID.

use crate::config::{KERNEL_STACK_SIZE, PAGE_SIZE, TRAMPOLINE};
use crate::mm::{MapPermission, VirtAddr, KERNEL_SPACE};
use crate::sync::UPSafeCell;
use alloc::vec::Vec;
use lazy_static::*;

// NOTE: 类似于之前的页帧分配器`FrameAllocator`，我们同样实现一个简单栈式分配策略的
// 进程标识符分器`RecycleAllocator`，并全局化为`PID_ALLOCATOR`
pub struct RecycleAllocator {
    current: usize,
    recycled: Vec<usize>,
}

impl RecycleAllocator {
    pub fn new() -> Self {
        RecycleAllocator {
            current: 0,
            recycled: Vec::new(),
        }
    }
    // NOTE: 将会分配出一个将`usize`包装后的PidHandle
    // 我们将其包装为一个全局分配进程标识符的接口
    pub fn alloc(&mut self) -> usize {
        if let Some(id) = self.recycled.pop() {
            id
        } else {
            self.current += 1;
            self.current - 1
        }
    }
    pub fn dealloc(&mut self, id: usize) {
        assert!(id < self.current);
        assert!(
            !self.recycled.iter().any(|i| *i == id),
            "id {} has been deallocated!",
            id
        );
        self.recycled.push(id);
    }
}

lazy_static! {
    static ref PID_ALLOCATOR: UPSafeCell<RecycleAllocator> =
        unsafe { UPSafeCell::new(RecycleAllocator::new()) };
    static ref KSTACK_ALLOCATOR: UPSafeCell<RecycleAllocator> =
        unsafe { UPSafeCell::new(RecycleAllocator::new()) };
}

// NOTE: 同一时间存在的所有进程都有一个自己的进程标识符，他们是互不相同的整数。
// 这里抽象为一个`PidHanlde`类型，其生命周期结束后，对应的整数会被编译器自动回收
/// Abstract structure of `PID`
pub struct PidHandle(pub usize);

// NOTE: 实现Drop特征以允许编译器进行自动资源回收
impl Drop for PidHandle {
    fn drop(&mut self) {
        PID_ALLOCATOR.exclusive_access().dealloc(self.0);
    }
}

// NOTE: 被封装作全局的分配`PID`的接口
/// Allocate a new `PID`
pub fn pid_alloc() -> PidHandle {
    PidHandle(PID_ALLOCATOR.exclusive_access().alloc())
}

/// Return (bottom, top) of a kernel stack in kernel space.
pub fn kernel_stack_position(app_id: usize) -> (usize, usize) {
    let top = TRAMPOLINE - app_id * (KERNEL_STACK_SIZE + PAGE_SIZE);
    let bottom = top - KERNEL_STACK_SIZE;
    (bottom, top)
}

// NOTE: 在这里保存它所需进程的`PID`
/// Kernel stack for a process(task)
pub struct KernelStack(pub usize);

/// Allocate a new kernel stack
pub fn kstack_alloc() -> KernelStack {
    let kstack_id = KSTACK_ALLOCATOR.exclusive_access().alloc();
    let (kstack_bottom, kstack_top) = kernel_stack_position(kstack_id);
    KERNEL_SPACE.exclusive_access().insert_framed_area(
        kstack_bottom.into(),
        kstack_top.into(),
        MapPermission::R | MapPermission::W,
    );
    KernelStack(kstack_id)
}

// NOTE: 为`KernelStack`实现`Drop`Trait，一旦其生命周期结束，
// 就将内核地址空间中对应的逻辑段删除，
// 为此在`MemorySet`中新增了一个名为`remove_area_with_start_vpn`的方法
impl Drop for KernelStack {
    fn drop(&mut self) {
        let (kernel_stack_bottom, _) = kernel_stack_position(self.0);
        let kernel_stack_bottom_va: VirtAddr = kernel_stack_bottom.into();
        KERNEL_SPACE
            .exclusive_access()
            .remove_area_with_start_vpn(kernel_stack_bottom_va.into());
        KSTACK_ALLOCATOR.exclusive_access().dealloc(self.0);
    }
}

// NOTE: 内核栈`KernelStack`用到了`RAII`思想
// 实际保存它的物理页帧的生命周期被绑定到它下面，
// 当`KernelStack`生命周期结束后，这些物理页帧将被编译器自动回收
impl KernelStack {
    // NOTE: 方法可以将一个类型为T的变量压入内核栈顶并返回其裸指针，
    // 这也是一个泛型函数。
    /// Push a variable of type T into the top of the KernelStack and return its raw pointer
    #[allow(unused)]
    pub fn push_on_top<T>(&self, value: T) -> *mut T
    where
        T: Sized,
    {
        let kernel_stack_top = self.get_top();
        let ptr_mut = (kernel_stack_top - core::mem::size_of::<T>()) as *mut T;
        unsafe {
            *ptr_mut = value;
        }
        ptr_mut
    }
    // NOTE: 它在实现的时候用到了第32行的`get_top`方法来获取当前内核栈顶在内核地址空间中的地址
    /// Get the top of the KernelStack
    pub fn get_top(&self) -> usize {
        let (_, kernel_stack_top) = kernel_stack_position(self.0);
        kernel_stack_top
    }

    /// Implement in [CH5]
    /// This function is noted in book, but not implement in real project,
    /// so I re implement this function.
    pub fn new(pid_handle: &PidHandle) -> Self {
        let pid = pid_handle.0;
        let (kernel_stack_bottom, kernel_stack_top) = kernel_stack_position(pid);
        KERNEL_SPACE.exclusive_access().insert_framed_area(
            kernel_stack_bottom.into(),
            kernel_stack_top.into(),
            MapPermission::R | MapPermission::W,
        );
        KernelStack(pid_handle.0)
    }
}

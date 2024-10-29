//! Implementation of [`FrameAllocator`] which
//! controls all the frames in the operating system.

use super::{PhysAddr, PhysPageNum};
use crate::config::MEMORY_END;
use crate::sync::UPSafeCell;
use alloc::vec::Vec;
use core::fmt::{self, Debug, Formatter};
use lazy_static::*;

// NOTE: 从其他内核模块的视角看来，
// 物理页帧分配的接口是调用 frame_alloc 函数得到一个 FrameTracker （如果物理内存还有剩余），
// 它就代表了一个物理页帧，当它的生命周期结束之后它所控制的物理页帧将被自动回收。

/// tracker for physical page frame allocation and deallocation
pub struct FrameTracker {
    /// physical page number
    pub ppn: PhysPageNum,
}
// NOTE: 借用了 RAII 的思想，
// 将一个物理页帧的生命周期绑定到一个 FrameTracker 变量上，
// 当一个 FrameTracker 被创建的时候，我们需要从 FRAME_ALLOCATOR 中分配一个物理页帧
impl FrameTracker {
    /// Create a new FrameTracker
    pub fn new(ppn: PhysPageNum) -> Self {
        // page cleaning
        let bytes_array = ppn.get_bytes_array();
        for i in bytes_array {
            *i = 0;
        }
        Self { ppn }
    }
}

impl Debug for FrameTracker {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!("FrameTracker:PPN={:#x}", self.ppn.0))
    }
}
// NOTE: 当一个 FrameTracker 生命周期结束被编译器回收的时候，
// 我们需要将它控制的物理页帧回收到 FRAME_ALLOCATOR 中。
// 我们只需要为这个实现DropTrait就行了
impl Drop for FrameTracker {
    fn drop(&mut self) {
        frame_dealloc(self.ppn);
    }
}

// NOTE: 描述一个物理页帧管理器需要提供哪些功能
// 创建一个物理页帧管理器的实例，以及以物理页号为单位进行物理页帧的分配和回收
trait FrameAllocator {
    fn new() -> Self;
    fn alloc(&mut self) -> Option<PhysPageNum>;
    fn dealloc(&mut self, ppn: PhysPageNum);
}
// NOTE: 最简单的栈式物理页帧管理策略
/// an implementation for frame allocator
pub struct StackFrameAllocator {
    current: usize,       // NOTE: 空闲内存的起始物理页号
    end: usize,           // NOTE: 空闲内存的结束物理页号
    recycled: Vec<usize>, // NOTE: 以后入先出的方式保存了被回收的物理页号
}

impl StackFrameAllocator {
    // NOTE: frame实例真正被使用起来之前，
    // 需要调用 init 方法将自身的[current, end)初始化为可用物理页号区间
    pub fn init(&mut self, l: PhysPageNum, r: PhysPageNum) {
        self.current = l.0;
        self.end = r.0;
        // trace!("last {} Physical Frames.", self.end - self.current);
    }
}
impl FrameAllocator for StackFrameAllocator {
    // NOTE: 通过 FrameAllocator 的 new 方法创建实例的时候，
    // 只需将区间两端均设为0，然后创建一个新的向量
    fn new() -> Self {
        Self {
            current: 0,
            end: 0,
            recycled: Vec::new(),
        }
    }
    // NOTE: 核心的物理页帧分配
    fn alloc(&mut self) -> Option<PhysPageNum> {
        if let Some(ppn) = self.recycled.pop() {
            // NOTE: 检查栈 recycled 内有没有之前回收的物理页号，如果有的话直接弹出栈顶并返回
            Some(ppn.into())
        } else if self.current == self.end {
            // NOTE: 内存耗尽分配失败
            None
        } else {
            self.current += 1;
            Some((self.current - 1).into())
        }
    }
    // NOTE: 核心物理页帧回收
    // 我们需要检查回收页面的合法性，然后将其压入 recycled 栈中
    // 回收页面合法有两个条件：
    // 该页面之前一定被分配出去过，因此它的物理页号一定 < current ；
    // 该页面没有正处在回收状态，即它的物理页号不能在栈 recycled 中找到。
    fn dealloc(&mut self, ppn: PhysPageNum) {
        let ppn = ppn.0;
        // NOTE: 通过 recycled.iter() 获取栈上内容的迭代器
        // validity check
        if ppn >= self.current || self.recycled.iter().any(|&v| v == ppn) {
            panic!("Frame ppn={:#x} has not been allocated!", ppn);
        }
        // recycle
        self.recycled.push(ppn);
    }
}

type FrameAllocatorImpl = StackFrameAllocator;

// NOTE: 全局实例 FRAME_ALLOCATOR
lazy_static! {
    /// frame allocator instance through lazy_static!
    pub static ref FRAME_ALLOCATOR: UPSafeCell<FrameAllocatorImpl> =
        unsafe { UPSafeCell::new(FrameAllocatorImpl::new()) };
}
// NOTE: 在正式分配物理页帧之前，我们需要将物理页帧全局管理器 FRAME_ALLOCATOR 初始化
/// initiate the frame allocator using `ekernel` and `MEMORY_END`
pub fn init_frame_allocator() {
    extern "C" {
        fn ekernel();
    }
    FRAME_ALLOCATOR.exclusive_access().init(
        PhysAddr::from(ekernel as usize).ceil(),
        PhysAddr::from(MEMORY_END).floor(),
    );
}
// NOTE: 公开给其他内核模块调用的分配/回收物理页帧的接口
/// Allocate a physical page frame in FrameTracker style
pub fn frame_alloc() -> Option<FrameTracker> {
    FRAME_ALLOCATOR
        .exclusive_access()
        .alloc()
        .map(FrameTracker::new)
}

/// Deallocate a physical page frame with a given ppn
pub fn frame_dealloc(ppn: PhysPageNum) {
    FRAME_ALLOCATOR.exclusive_access().dealloc(ppn);
}

#[allow(unused)]
/// a simple test for frame allocator
pub fn frame_allocator_test() {
    let mut v: Vec<FrameTracker> = Vec::new();
    for i in 0..5 {
        let frame = frame_alloc().unwrap();
        println!("{:?}", frame);
        v.push(frame);
    }
    v.clear();
    for i in 0..5 {
        let frame = frame_alloc().unwrap();
        println!("{:?}", frame);
        v.push(frame);
    }
    drop(v);
    println!("frame_allocator_test passed!");
}

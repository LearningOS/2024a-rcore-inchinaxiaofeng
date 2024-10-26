//! The global allocator
use crate::config::KERNEL_HEAP_SIZE;
use buddy_system_allocator::LockedHeap;

// NOTE:
// 我们直接将`buddy_system_allocator`中提供的`LockedHeap`实例化成一个全局变量，并使用`alloc`要求的`#[global_allocator]`语义项进行标记
// 注意`LockedHeap`已经实现了`GlobalAlloc`要求的抽象接口。
#[global_allocator]
/// heap allocator instance
static HEAP_ALLOCATOR: LockedHeap = LockedHeap::empty();

// NOTE: 在main.rs中，也需要添加feature(alloc_error_handler)来表明存在动态内存分配失败的处理
// 直接Panic
#[alloc_error_handler]
/// panic when heap allocation error occurs
pub fn handle_alloc_error(layout: core::alloc::Layout) -> ! {
    panic!("Heap allocation error, layout = {:?}", layout);
}

// NOTE: 这块内存是一个`static mut`且被0初始化的字节数组，位于内核`.bss`段中。
/// heap space ([u8; KERNEL_HEAP_SIZE])
static mut HEAP_SPACE: [u8; KERNEL_HEAP_SIZE] = [0; KERNEL_HEAP_SIZE];

// NOTE:
// 在使用任何`alloc`中提供的堆数据结构之前，我们需要调用这个函数，来给我们的全局分配器一块内存用于分配
// LockedHeap 也是一个被互斥锁 Mutex<T> 保护的类型，在对它任何进行任何操作之前都要先获取锁以避免其他线程同时对它进行操作导致数据竞争。
// 然后，调用 init 方法告知它能够用来分配的空间的起始地址和大小即可。
/// initiate heap allocator
pub fn init_heap() {
    unsafe {
        HEAP_ALLOCATOR
            .lock()
            .init(HEAP_SPACE.as_ptr() as usize, KERNEL_HEAP_SIZE);
    }
}

// NOTE: 测试函数
#[allow(unused)]
pub fn heap_test() {
    use alloc::boxed::Box;
    use alloc::vec::Vec;
    extern "C" {
        fn sbss();
        fn ebss();
    }
    let bss_range = sbss as usize..ebss as usize;
    let a = Box::new(5);
    assert_eq!(*a, 5);
    assert!(bss_range.contains(&(a.as_ref() as *const _ as usize)));
    drop(a);
    let mut v: Vec<usize> = Vec::new();
    for i in 0..500 {
        v.push(i);
    }
    for (i, val) in v.iter().take(500).enumerate() {
        assert_eq!(*val, i);
    }
    assert!(bss_range.contains(&(v.as_ptr() as usize)));
    drop(v);
    println!("heap_test passed!");
}

use super::File;
use crate::mm::UserBuffer;
use crate::sync::UPSafeCell;
use alloc::sync::{Arc, Weak};

use crate::task::suspend_current_and_run_next;

/// `IPC` pipe
pub struct Pipe {
    /// 指出该管道端可否支持读取
    readable: bool,
    /// 指出该管道端可否支持写入
    writable: bool,
    /// 通过`buffer`字段可以找到该管道端所在的管道自身
    buffer: Arc<UPSafeCell<PipeRingBuffer>>,
}

impl Pipe {
    /// Create readable pipe
    /// 从一个已有的管道创建它的读端
    /// 读端和写端的访问权限进行了相应设置：不允许向读端写入，也不允许从写端读取
    pub fn read_end_with_buffer(buffer: Arc<UPSafeCell<PipeRingBuffer>>) -> Self {
        Self {
            readable: true,
            writable: false,
            buffer,
        }
    }
    /// Create writable pipe
    /// 从一个已有的管道创建它的写端
    /// 读端和写端的访问权限进行了相应设置：不允许向读端写入，也不允许从写端读取
    pub fn write_end_with_buffer(buffer: Arc<UPSafeCell<PipeRingBuffer>>) -> Self {
        Self {
            readable: false,
            writable: true,
            buffer,
        }
    }
}

const RING_BUFFER_SIZE: usize = 32;

/// 管道自身，也就是那个带有一定大小缓冲区的字节队列，我们抽象为`PipeRingBuffer`类型
#[derive(Copy, Clone, PartialEq)]
enum RingBufferStatus {
    /// 表示缓冲区已满不能再继续写入
    Full,
    /// 表示缓冲区为空无法从里面读取
    Empty,
    /// 表示除了`FULL`和`EMPTY`之外的其他状态
    Normal,
}

/// `arr/head/tail`三个字段用来维护一个循环队列
pub struct PipeRingBuffer {
    /// 存放数据的数组
    arr: [u8; RING_BUFFER_SIZE],
    /// 循环队列队头的下标
    head: usize,
    /// 循环队列队尾的下标
    tail: usize,
    /// 记录了缓冲区目前的状态
    status: RingBufferStatus,
    /// 保存了它的写端的一个弱引用计数，
    /// 这是由于在某些情况下需要确认该管道所有的写端是否都已经被关闭了，通过这个字段很容易确认这一点
    write_end: Option<Weak<Pipe>>,
}

impl PipeRingBuffer {
    /// 通过`new`可以创建一个新的管道
    pub fn new() -> Self {
        Self {
            arr: [0; RING_BUFFER_SIZE],
            head: 0,
            tail: 0,
            status: RingBufferStatus::Empty,
            write_end: None,
        }
    }
    /// 调用`PipeRingBuffer::set_write_end`在管道中保留它的写端的弱引用计数
    pub fn set_write_end(&mut self, write_end: &Arc<Pipe>) {
        self.write_end = Some(Arc::downgrade(write_end));
    }

    pub fn write_byte(&mut self, byte: u8) {
        self.status = RingBufferStatus::Normal;
        self.arr[self.tail] = byte;
        self.tail = (self.tail + 1) % RING_BUFFER_SIZE;
        if self.tail == self.head {
            self.status = RingBufferStatus::Full;
        }
    }

    /// 可以从管道中读取一个字节，注意在调用它之前需要确保管道缓冲区中不是空的
    /// 它会更新循环队列队头的位置，并比较队头和队尾是否相同，如果相同的话则说明管道的状态变为空`EMPTY`
    pub fn read_byte(&mut self) -> u8 {
        self.status = RingBufferStatus::Normal;
        let c = self.arr[self.head];
        self.head = (self.head + 1) % RING_BUFFER_SIZE;
        if self.head == self.tail {
            self.status = RingBufferStatus::Empty;
        }
        c
    }

    /// 计算管道中还有多少个字符可以读取
    /// 首先需要需要判断队列是否为空，因为队头和队尾相等可能表示队列为空或队列已满，两种情况`available_read`的返回值截然不同。
    /// 如果队列为空的话直接返回 0，否则根据队头和队尾的相对位置进行计算。
    pub fn available_read(&self) -> usize {
        if self.status == RingBufferStatus::Empty {
            0
        } else if self.tail > self.head {
            self.tail - self.head
        } else {
            self.tail + RING_BUFFER_SIZE - self.head
        }
    }

    /// 判断管道的所有写端是否都被关闭了，这是通过尝试将管道中保存的写端的弱引用计数升级为强引用计数来实现的。
    /// 如果升级失败的话，说明管道写端的强引用计数为 0 ，也就意味着管道所有写端都被关闭了，
    /// 从而管道中的数据不会再得到补充，待管道中仅剩的数据被读取完毕之后，管道就可以被销毁了。
    pub fn available_write(&self) -> usize {
        if self.status == RingBufferStatus::Full {
            0
        } else {
            RING_BUFFER_SIZE - self.available_read()
        }
    }
    pub fn all_write_ends_closed(&self) -> bool {
        self.write_end.as_ref().unwrap().upgrade().is_none()
    }
}

/// Return (read_end, write_end)
/// 创建一个管道并返回它的读端和写端
pub fn make_pipe() -> (Arc<Pipe>, Arc<Pipe>) {
    let buffer = Arc::new(unsafe { UPSafeCell::new(PipeRingBuffer::new()) });
    let read_end = Arc::new(Pipe::read_end_with_buffer(buffer.clone()));
    let write_end = Arc::new(Pipe::write_end_with_buffer(buffer.clone()));
    buffer.exclusive_access().set_write_end(&write_end);
    (read_end, write_end)
}

impl File for Pipe {
    fn readable(&self) -> bool {
        self.readable
    }
    fn writable(&self) -> bool {
        self.writable
    }
    fn read(&self, buf: UserBuffer) -> usize {
        assert!(self.readable());
        let want_to_read = buf.len();
        // `buf_iter`将传入的应用缓冲区`buf`转化为一个能够逐字节对于缓冲区进行访问的迭代器，
        // 每次调用`buf_iter.next()`即可按顺序取出用于访问缓冲区中一个字节的裸指针
        let mut buf_iter = buf.into_iter();
        // 维护实际有多少字节从管道读入应用的缓冲区
        let mut already_read = 0usize;
        // 当循环队列中不存在足够字符的时候暂时进行任务切换，等待循环队列中的字符得到补充之后再继续读取
        loop {
            let mut ring_buffer = self.buffer.exclusive_access();
            // 保存循环在这一轮次中可以从管道循环队列中读取多少字符。
            let loop_read = ring_buffer.available_read();
            // 如果管道为空，检查管道的所有写端是否已经被关闭
            if loop_read == 0 {
                // 是，返回；
                if ring_buffer.all_write_ends_closed() {
                    return already_read;
                }
                // 否则调用suspend_current_and_run_next切换到其他任务，等切换回来之后回到循环开头，再看一下有没有字符了
                drop(ring_buffer); // 手动释放管道自身的锁，因为切换任务的`__switch`不是一个正常的函数调用
                suspend_current_and_run_next();
                continue;
            }
            // loop_read不是0,说明有loop_read个字节可以读取，
            // 迭代应用缓冲区中的每个字节指针，调用`PipeRingBuffer::read_byte`方法来从管道中进行读取
            for _ in 0..loop_read {
                // 还没填满应用缓冲区，下一个轮次
                if let Some(byte_ref) = buf_iter.next() {
                    unsafe {
                        *byte_ref = ring_buffer.read_byte();
                    }
                    already_read += 1;
                    if already_read == want_to_read {
                        return want_to_read;
                    }
                } else {
                    // 满了，就直接返回
                    return already_read;
                }
            }
        }
    }
    fn write(&self, buf: UserBuffer) -> usize {
        assert!(self.writable());
        let want_to_write = buf.len();
        let mut buf_iter = buf.into_iter();
        let mut already_write = 0usize;
        loop {
            let mut ring_buffer = self.buffer.exclusive_access();
            let loop_write = ring_buffer.available_write();
            if loop_write == 0 {
                drop(ring_buffer);
                suspend_current_and_run_next();
                continue;
            }
            // Write at most loop_write bytes
            for _ in 0..loop_write {
                if let Some(byte_ref) = buf_iter.next() {
                    ring_buffer.write_byte(unsafe { *byte_ref });
                    already_write += 1;
                    if already_write == want_to_write {
                        return want_to_write;
                    }
                } else {
                    return already_write;
                }
            }
        }
    }
}

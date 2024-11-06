use super::{BlockDevice, BLOCK_SZ};
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use lazy_static::*;
use spin::Mutex;
/// Cached block inside memory
pub struct BlockCache {
    /// Cached block data
    /// 512字节的数组，表示位于内存中的缓冲区
    cache: [u8; BLOCK_SZ],
    /// Underlying block id
    /// 记录了这个块的编号
    block_id: usize,
    /// Underlying block device
    /// 记录块所属的底层设备
    block_device: Arc<dyn BlockDevice>,
    /// Whether the block is dirty
    /// 记录自从这个块缓存从磁盘载入内存之后，它有没有被修改过
    modified: bool,
}

impl BlockCache {
    /// Load a new BlockCache from disk.
    /// 创建时，将一个块从磁盘读取到缓冲区`cache`
    pub fn new(block_id: usize, block_device: Arc<dyn BlockDevice>) -> Self {
        let mut cache = [0u8; BLOCK_SZ];
        block_device.read_block(block_id, &mut cache);
        Self {
            cache,
            block_id,
            block_device,
            modified: false,
        }
    }
    /// Get the address of an offset inside the cached block data
    /// 可以得到一个`BlockCache`内部的缓冲区中指定偏移量`offset`的字节地址
    fn addr_of_offset(&self, offset: usize) -> usize {
        &self.cache[offset] as *const _ as usize
    }

    /// 泛型方法，可以获取缓冲区中的位于偏移量`offset`的一个类型为`T`的磁盘上数据结构的不可变引用。
    /// 该方法的`Trait Bound`限制类型`T`必须是一个编译时已知大小的类型，
    /// 我们通过`core::mem::size_of::<T>()`在编译时获取类型`T`的大小并确认该数据结构被整个包含在磁盘块及其缓冲区内。
    /// 这里编译器会自动进行生命周期标注，约束返回的引用的生命周期不超过`BlockCache`自身，在使用的时候我们会保证这一点
    pub fn get_ref<T>(&self, offset: usize) -> &T
    where
        T: Sized,
    {
        let type_size = core::mem::size_of::<T>();
        assert!(offset + type_size <= BLOCK_SZ);
        let addr = self.addr_of_offset(offset);
        unsafe { &*(addr as *const T) }
    }

    /// 它会获取磁盘上数据结构的可变引用，由此可以对数据结构进行修改。
    /// 由于这些数据结构目前位于内存中的缓冲区中，我们需要将`BlackCache`的`modified`标记为true表示该缓冲区已经被修改，
    /// 之后需要将数据写回磁盘块才能真正将修改同步到磁盘
    pub fn get_mut<T>(&mut self, offset: usize) -> &mut T
    where
        T: Sized,
    {
        let type_size = core::mem::size_of::<T>();
        assert!(offset + type_size <= BLOCK_SZ);
        self.modified = true;
        let addr = self.addr_of_offset(offset);
        unsafe { &mut *(addr as *mut T) }
    }

    /// 将`get_ref`进一步封装为更容易使用的形式
    /// 在`BlockCache`缓冲区偏移量为`offset`的位置，获取一个类型为`T`不可变引用，
    /// 将闭包`f`作用于这个引用，返回`f`的返回值中定义的操作
    pub fn read<T, V>(&self, offset: usize, f: impl FnOnce(&T) -> V) -> V {
        f(self.get_ref(offset))
    }

    /// 将`get_mut`进一步封装为更容易使用的形式
    /// 在`BlockCache`缓冲区偏移量为`offset`的位置，获取一个类型为`T`可变引用，
    /// 将闭包`f`作用于这个引用，返回`f`的返回值中定义的操作
    pub fn modify<T, V>(&mut self, offset: usize, f: impl FnOnce(&mut T) -> V) -> V {
        f(self.get_mut(offset))
    }

    /// `modified`决定是否需要写回磁盘
    pub fn sync(&mut self) {
        if self.modified {
            self.modified = false;
            self.block_device.write_block(self.block_id, &self.cache);
        }
    }
}

/// 当`BlockCache`的生命周期结束后，缓冲区也会被回收，`modified`会决定是否需要写回磁盘
impl Drop for BlockCache {
    fn drop(&mut self) {
        self.sync()
    }
}
/// Use a block cache of 16 blocks
const BLOCK_CACHE_SIZE: usize = 16;

/// 块缓存全局管理器
/// 内存只能同时缓存有限个磁盘块。
/// 当我们要对一个磁盘块进行读写时，块缓存全局管理器检查它是否已经被载入内存中，
/// 如果是则直接返回，否则就读取磁盘块到内存。
/// 如果内存中驻留的磁盘块缓冲区的数量已满，则需要进行缓存替换。
/// 这里使用一种类`FIFO`的缓存替换算法，在管理器中只需维护一个队列
pub struct BlockCacheManager {
    /// 维护块编号和块缓存的二元组
    /// 块缓存的类型是一个`Arc<Mutex<BlockCache>>`，这是 Rust 中的经典组合，它可以同时提供共享引用和互斥访问。
    /// 这里的共享引用意义在于块缓存既需要在管理器`BlockCacheManager`保留一个引用，还需要将引用返回给块缓存的请求者。
    /// 而互斥访问在单核上的意义在于提供内部可变性通过编译，在多核环境下则可以帮助我们避免可能的并发冲突。
    queue: VecDeque<(usize, Arc<Mutex<BlockCache>>)>,
}

impl BlockCacheManager {
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
        }
    }

    /// 尝试从块缓存管理器中获取一个编号为 block_id 的块缓存，如果找不到的话会读取磁盘，还有可能会发生缓存替换
    pub fn get_block_cache(
        &mut self,
        block_id: usize,
        block_device: Arc<dyn BlockDevice>,
    ) -> Arc<Mutex<BlockCache>> {
        // 遍历整个队列试图找到一个编号相同的块缓存，如果找到，将块缓存管理器中保存的块缓存的引用复制一份并返回
        if let Some(pair) = self.queue.iter().find(|pair| pair.0 == block_id) {
            Arc::clone(&pair.1)
        } else {
            // 此时必须将块从磁盘读入内存中的缓冲区。读取前需要判断已保存的块数量是否达到了上限。
            // 是，则执行缓存替换算法，替换的标准是其强引用计数=1 ，即除了块缓存管理器保留的一份副本之外，在外面没有副本正在使用。
            // Substitute
            if self.queue.len() == BLOCK_CACHE_SIZE {
                // From front to tail
                if let Some((idx, _)) = self
                    .queue
                    .iter()
                    .enumerate()
                    .find(|(_, pair)| Arc::strong_count(&pair.1) == 1)
                {
                    self.queue.drain(idx..=idx);
                } else {
                    panic!("Run out of BlockCache!");
                }
            }
            // Load block into mem and push back
            // 创建一个新的块缓存（会触发`read_block`进行块读取）并加入到队尾，最后返回给请求着。
            let block_cache = Arc::new(Mutex::new(BlockCache::new(
                block_id,
                Arc::clone(&block_device),
            )));
            self.queue.push_back((block_id, Arc::clone(&block_cache)));
            block_cache
        }
    }
}

lazy_static! {
    /// The global block cache manager
    pub static ref BLOCK_CACHE_MANAGER: Mutex<BlockCacheManager> =
        Mutex::new(BlockCacheManager::new());
}
/// Get the block cache corresponding to the given block id and block device
pub fn get_block_cache(
    block_id: usize,
    block_device: Arc<dyn BlockDevice>,
) -> Arc<Mutex<BlockCache>> {
    BLOCK_CACHE_MANAGER
        .lock()
        .get_block_cache(block_id, block_device)
}
/// Sync all block cache to block device
pub fn block_cache_sync_all() {
    let manager = BLOCK_CACHE_MANAGER.lock();
    for (_, cache) in manager.queue.iter() {
        cache.lock().sync();
    }
}

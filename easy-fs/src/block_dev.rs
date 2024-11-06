use core::any::Any;

/// Trait for block devices
/// which reads and writes data in the unit of blocks
/// 最底层申明的块设备抽象接口，使用者将负责提供抽象方法的实现
pub trait BlockDevice: Send + Sync + Any {
    ///Read data form block to buffer
    ///可以将编号为`block_id`的块从磁盘读入内存中的缓冲区`buf`
    fn read_block(&self, block_id: usize, buf: &mut [u8]);
    ///Write data from buffer to block
    ///可以将内存中的缓冲区`buf`中的数据写入磁盘编号为`block_id`的块
    fn write_block(&self, block_id: usize, buf: &[u8]);
}

use core::usize;

use super::{
    block_cache_sync_all, get_block_cache, BlockDevice, DirEntry, DiskInode, DiskInodeType,
    EasyFileSystem, DIRENT_SZ,
};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use spin::{Mutex, MutexGuard};

/// 放在内存中的记录文件索引节点信息的数据结构
/// Virtual `filesystem` layer over easy-fs
/// Change in [CH6], let block_id, block_offset, from private to public
pub struct Inode {
    /// 记录该 Inode 对应的 DiskInode 保存在磁盘上的具体位置方便我们后续对它进行访问
    pub block_id: usize,
    /// 记录该 Inode 对应的 DiskInode 保存在磁盘上的具体位置方便我们后续对它进行访问
    pub block_offset: usize,
    /// 指向 EasyFileSystem 的一个指针，因为对 Inode 的种种操作实际上都是要通过底层的文件系统来完成。
    fs: Arc<Mutex<EasyFileSystem>>,
    block_device: Arc<dyn BlockDevice>,
}

impl Inode {
    /// Create a `vfs inode`
    /// 在`root_inode`中，主要是在`Inode::new`的时候将传入的`inode_id`设置为 0 ，
    /// 因为根目录对应于文件系统中第一个分配的`inode`，因此它的`inode_id`总会是 0 。
    pub fn new(
        block_id: u32,
        block_offset: usize,
        fs: Arc<Mutex<EasyFileSystem>>,
        block_device: Arc<dyn BlockDevice>,
    ) -> Self {
        Self {
            block_id: block_id as usize,
            block_offset,
            fs,
            block_device,
        }
    }
    /// Call a function over a disk `inode` to read it
    /// 简化的访问方法
    fn read_disk_inode<V>(&self, f: impl FnOnce(&DiskInode) -> V) -> V {
        get_block_cache(self.block_id, Arc::clone(&self.block_device))
            .lock()
            .read(self.block_offset, f)
    }
    /// Call a function over a disk `inode` to modify it
    /// 简化的访问方法
    fn modify_disk_inode<V>(&self, f: impl FnOnce(&mut DiskInode) -> V) -> V {
        get_block_cache(self.block_id, Arc::clone(&self.block_device))
            .lock()
            .modify(self.block_offset, f)
    }
    /// Find `inode` under a disk `inode` by name
    fn find_inode_id(&self, name: &str, disk_inode: &DiskInode) -> Option<u32> {
        // Assert it is a directory
        assert!(disk_inode.is_dir());
        let file_count = (disk_inode.size as usize) / DIRENT_SZ;
        let mut dirent = DirEntry::empty();
        for i in 0..file_count {
            assert_eq!(
                disk_inode.read_at(DIRENT_SZ * i, dirent.as_bytes_mut(), &self.block_device,),
                DIRENT_SZ,
            );
            if dirent.name() == name {
                return Some(dirent.inode_id() as u32);
            }
        }
        None
    }
    /// Find `inode` under current `inode` by name
    /// 因为文件是扁平的，我们不需要实现目录索引
    pub fn find(&self, name: &str) -> Option<Arc<Inode>> {
        let fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| {
            self.find_inode_id(name, disk_inode).map(|inode_id| {
                let (block_id, block_offset) = fs.get_disk_inode_pos(inode_id);
                Arc::new(Self::new(
                    block_id,
                    block_offset,
                    self.fs.clone(),
                    self.block_device.clone(),
                ))
            })
        })
    }
    /// Increase the size of a disk `inode`
    /// 需要注意在 DiskInode::write_at 之前先调用 increase_size 对自身进行扩容
    fn increase_size(
        &self,
        new_size: u32,
        disk_inode: &mut DiskInode,
        fs: &mut MutexGuard<EasyFileSystem>,
    ) {
        if new_size < disk_inode.size {
            return;
        }
        let blocks_needed = disk_inode.blocks_num_needed(new_size);
        let mut v: Vec<u32> = Vec::new();
        for _ in 0..blocks_needed {
            v.push(fs.alloc_data());
        }
        disk_inode.increase_size(new_size, v, &self.block_device);
    }
    /// Create `inode` under current `inode` by name
    /// 可以在根目录下创建一个文件，该方法只有根目录的`Inode`会调用
    pub fn create(&self, name: &str) -> Option<Arc<Inode>> {
        let mut fs = self.fs.lock();
        let op = |root_inode: &DiskInode| {
            // Assert it is a directory
            assert!(root_inode.is_dir());
            // Has the file been created?
            self.find_inode_id(name, root_inode)
        };
        if self.read_disk_inode(op).is_some() {
            return None;
        }
        // Create a new file
        // Alloc a `inode` with an indirect block
        let new_inode_id = fs.alloc_inode();
        // Initialize `inode`
        let (new_inode_block_id, new_inode_block_offset) = fs.get_disk_inode_pos(new_inode_id);
        get_block_cache(new_inode_block_id as usize, Arc::clone(&self.block_device))
            .lock()
            .modify(new_inode_block_offset, |new_inode: &mut DiskInode| {
                new_inode.initialize(DiskInodeType::File);
            });
        self.modify_disk_inode(|root_inode| {
            // Append file in the dirent
            let file_count = (root_inode.size as usize) / DIRENT_SZ;
            let new_size = (file_count + 1) * DIRENT_SZ;
            // Increase size
            self.increase_size(new_size as u32, root_inode, &mut fs);
            // Write dirent
            let dirent = DirEntry::new(name, new_inode_id);
            root_inode.write_at(
                file_count * DIRENT_SZ,
                dirent.as_bytes(),
                &self.block_device,
            );
        });

        let (block_id, block_offset) = fs.get_disk_inode_pos(new_inode_id);
        block_cache_sync_all();
        // Return `inode`
        Some(Arc::new(Self::new(
            block_id,
            block_offset,
            self.fs.clone(),
            self.block_device.clone(),
        )))
        // Release `efs` lock automatically by compiler
    }
    /// List `inodes` under current `inode`
    /// `ls`方法可以收集根目录下的所有文件的文件名并以向量的形式返回
    pub fn ls(&self) -> Vec<String> {
        let _fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| {
            let file_count = (disk_inode.size as usize) / DIRENT_SZ;
            let mut v: Vec<String> = Vec::new();
            for i in 0..file_count {
                let mut dirent = DirEntry::empty();
                assert_eq!(
                    disk_inode.read_at(i * DIRENT_SZ, dirent.as_bytes_mut(), &self.block_device,),
                    DIRENT_SZ,
                );
                v.push(String::from(dirent.name()));
            }
            v
        })
    }
    /// Read data from current `inode`
    /// 从根目录索引到一个文件之后可以对它进行读写，注意，和 DiskInode 一样，这里的读写作用在字节序列的一段区间上
    pub fn read_at(&self, offset: usize, buf: &mut [u8]) -> usize {
        let _fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| disk_inode.read_at(offset, buf, &self.block_device))
    }
    /// Write data to current `inode`
    /// 从根目录索引到一个文件之后可以对它进行读写，注意，和 DiskInode 一样，这里的读写作用在字节序列的一段区间上
    pub fn write_at(&self, offset: usize, buf: &[u8]) -> usize {
        let mut fs = self.fs.lock();
        let size = self.modify_disk_inode(|disk_inode| {
            self.increase_size((offset + buf.len()) as u32, disk_inode, &mut fs);
            disk_inode.write_at(offset, buf, &self.block_device)
        });
        block_cache_sync_all();
        size
    }
    /// Clear the data in current `inode`
    /// 在以某些标志位打开文件（例如带有 CREATE 标志打开一个已经存在的文件）的时候，需要首先将文件清空。
    /// 在索引到文件的 Inode 之后可以调用 clear 方法
    pub fn clear(&self) {
        let mut fs = self.fs.lock();
        self.modify_disk_inode(|disk_inode| {
            let size = disk_inode.size;
            let data_blocks_dealloc = disk_inode.clear_size(&self.block_device);
            assert!(data_blocks_dealloc.len() == DiskInode::total_blocks(size) as usize);
            for data_block in data_blocks_dealloc.into_iter() {
                fs.dealloc_data(data_block);
            }
        });
        block_cache_sync_all();
    }

    /// Implement in [CH6]
    /// Create hard link, only ROOT_NODE can call it
    pub fn link(&self, old: &str, new: &str) -> Option<Arc<Inode>> {
        let mut fs = self.fs.lock();
        let op = |root_inode: &DiskInode| {
            // Assert it is a directory
            assert!(root_inode.is_dir());
            // Has the file been created?
            self.find_inode_id(old, root_inode)
        };
        if let Some(old_inode_id) = self.read_disk_inode(op) {
            // We need to keep old `inode` and new `inode` has the same 'block_id' and 'block_offset'.
            // Thus we can create a hard link.
            let new_inode_id = old_inode_id;
            let (new_inode_block_id, new_inode_block_offset) = fs.get_disk_inode_pos(new_inode_id);
            self.modify_disk_inode(|root_inode| {
                // Append file in the dirent
                let file_count = (root_inode.size as usize) / DIRENT_SZ;
                let new_size = (file_count + 1) * DIRENT_SZ;
                // Increase size
                self.increase_size(new_size as u32, root_inode, &mut fs);
                // Write dirent
                let dirent = DirEntry::new(new, new_inode_id);
                root_inode.write_at(
                    file_count * DIRENT_SZ,
                    dirent.as_bytes(),
                    &self.block_device,
                );
            });
            Some(Arc::new(Self::new(
                new_inode_block_id,
                new_inode_block_offset,
                self.fs.clone(),
                self.block_device.clone(),
            )))
        } else {
            // The old directory doesn't exist
            None
        }
    }

    /// Implement in [CH6]
    pub fn unlink(&self, name: &str) -> isize {
        let _fs = self.fs.lock();
        let op = |root_inode: &DiskInode| {
            // Assert it is a directory
            assert!(root_inode.is_dir());
            // Has the file been created?
            self.find_inode_id(name, root_inode)
        };
        // Only when we find the path name, can we unlink it
        if let Some(_) = self.read_disk_inode(op) {
            self.modify_disk_inode(|root_inode| {
                let mut buf = DirEntry::empty();
                let mut swap = DirEntry::empty();
                let file_count = (root_inode.size as usize) / DIRENT_SZ;
                for i in 0..file_count {
                    if root_inode.read_at(DIRENT_SZ * i, buf.as_bytes_mut(), &self.block_device)
                        == DIRENT_SZ
                    {
                        if buf.name() == name {
                            // We are asked not to delete the node so we overwrite the node
                            root_inode.read_at(
                                DIRENT_SZ * (file_count - 1),
                                swap.as_bytes_mut(),
                                &self.block_device,
                            );
                            root_inode.write_at(
                                DIRENT_SZ * i,
                                swap.as_bytes_mut(),
                                &self.block_device,
                            );
                            root_inode.size -= DIRENT_SZ as u32;
                            // Unlink one per call
                            break;
                        }
                    }
                }
            });
            0
        } else {
            // Cannot find the file
            -1
        }
    }

    /// Implement in [CH6]
    pub fn get_link_num(&self, block_id: usize, block_offset: usize) -> u32 {
        let fs = self.fs.lock();
        let mut count = 0;
        self.read_disk_inode(|root_inode| {
            let mut buf = DirEntry::empty();
            let file_count = (root_inode.size as usize) / DIRENT_SZ;
            for i in 0..file_count {
                assert_eq!(
                    root_inode.read_at(DIRENT_SZ * i, buf.as_bytes_mut(), &self.block_device),
                    DIRENT_SZ,
                );
                let (this_inode_block_id, this_inode_block_offset) =
                    fs.get_disk_inode_pos(buf.inode_id());
                if this_inode_block_id as usize == block_id
                    && this_inode_block_offset == block_offset
                {
                    count += 1;
                }
            }
        });
        count
    }
}

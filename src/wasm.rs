extern crate alloc;

use tinywasm::{Store, Module};
use crate::ext2::Ext2;
use crate::ramdisk::Ramdisk;
use alloc::sync::Arc;
use crate::print::kprintln;

pub fn run_a_wasm_mod() {
        kprintln!("testing wasm loading...");
        let disk = Ramdisk::new(512);
        let fs = Arc::new(Ext2::<Ramdisk>::new(disk).unwrap());
        let root = fs.get_root().upgrade().unwrap();
        let wasm_mod = root.search("bc").unwrap().upgrade().unwrap();
        let mut buf = alloc::vec![0u8; 4096];
        {
            let mut inode = wasm_mod.inode.lock();
            wasm_mod.read_block(0, &mut buf, &inode);
        }
        kprintln!("wasm file contents: {:x?}", &buf[..16]);
        let mut store = Store::default();
}

#[cfg(test)]
mod test {
    use crate::ext2::Ext2;
    use crate::ramdisk::Ramdisk;
    use alloc::sync::Arc;
    use crate::print::kprintln;
    use tinywasm::{Store, Module};

    #[test_case]
    fn test_wasm() {
        kprintln!("testing wasm loading...");
        let disk = Ramdisk::new(512);
        let fs = Arc::new(Ext2::<Ramdisk>::new(disk).unwrap());
        let root = fs.get_root().upgrade().unwrap();
        let wasm_mod = root.search("bc").unwrap().upgrade().unwrap();
        let mut buf = alloc::vec![0u8; 4096];
        {
            let mut inode = wasm_mod.inode.lock();
            wasm_mod.read_block(0, &mut buf, &inode);
        }
        kprintln!("wasm file contents: {:x?}", &buf[..16]);
        let mut store = Store::default();

    }
}
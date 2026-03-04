use crate::print::kprintln;
use fdt::Fdt;

// takes the raw dtb pointer from limine and prints every node in the tree
pub unsafe fn walk_device_tree(dtb_ptr: *const ()) {
    let ptr = dtb_ptr as *const u8;

    let fdt = match unsafe { Fdt::from_ptr(ptr) } {
        Ok(fdt) => fdt,
        Err(e) => {
            kprintln!("device_tree: failed to parse DTB: {:?}", e);
            return;
        }
    };

    kprintln!("device_tree: parsed OK, {} bytes total", fdt.total_size());

    for node in fdt.all_nodes() {
        // compatible is a list of strings identifying the device, we just want the first one
        let compat = node
            .compatible()
            .map(|c| c.first())
            .unwrap_or("(none)");

        kprintln!("  node: {}  compatible: {}", node.name, compat);
    }
}

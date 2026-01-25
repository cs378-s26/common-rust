extern crate alloc;

use alloc::boxed::Box;
use intrusive_collections::LinkedListAtomicLink;

pub struct Thread {
    link: LinkedListAtomicLink,
    pub tls: Box<[u8]>,
}

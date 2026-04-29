pub mod context;
pub mod controller;
pub mod descriptors;
pub mod device;
pub mod discovery;
pub mod events;
pub mod registers;
pub mod ring;

use alloc::{sync::Arc, vec::Vec};

use crate::sync::IntMutex;

pub static CONTROLLERS: IntMutex<Vec<Arc<controller::XhciController>>> = IntMutex::new(Vec::new());

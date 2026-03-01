pub mod device;
pub mod acpi;
pub mod pci;

pub use acpi::{init_acpi, acpi_tables, get_acpi};
pub use pci::init_pci;

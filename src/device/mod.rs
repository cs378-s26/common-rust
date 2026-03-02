pub mod acpi;
pub mod pci;

pub use acpi::{acpi_tables, get_acpi, init_acpi};
pub use pci::init_pci;

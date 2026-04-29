

use alloc::{vec, vec::Vec};

use crate::devices::usb::xhci::registers::{
    USB_DESC_CONFIGURATION, USB_DESC_ENDPOINT, USB_DESC_INTERFACE,
};

#[derive(Debug, Clone, Copy)]
pub struct DeviceDescriptor {
    pub b_length: u8,
    pub b_descriptor_type: u8,
    pub bcd_usb: u16,
    pub b_device_class: u8,
    pub b_device_subclass: u8,
    pub b_device_protocol: u8,
    pub b_max_packet_size0: u8,
    pub id_vendor: u16,
    pub id_product: u16,
    pub bcd_device: u16,
    pub i_manufacturer: u8,
    pub i_product: u8,
    pub i_serial_number: u8,
    pub b_num_configurations: u8,
}

impl DeviceDescriptor {
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 18 {
            return None;
        }
        Some(Self {
            b_length: bytes[0],
            b_descriptor_type: bytes[1],
            bcd_usb: u16::from_le_bytes([bytes[2], bytes[3]]),
            b_device_class: bytes[4],
            b_device_subclass: bytes[5],
            b_device_protocol: bytes[6],
            b_max_packet_size0: bytes[7],
            id_vendor: u16::from_le_bytes([bytes[8], bytes[9]]),
            id_product: u16::from_le_bytes([bytes[10], bytes[11]]),
            bcd_device: u16::from_le_bytes([bytes[12], bytes[13]]),
            i_manufacturer: bytes[14],
            i_product: bytes[15],
            i_serial_number: bytes[16],
            b_num_configurations: bytes[17],
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ConfigurationDescriptor {
    pub b_length: u8,
    pub b_descriptor_type: u8,
    pub w_total_length: u16,
    pub b_num_interfaces: u8,
    pub b_configuration_value: u8,
}

impl ConfigurationDescriptor {
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 9 {
            return None;
        }
        Some(Self {
            b_length: bytes[0],
            b_descriptor_type: bytes[1],
            w_total_length: u16::from_le_bytes([bytes[2], bytes[3]]),
            b_num_interfaces: bytes[4],
            b_configuration_value: bytes[5],
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct InterfaceDescriptor {
    pub b_interface_number: u8,
    pub b_alternate_setting: u8,
    pub b_num_endpoints: u8,
    pub b_interface_class: u8,
    pub b_interface_subclass: u8,
    pub b_interface_protocol: u8,
}

#[derive(Debug, Clone, Copy)]
pub struct EndpointDescriptor {
    pub b_endpoint_address: u8,
    pub bm_attributes: u8,
    pub w_max_packet_size: u16,
    pub b_interval: u8,
}

impl EndpointDescriptor {
    pub fn ep_number(&self) -> u8 {
        self.b_endpoint_address & 0x0F
    }

    pub fn is_in(&self) -> bool {
        (self.b_endpoint_address & 0x80) != 0
    }

    pub fn transfer_type(&self) -> u8 {
        self.bm_attributes & 0x03
    }

    /// xHCI Device Context Index per  4.8.1. EP0 (DCI 1) is handled
    /// separately — this returns DCIs for non-control endpoints.
    pub fn dci(&self) -> u8 {
        2 * self.ep_number() + if self.is_in() { 1 } else { 0 }
    }
}

pub struct ParsedConfig {
    pub config: ConfigurationDescriptor,
    pub interfaces: Vec<(InterfaceDescriptor, Vec<EndpointDescriptor>)>,
}

/// Parse a configuration descriptor blob. Class-specific descriptors
/// (HID, etc.) interleaved between standard ones are skipped.
pub fn parse_configuration(bytes: &[u8]) -> Option<ParsedConfig> {
    let config = ConfigurationDescriptor::from_bytes(bytes)?;
    let mut offset = config.b_length as usize;

    let mut interfaces: Vec<(InterfaceDescriptor, Vec<EndpointDescriptor>)> = vec![];

    while offset + 2 <= bytes.len() {
        let len = bytes[offset] as usize;
        let dtype = bytes[offset + 1];
        if len == 0 || offset + len > bytes.len() {
            break;
        }

        match dtype {
            USB_DESC_INTERFACE if len >= 9 => {
                let iface = InterfaceDescriptor {
                    b_interface_number: bytes[offset + 2],
                    b_alternate_setting: bytes[offset + 3],
                    b_num_endpoints: bytes[offset + 4],
                    b_interface_class: bytes[offset + 5],
                    b_interface_subclass: bytes[offset + 6],
                    b_interface_protocol: bytes[offset + 7],
                };
                interfaces.push((iface, vec![]));
            }
            USB_DESC_ENDPOINT if len >= 7 => {
                let ep = EndpointDescriptor {
                    b_endpoint_address: bytes[offset + 2],
                    bm_attributes: bytes[offset + 3],
                    w_max_packet_size: u16::from_le_bytes([bytes[offset + 4], bytes[offset + 5]]),
                    b_interval: bytes[offset + 6],
                };
                if let Some(last) = interfaces.last_mut() {
                    last.1.push(ep);
                }
            }
            USB_DESC_CONFIGURATION => {}
            _ => {}
        }

        offset += len;
    }

    Some(ParsedConfig { config, interfaces })
}

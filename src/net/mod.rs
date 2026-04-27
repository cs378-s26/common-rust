pub mod arp;
pub mod ethernet;
pub mod icmp;
pub mod ipv4;

/// an IPv4 address — shared between arp and ipv4
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ipv4Addr(pub [u8; 4]);

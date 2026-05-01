use alloc::vec::Vec;

use crate::{
    devices::discovery::NETWORK_DEVICES,
    sync::MutexLike,
    net::{
        Ipv4Addr,
        arp::{ARP_TABLE, ArpAction, ArpPacket},
        ethernet::{EtherType, EthernetFrame, MacAddr, build_ethernet_frame},
        icmp::{IcmpEcho, build_echo_reply},
        ipv4::{Ipv4Header, NetConfig, Protocol, build_ipv4_packet, get_net_config},
        udp::{UDP_DEMUX, UdpDatagram, UdpHeader},
    },
    thread::yield_thread,
};

// 1536 covers a max-size ethernet frame (14 header + 1500 payload + 22 slack)
const BUF_SIZE: usize = 1536;

pub fn start_network_receive_loop() {
    crate::thread::spawn_thread(receive_loop);
}

fn receive_loop() {
    let mut buf = [0u8; BUF_SIZE];

    loop {
        let result = {
            let mut devs = NETWORK_DEVICES.lock();
            match devs.first_mut() {
                None => None,
                Some(nic) => match nic.receive_packet(&mut buf) {
                    Ok(len) => Some((len, nic.mac_address())),
                    Err(_) => None,
                },
            }
        };

        match result {
            None => yield_thread(),
            Some((len, our_mac)) => {
                let Some(config) = get_net_config() else {
                    continue;
                };
                process_packet(&buf[..len], our_mac, config);
            }
        }
    }
}

fn send(packet: &[u8]) {
    let mut devs = NETWORK_DEVICES.lock();
    if let Some(nic) = devs.first_mut() {
        let _ = nic.send_packet(packet);
    }
}

fn process_packet(raw: &[u8], our_mac: MacAddr, config: &NetConfig) {
    let Ok(frame) = EthernetFrame::parse(raw) else {
        return;
    };

    match frame.ether_type {
        EtherType::Arp => handle_arp(frame.payload, our_mac, config),
        EtherType::Ipv4 => handle_ipv4(frame.payload, frame.src, our_mac, config),
        _ => {}
    }
}

fn handle_arp(payload: &[u8], our_mac: MacAddr, config: &NetConfig) {
    let Ok(packet) = ArpPacket::parse(payload) else {
        return;
    };

    match ARP_TABLE.handle_incoming(&packet, config.ip, our_mac) {
        ArpAction::None => {}
        ArpAction::SendReply { dst_mac, payload: arp_bytes } => {
            let mut frame_buf = [0u8; BUF_SIZE];
            if let Ok(len) =
                build_ethernet_frame(dst_mac, our_mac, EtherType::Arp, &arp_bytes, &mut frame_buf)
            {
                send(&frame_buf[..len]);
            }
        }
    }
}

fn handle_ipv4(payload: &[u8], frame_src_mac: MacAddr, our_mac: MacAddr, config: &NetConfig) {
    let Ok(ip) = Ipv4Header::parse(payload) else {
        return;
    };

    if ip.dst != config.ip {
        return;
    }

    match ip.protocol {
        Protocol::Icmp => handle_icmp(ip.payload, ip.src, frame_src_mac, our_mac, config),
        Protocol::Udp => handle_udp(ip.payload, ip.src, ip.dst),
        _ => {}
    }
}

fn handle_icmp(
    payload: &[u8],
    src_ip: Ipv4Addr,
    dst_mac: MacAddr,
    our_mac: MacAddr,
    config: &NetConfig,
) {
    let Ok(echo) = IcmpEcho::parse(payload) else {
        return;
    };
    if !echo.is_request() {
        return;
    }

    let mut icmp_buf = [0u8; BUF_SIZE];
    let Ok(icmp_len) = build_echo_reply(&echo, &mut icmp_buf) else {
        return;
    };

    let mut ip_buf = [0u8; BUF_SIZE];
    let Ok(ip_len) =
        build_ipv4_packet(config.ip, src_ip, Protocol::Icmp, &icmp_buf[..icmp_len], &mut ip_buf)
    else {
        return;
    };

    let mut frame_buf = [0u8; BUF_SIZE];
    if let Ok(frame_len) =
        build_ethernet_frame(dst_mac, our_mac, EtherType::Ipv4, &ip_buf[..ip_len], &mut frame_buf)
    {
        send(&frame_buf[..frame_len]);
    }
}

fn handle_udp(payload: &[u8], src_ip: Ipv4Addr, dst_ip: Ipv4Addr) {
    let Ok(header) = UdpHeader::parse(payload, src_ip, dst_ip) else {
        return;
    };

    UDP_DEMUX.deliver(UdpDatagram {
        src_ip,
        src_port: header.src_port,
        dst_port: header.dst_port,
        data: Vec::from(header.payload),
    });
}

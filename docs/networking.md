# Networking

This document describes the kernel's network stack: the protocol layers in `src/net/`, the socket abstraction and file descriptor table, the receive loop, and the syscall surface exposed to userspace.

## Overview

The network stack is a from-scratch implementation covering Ethernet through TCP. It is organized as a series of independent, composable layers:

- `src/net/ethernet.rs` — Ethernet II frame parsing and building
- `src/net/arp.rs` — ARP cache and packet handling
- `src/net/ipv4.rs` — IPv4 header parsing, building, and checksum
- `src/net/icmp.rs` — ICMP echo request/reply
- `src/net/udp.rs` — UDP framing, checksum, and port demultiplexing
- `src/net/tcp.rs` — TCP header, connection state machine, and demultiplexing
- `src/net/socket.rs` — `UdpSocket` and `TcpSocket` wrappers that implement `File`
- `src/net/receive.rs` — the kernel receive thread that polls the NIC and dispatches packets
- `src/fs/file.rs` — the `File` trait shared by sockets, regular files, and other fd types
- `src/process.rs` — per-process `FdTable` mapping file descriptors to `Arc<dyn File>`

Outbound packets are built bottom-up: payload → transport header → IPv4 header → Ethernet frame → NIC send. Inbound packets are dismantled top-down by the receive loop and handed off to the matching socket via a demultiplexer.

Concurrency primitives used throughout: `IntMutex` for short critical sections, `Promise<T>` for one-shot blocking rendezvous (ARP lookups, TCP handshake completion), and `BoundedBuffer<T, N>` for bounded receive queues.

## Protocol Layers

### Ethernet

`ethernet.rs` handles Layer 2 framing. The only format supported is Ethernet II (no 802.1Q tags).

```rust
pub struct MacAddr(pub [u8; 6]);

pub enum EtherType { Ipv4, Arp, Unknown(u16) }

pub struct EthernetFrame<'a> {
    pub dst: MacAddr,
    pub src: MacAddr,
    pub ether_type: EtherType,
    pub payload: &'a [u8],
}
```

`EthernetFrame::parse(buf)` expects at least 14 bytes and returns a frame that borrows the original buffer — no copying. `build_ethernet_frame(dst, src, ether_type, payload, out)` writes the 14-byte header followed by the payload and returns the total byte count.

### ARP

`arp.rs` handles address resolution between IPv4 and MAC addresses.

```rust
pub struct ArpPacket {
    pub operation: u16,        // 1 = request, 2 = reply
    pub sender_mac: MacAddr,
    pub sender_ip:  Ipv4Addr,
    pub target_mac: MacAddr,
    pub target_ip:  Ipv4Addr,
}
```

The global `ARP_TABLE` maintains two maps: a `cache` of resolved entries and a `pending` map of outstanding lookups. Each pending lookup is represented as an `Arc<Promise<MacAddr>>`. When `handle_incoming()` receives a reply it updates the cache and resolves any matching promise, unblocking any thread that was waiting on it.

Outbound paths (sendto, connect) that need a MAC for a destination IP call `ARP_TABLE.resolve()` first. If the address is already cached that returns immediately. If not, they send an ARP request and block on `ARP_TABLE.start_lookup()` until a reply arrives.

`handle_incoming()` returns `ArpAction::SendReply` when the packet is an ARP request directed at our IP, so the receive loop can send a reply without knowing ARP internals.

### IPv4

`ipv4.rs` handles Layer 3 routing and framing.

```rust
pub struct Ipv4Addr(pub [u8; 4]);

pub enum Protocol { Icmp, Tcp, Udp, Other(u8) }

pub struct Ipv4Header<'a> {
    pub src:      Ipv4Addr,
    pub dst:      Ipv4Addr,
    pub protocol: Protocol,
    pub payload:  &'a [u8],
}
```

`parse()` validates the version field, verifies the one's-complement checksum, and returns a header that borrows the payload slice. Fragmented packets are accepted (the DF bit is set on outbound packets, but not enforced on inbound).

`build_ipv4_packet()` writes a fixed 20-byte header (IHL=5, no options) with TTL=64 and the appropriate checksum.

The `checksum()` function implements RFC 791: sum all 16-bit big-endian words, fold the carry into the low 16 bits, invert. A correctly received packet sums to zero.

The system IP address, netmask, and gateway are stored in a `NetConfig` that is set once at boot by `set_net_config()` and queried via `get_net_config()`. The receive loop drops incoming IPv4 packets whose destination does not match our configured IP.

### ICMP

`icmp.rs` handles ICMP echo requests (type 8) and replies (type 0).

```rust
pub struct IcmpEcho<'a> {
    pub is_request: bool,
    pub id:         u16,
    pub seq:        u16,
    pub data:       &'a [u8],
}
```

`parse()` checks the type byte and verifies the checksum. `build_echo_reply()` constructs a reply in place, copying the request's ID, sequence number, and data and computing a fresh checksum. The receive loop calls this directly and sends the reply — no sockets are involved.

### UDP

`udp.rs` handles connectionless datagram transport.

```rust
pub struct UdpHeader<'a> {
    pub src_port: u16,
    pub dst_port: u16,
    pub payload:  &'a [u8],
}

pub struct UdpDatagram {
    pub src_ip:   Ipv4Addr,
    pub src_port: u16,
    pub dst_port: u16,
    pub data:     Vec<u8>,
}
```

The checksum covers a 12-byte pseudo-header (src IP | dst IP | zero | proto=17 | UDP length) followed by the UDP segment, per RFC 768. A checksum of zero in a received packet means the sender skipped it; `parse()` accepts that.

`UDP_DEMUX` is a global demultiplexer:

```rust
UDP_DEMUX.register(port, sink: Arc<dyn UdpSink>)
UDP_DEMUX.unregister(port)
UDP_DEMUX.is_port_bound(port) -> bool
UDP_DEMUX.deliver(datagram)
```

`UdpSink` is a one-method trait implemented by `UdpSocket`. When a packet arrives the receive loop calls `UDP_DEMUX.deliver()` which looks up the destination port and calls `sink.receive()`. If no socket is registered for that port the datagram is silently dropped.

### TCP

`tcp.rs` is the most complex module. It covers wire format, the transmission control block (TCB), the connection state machine, and the demultiplexer.

#### Wire format

```rust
pub struct TcpHeader<'a> {
    pub src_port: u16,
    pub dst_port: u16,
    pub seq:      u32,
    pub ack:      u32,
    pub flags:    u8,
    pub window:   u16,
    pub mss:      Option<u16>,  // extracted from SYN options if present
    pub payload:  &'a [u8],
}
```

`parse()` reads the 20-byte fixed header, scans options for MSS (kind=2), and verifies the checksum over the TCP pseudo-header (src IP | dst IP | zero | proto=6 | TCP length). The flag constants are `FLAG_FIN`, `FLAG_SYN`, `FLAG_RST`, `FLAG_PSH`, and `FLAG_ACK`.

`build_tcp_segment()` writes either a 20-byte header (no options) or a 24-byte header (with MSS option when `mss: Some(v)` is passed). The checksum is computed over the same pseudo-header.

#### Connection state

Each connection owns a `TcpConn`:

```rust
pub struct TcpConn {
    pub inner:        IntMutex<TcbInner>,
    pub rx_buf:       BoundedBuffer<Vec<u8>, 32>,
    pub accept_queue: BoundedBuffer<Arc<TcpConn>, 8>,
    pub listener:     Option<Weak<TcpConn>>,
    pub connected:    Arc<Promise<()>>,
}

pub struct TcbInner {
    pub state:       TcpState,
    pub local_addr:  Ipv4Addr,
    pub local_port:  u16,
    pub remote_addr: Ipv4Addr,
    pub remote_port: u16,
    pub remote_mac:  MacAddr,
    pub snd_nxt:     u32,
    pub snd_una:     u32,
    pub rcv_nxt:     u32,
    pub rcv_wnd:     u16,
    pub mss:         u16,
}
```

`rx_buf` holds inbound data segments queued for `read()`. `accept_queue` holds fully-established child connections waiting for `accept()`. `listener` is a weak back-reference from a child to the listener that spawned it. `connected` is resolved when the three-way handshake completes, unblocking a `connect()` syscall.

#### State machine

```
Closed → Listen                       (listen)
Closed → SynSent                      (connect)
Listen → (child created in SynRcvd)   (incoming SYN)
SynSent → Established                 (incoming SYN-ACK + sending ACK)
SynRcvd → Established                 (incoming ACK)
Established → FinWait1                (local close)
Established → CloseWait               (incoming FIN)
FinWait1 → FinWait2                   (incoming ACK of our FIN)
FinWait2 → Closed                     (incoming FIN)
CloseWait → LastAck                   (local close)
LastAck → Closed                      (incoming ACK of our FIN)
```

`listen()` transitions to Listen and registers the connection in `TCP_DEMUX` as a listener. It reads `local_port` from `inner` inside the same lock acquisition that sets the state, so no race exists between the state change and the registration.

`connect()` generates an ISN from the global `ISN_COUNTER` (incremented by 100 per connection), sends a SYN with the MSS option, and registers in `TCP_DEMUX` as an established-table entry.

Incoming segments are dispatched from the receive loop to `handle_segment()`, which branches on the current state:

- `handle_listen()` — creates a child `TcpConn` in SynRcvd, sends SYN-ACK, registers the child in the established table
- `handle_syn_rcvd()` — on the correct ACK, moves to Established and pushes the child into the listener's `accept_queue`; if the queue is full it sends RST and unregisters
- `handle_syn_sent()` — validates the SYN-ACK, moves to Established, sends the final ACK, and resolves the `connected` promise
- `handle_established()` — ACKs inbound data, queues payload in `rx_buf`, and moves to CloseWait on an incoming FIN
- `handle_fin_wait1/2()`, `handle_peer_fin()`, `handle_last_ack()` — drive the close sequence to completion and unregister from the demux when the connection reaches Closed

#### Demultiplexing

`TCP_DEMUX` routes incoming segments to the right connection:

```rust
TCP_DEMUX.register_listener(port, conn)
TCP_DEMUX.unregister_listener(port)
TCP_DEMUX.register_established(local_port, remote_addr, remote_port, conn)
TCP_DEMUX.unregister_established(local_port, remote_addr, remote_port)
TCP_DEMUX.deliver(src_ip, dst_ip, src_mac, segment)
```

`deliver()` first looks up the 3-tuple `(dst_port, src_ip, src_port)` in the established table. If not found it falls back to the listener table keyed by `dst_port`. Connections in terminal states (Closed) unregister themselves from the demux automatically.

## Socket Abstraction

### File trait

`src/fs/file.rs` defines the interface shared by all file descriptor types:

```rust
pub trait File: Send + Sync {
    fn read(&self,  buf: &mut [u8]) -> Result<usize, FileError>;
    fn write(&self, buf: &[u8])     -> Result<usize, FileError>;
    fn close(&self)                 -> Result<(), FileError>;
    fn as_any_arc(self: Arc<Self>)  -> Arc<dyn Any + Send + Sync>;
}
```

`as_any_arc()` enables downcasting at the syscall boundary when a socket-specific operation (e.g. `sendto`, `bind`) needs access to the concrete socket type that a plain `Arc<dyn File>` does not expose.

### UdpSocket

`UdpSocket` holds the local binding and a bounded receive queue:

```rust
pub struct UdpSocket {
    local_addr: IntMutex<Option<(Ipv4Addr, u16)>>,
    rx_queue:   BoundedBuffer<UdpDatagram, 32>,
}
```

It implements `UdpSink` so it can be registered with `UDP_DEMUX`. Inbound datagrams are pushed into `rx_queue` by the receive loop; `read()` pops from that queue, blocking until one arrives. `close()` unregisters from `UDP_DEMUX`.

UDP `write()` is not implemented — outbound sends go through `sys_sendto`, which builds the entire UDP/IPv4/Ethernet stack directly rather than going through the `File` interface.

### TcpSocket

`TcpSocket` is a thin wrapper around an `Arc<TcpConn>`:

```rust
pub struct TcpSocket {
    pub conn: Arc<TcpConn>,
}
```

`read()` pops a segment from `conn.rx_buf`, blocking until data is available. `write()` segments the buffer by the connection's negotiated MSS, builds and sends each chunk as a PSH|ACK segment, and updates `snd_nxt`. `close()` calls `conn.close()`, which sends a FIN and drives the state machine toward Closed.

### FD table

Each process holds an `FdTable`:

```rust
pub struct FdTable {
    // maps fd (i32) → Arc<dyn File>
}
```

`insert(file)` allocates the lowest available non-negative integer and returns it as the fd. `get(fd)` and `remove(fd)` do lookups. The table is wrapped in an `IntMutex` on `Process` so syscalls from different threads on the same process serialize cleanly.

## Receive Loop

`src/net/receive.rs` spawns a dedicated kernel thread via `start_network_receive_loop()`. The thread runs `receive_loop()` in a tight poll:

1. Lock `NETWORK_DEVICES`, grab the first NIC, call `receive_packet()`. If nothing is ready, yield.
2. Strip ANSI escapes / framing noise, parse the Ethernet header.
3. Branch on `EtherType`:
   - **ARP** — parse the packet, call `ARP_TABLE.handle_incoming()`. If it returns `SendReply`, build an ARP reply and send it.
   - **IPv4** — parse the IPv4 header. Drop the packet if the destination IP does not match `NetConfig`. Branch on `Protocol`:
     - **ICMP** — parse the echo request, build a reply, send it.
     - **UDP** — parse the UDP header, build a `UdpDatagram`, call `UDP_DEMUX.deliver()`.
     - **TCP** — call `TCP_DEMUX.deliver()`, which routes to the matching connection and drives its state machine.

The receive loop never blocks on a promise or sleeps on a queue. Critical sections inside the demux tables and connection state are kept short so the loop does not stall behind application code.

## Syscall Interface

Socket syscalls follow the standard POSIX shape. All argument parsing uses the `SyscallContext` trait to read registers and validate user-space pointers.

### socket

```
sys_socket(domain, type, protocol) -> fd
```

`domain` must be `AF_INET` (2). `type` selects the socket kind:

- `SOCK_STREAM` (1) — creates a `TcpSocket` with an unbound `TcpConn`
- `SOCK_DGRAM` (2) — creates a `UdpSocket`

Returns the new fd or `EINVAL`.

### bind

```
sys_bind(fd, sockaddr_ptr, addrlen) -> 0 or error
```

Parses a `sockaddr_in` (16 bytes: 2-byte family, 2-byte port big-endian, 4-byte IP, 8-byte padding). For UDP it stores the local address and calls `UDP_DEMUX.register()`, returning `EADDRINUSE` if the port is already taken. For TCP it sets `local_addr` and `local_port` on the `TcbInner`.

### sendto

```
sys_sendto(fd, buf, len, flags, dest_addr_ptr, addrlen) -> bytes_sent or error
```

UDP only. Copies the user buffer into the kernel, parses the destination `sockaddr_in`, resolves the destination MAC (ARP cache or blocking lookup), then builds UDP → IPv4 → Ethernet and sends. Returns the number of bytes sent from the original payload.

### recvfrom

```
sys_recvfrom(fd, buf, len, flags, src_addr_ptr, addrlen_ptr) -> bytes_received or error
```

UDP only. Blocks on `socket.recv_datagram()` until a datagram arrives, copies the payload into the user buffer, and optionally writes the sender's address and address length back through `src_addr_ptr` / `addrlen_ptr`.

### listen

```
sys_listen(fd, backlog) -> 0 or error
```

TCP only. Calls `conn.listen()`, which transitions the connection to Listen and registers it in `TCP_DEMUX`. The `backlog` argument is accepted but not used — the accept queue depth is fixed at 8.

### accept / accept4

```
sys_accept4(fd, addr_ptr, addrlen_ptr, flags) -> new_fd or error
```

TCP only. Blocks on `conn.accept_queue.pop()` until a fully-established child connection is available. Optionally writes the peer's address back through `addr_ptr` / `addrlen_ptr`. Inserts the child `TcpConn` as a new `TcpSocket` in the fd table and returns the new fd.

### connect

```
sys_connect(fd, addr_ptr, addrlen) -> 0 or error
```

TCP only. Parses the destination `sockaddr_in`. If the socket has not been bound, uses the kernel's configured IP as the local address. Resolves the destination MAC via ARP (blocking if needed). Calls `conn.connect()`, which sends a SYN and transitions to SynSent, then blocks on `conn.connected` until the three-way handshake completes.

### read, write, close

```
sys_read(fd, buf, count)   -> bytes_read or error
sys_write(fd, buf, count)  -> bytes_written or error
sys_close(fd)              -> 0 or error
```

Generic over any `File`. `read` calls `file.read()`, which for TCP blocks until data is in `rx_buf` and for UDP blocks until a datagram arrives. `write` calls `file.write()`, which for TCP segments and sends, for UDP is unsupported. `close` removes the fd from the table and calls `file.close()`.

### Error codes

| Code | Value | Meaning |
|------|-------|---------|
| EBADF | -9 | bad file descriptor |
| EIO | -5 | I/O error |
| EFAULT | -14 | invalid user pointer |
| EINVAL | -22 | invalid argument |
| ENOTSOCK | -88 | fd is not a socket |
| EADDRINUSE | -98 | port already bound |

## Data flow

### Sending a UDP datagram

1. `sys_sendto` copies user data, resolves destination MAC (ARP)
2. `build_udp_packet` — 8-byte header + payload, pseudo-header checksum
3. `build_ipv4_packet` — 20-byte header, one's-complement checksum
4. `build_ethernet_frame` — 14-byte header
5. NIC send

### Receiving a UDP datagram

1. Receive loop polls NIC, parses Ethernet → IPv4 → UDP
2. `UDP_DEMUX.deliver()` routes by destination port to the registered `UdpSocket`
3. Datagram is pushed into `socket.rx_queue`
4. `sys_recvfrom` (blocked on `rx_queue.pop()`) unblocks, copies payload to userspace

### TCP three-way handshake (server side)

1. `sys_socket` + `sys_bind` + `sys_listen` → listener in Listen state, registered in `TCP_DEMUX`
2. Incoming SYN → `handle_listen()` creates child in SynRcvd, sends SYN-ACK
3. Incoming ACK → `handle_syn_rcvd()` moves child to Established, pushes to `accept_queue`
4. `sys_accept4` (blocked on `accept_queue.pop()`) unblocks, returns new fd

### TCP three-way handshake (client side)

1. `sys_socket` → unbound `TcpConn`
2. `sys_connect` resolves destination MAC, calls `conn.connect()` (SYN sent, SynSent state)
3. `sys_connect` blocks on `conn.connected`
4. Incoming SYN-ACK → `handle_syn_sent()` → Established, resolves `connected`
5. `sys_connect` unblocks, returns 0

## Current boundaries

The current implementation covers the common-case paths but is deliberately incomplete in several areas.

- **No TCP retransmission.** Lost segments are never resent. A dropped SYN or SYN-ACK will stall `connect()` or `accept()` forever.
- **No TCP TIME_WAIT.** After the final ACK is sent the connection moves directly to Closed. Duplicate delayed segments from the old connection could confuse a new connection on the same port.
- **No TCP flow control.** The advertised window is never updated based on actual buffer space. A fast sender can overflow `rx_buf`, causing silent drops.
- **No ARP timeout.** If an ARP reply never arrives, `sendto` or `connect` will block the calling thread indefinitely.
- **UDP write through File unsupported.** `sys_write` on a UDP socket returns an error; callers must use `sys_sendto`.
- **Single NIC.** The receive loop and send paths both use the first device in `NETWORK_DEVICES`. Multi-homing and routing are not implemented.
- **No IP fragmentation.** Outbound packets set the DF bit. Inbound fragments are passed to the transport layer as-is, which will produce a checksum failure or malformed parse.
- **No userspace-visible raw sockets or select/poll.** Applications can only block one fd at a time per thread.

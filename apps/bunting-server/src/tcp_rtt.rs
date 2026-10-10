//! Kernel TCP round-trip time for one connection (ADR 0034 §2).
//!
//! The kernel measures RTT from ACKs that the peer's operating system sends,
//! which the peer's application cannot delay, and it starts with the
//! handshake sample. Linux exposes it as `tcp_info.tcpi_min_rtt`. The
//! workspace forbids `unsafe`, so instead of `getsockopt(TCP_INFO)` this
//! asks the kernel through a netlink `sock_diag` request for exactly this
//! socket, built and parsed as plain bytes. Other hosts return `None` and
//! the venue falls back to probe RTT.

use std::net::TcpStream;

/// Kernel RTT readings in microseconds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct KernelRtt {
    /// Minimum RTT the kernel has observed (Linux keeps a windowed minimum).
    pub(crate) min_rtt_us: u64,
    /// Smoothed RTT, for health output only.
    pub(crate) smoothed_rtt_us: u64,
}

/// Reads the kernel's RTT for `stream`, or `None` when the host cannot
/// report it (non-Linux, netlink refused, or no sample yet).
pub(crate) fn kernel_rtt(stream: &TcpStream) -> Option<KernelRtt> {
    #[cfg(target_os = "linux")]
    {
        linux::query(stream)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = stream;
        None
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::KernelRtt;
    use rustix::net::netlink::{SOCK_DIAG, SocketAddrNetlink};
    use rustix::net::{
        AddressFamily, RecvFlags, SendFlags, SocketType, recv, sendto, socket,
        sockopt::{Timeout, set_socket_timeout},
    };
    use std::net::{IpAddr, SocketAddr, TcpStream};
    use std::time::Duration;

    const SOCK_DIAG_BY_FAMILY: u16 = 20;
    const NLMSG_ERROR: u16 = 2;
    const NLM_F_REQUEST: u16 = 1;
    const AF_INET: u8 = 2;
    const AF_INET6: u8 = 10;
    const IPPROTO_TCP: u8 = 6;
    const INET_DIAG_INFO: u16 = 2;
    const NLMSG_HEADER: usize = 16;
    const INET_DIAG_MSG: usize = 72;
    /// `tcp_info` offsets (include/uapi/linux/tcp.h).
    const TCPI_RTT: usize = 68;
    const TCPI_MIN_RTT: usize = 148;

    pub(super) fn query(stream: &TcpStream) -> Option<KernelRtt> {
        let request = request(stream.local_addr().ok()?, stream.peer_addr().ok()?)?;
        let netlink = socket(AddressFamily::NETLINK, SocketType::DGRAM, Some(SOCK_DIAG)).ok()?;
        set_socket_timeout(&netlink, Timeout::Recv, Some(Duration::from_millis(100))).ok()?;
        sendto(
            &netlink,
            &request,
            SendFlags::empty(),
            &SocketAddrNetlink::new(0, 0),
        )
        .ok()?;
        let mut buffer = [0_u8; 1_024];
        let (received, _) = recv(&netlink, &mut buffer[..], RecvFlags::empty()).ok()?;
        parse(&buffer[..received])
    }

    /// One `SOCK_DIAG_BY_FAMILY` request for the exact socket, asking for
    /// `INET_DIAG_INFO` (`tcp_info`).
    fn request(local: SocketAddr, peer: SocketAddr) -> Option<Vec<u8>> {
        let family = match (local.ip(), peer.ip()) {
            (IpAddr::V4(_), IpAddr::V4(_)) => AF_INET,
            (IpAddr::V6(_), IpAddr::V6(_)) => AF_INET6,
            _ => return None,
        };
        let mut message = Vec::with_capacity(72);
        message.extend_from_slice(&72_u32.to_ne_bytes());
        message.extend_from_slice(&SOCK_DIAG_BY_FAMILY.to_ne_bytes());
        message.extend_from_slice(&NLM_F_REQUEST.to_ne_bytes());
        message.extend_from_slice(&1_u32.to_ne_bytes()); // sequence
        message.extend_from_slice(&0_u32.to_ne_bytes()); // port id: kernel
        message.extend_from_slice(&[family, IPPROTO_TCP, 1 << (INET_DIAG_INFO - 1), 0]);
        message.extend_from_slice(&u32::MAX.to_ne_bytes()); // every state
        message.extend_from_slice(&local.port().to_be_bytes());
        message.extend_from_slice(&peer.port().to_be_bytes());
        for address in [local.ip(), peer.ip()] {
            let mut bytes = [0_u8; 16];
            match address {
                IpAddr::V4(v4) => bytes[..4].copy_from_slice(&v4.octets()),
                IpAddr::V6(v6) => bytes.copy_from_slice(&v6.octets()),
            }
            message.extend_from_slice(&bytes);
        }
        message.extend_from_slice(&0_u32.to_ne_bytes()); // any interface
        message.extend_from_slice(&u32::MAX.to_ne_bytes()); // no cookie
        message.extend_from_slice(&u32::MAX.to_ne_bytes());
        (message.len() == 72).then_some(message)
    }

    fn u16_at(bytes: &[u8], offset: usize) -> Option<u16> {
        Some(u16::from_ne_bytes(
            bytes.get(offset..offset + 2)?.try_into().ok()?,
        ))
    }

    fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
        Some(u32::from_ne_bytes(
            bytes.get(offset..offset + 4)?.try_into().ok()?,
        ))
    }

    fn parse(response: &[u8]) -> Option<KernelRtt> {
        let length = usize::try_from(u32_at(response, 0)?).ok()?;
        if length > response.len() || u16_at(response, 4)? == NLMSG_ERROR {
            return None;
        }
        let mut offset = NLMSG_HEADER + INET_DIAG_MSG;
        while offset + 4 <= length {
            let attribute_length = usize::from(u16_at(response, offset)?);
            if attribute_length < 4 || offset + attribute_length > length {
                return None;
            }
            if u16_at(response, offset + 2)? == INET_DIAG_INFO {
                let info = &response[offset + 4..offset + attribute_length];
                let min_rtt_us = u64::from(u32_at(info, TCPI_MIN_RTT)?);
                return (min_rtt_us > 0).then(|| KernelRtt {
                    min_rtt_us,
                    smoothed_rtt_us: u64::from(u32_at(info, TCPI_RTT).unwrap_or(0)),
                });
            }
            offset += attribute_length.next_multiple_of(4);
        }
        None
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn requests_are_exact_and_truncated_replies_are_refused() {
            let local: SocketAddr = "127.0.0.1:9880".parse().unwrap_or_else(|_| unreachable!());
            let peer: SocketAddr = "127.0.0.1:40000".parse().unwrap_or_else(|_| unreachable!());
            let built = request(local, peer).unwrap_or_default();
            assert_eq!(built.len(), 72);
            assert_eq!(built[16], AF_INET);
            assert_eq!(&built[24..26], &9880_u16.to_be_bytes());
            assert_eq!(&built[26..28], &40000_u16.to_be_bytes());
            let mixed: SocketAddr = "[::1]:1".parse().unwrap_or_else(|_| unreachable!());
            assert!(request(local, mixed).is_none());
            assert!(parse(&[0; 8]).is_none());
            let mut error = vec![0_u8; 36];
            error[..4].copy_from_slice(&36_u32.to_ne_bytes());
            error[4..6].copy_from_slice(&NLMSG_ERROR.to_ne_bytes());
            assert!(parse(&error).is_none());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Loopback round trip: the accepted (server-side) socket reports a
    /// kernel RTT on Linux, and nothing elsewhere.
    #[test]
    fn accepted_loopback_socket_reports_a_kernel_rtt() -> std::io::Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let mut client = TcpStream::connect(listener.local_addr()?)?;
        let (mut server, _) = listener.accept()?;
        let mut byte = [0_u8; 1];
        for _ in 0..3 {
            server.write_all(b"x")?;
            client.read_exact(&mut byte)?;
            client.write_all(b"y")?;
            server.read_exact(&mut byte)?;
        }
        let reading = kernel_rtt(&server);
        if cfg!(target_os = "linux") {
            let reading = reading.ok_or_else(|| std::io::Error::other("no kernel RTT"))?;
            // Two independent fields agree: a wrong offset would not.
            assert!(reading.min_rtt_us > 0 && reading.min_rtt_us < 100_000);
            assert!(reading.smoothed_rtt_us >= reading.min_rtt_us);
            assert!(reading.smoothed_rtt_us < 1_000_000);
            println!("loopback kernel RTT: {reading:?}");
        } else {
            assert!(reading.is_none());
        }
        Ok(())
    }
}

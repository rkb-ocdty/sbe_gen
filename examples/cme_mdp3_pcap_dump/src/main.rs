//! Example program demonstrating how to decode CME MDP multicast packets
//! using generated SBE types. The build script uses the local `sbe_gen`
//! crate to translate the XML schema under `schemas/` into Rust modules
//! under `src/generated/`.
use anyhow::{Context, Result};
use clap::Parser;
use pcap::Capture;
use std::net::Ipv4Addr;
use std::path::PathBuf;

mod generated {
    pub mod cme_mdp3;
}

// Shims so generated modules that expect crate-local `types` and `message_header`
// continue to compile when nested under `generated::cme_mdp3`.
pub mod types {
    pub use crate::generated::cme_mdp3::types::*;
}

mod dump;
pub mod json_types;

use crate::dump::dump_cme_packet;

#[derive(Parser, Debug)]
#[command(name = "pcap_dump", version, about)]
struct Args {
    /// Pcap file path
    #[arg(long = "pcap")]
    pcap: PathBuf,

    /// Optional: only dump packets whose UDP src port matches
    #[arg(long = "src-port")]
    src_port: Option<u16>,

    /// Optional: only dump packets whose UDP dst port matches
    #[arg(long = "dst-port")]
    dst_port: Option<u16>,

    /// Optional: only dump packets whose UDP src OR dst port matches
    #[arg(long = "udp-port")]
    udp_port: Option<u16>,

    /// Optional: only dump packets from this IPv4 source
    #[arg(long = "src")]
    src: Option<Ipv4Addr>,

    /// Optional: only dump packets to this IPv4 destination
    #[arg(long = "dst")]
    dst: Option<Ipv4Addr>,

    /// Print pcap per-packet header line
    #[arg(long = "header", default_value_t = true)]
    header: bool,

    /// Stop after N dumped packets (debugging)
    #[arg(long = "limit")]
    limit: Option<usize>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let mut cap = Capture::from_file(&args.pcap)
        .with_context(|| format!("pcap open failed: {}", args.pcap.display()))?;

    let mut dumped = 0usize;
    while let Ok(pkt) = cap.next_packet() {
        let Some(udp) = parse_udp_ipv4(pkt.data) else {
            continue;
        };

        if !passes_filters(&args, &udp) {
            continue;
        }

        if args.header {
            println!(
                "=== ts={}.{:06} caplen={} len={} {}:{} -> {}:{} (payload={})",
                pkt.header.ts.tv_sec,
                pkt.header.ts.tv_usec,
                pkt.header.caplen,
                pkt.header.len,
                udp.src_ip,
                udp.src_port,
                udp.dst_ip,
                udp.dst_port,
                udp.payload.len()
            );
        }

        dump_cme_packet(udp.payload);
        dumped += 1;

        if let Some(limit) = args.limit
            && dumped >= limit
        {
            break;
        }
    }

    Ok(())
}

fn passes_filters(args: &Args, u: &UdpDatagram<'_>) -> bool {
    if let Some(p) = args.src_port
        && u.src_port != p
    {
        return false;
    }
    if let Some(p) = args.dst_port
        && u.dst_port != p
    {
        return false;
    }
    if let Some(p) = args.udp_port
        && u.src_port != p
        && u.dst_port != p
    {
        return false;
    }
    if let Some(ip) = args.src
        && u.src_ip != ip
    {
        return false;
    }
    if let Some(ip) = args.dst
        && u.dst_ip != ip
    {
        return false;
    }
    true
}

struct UdpDatagram<'a> {
    src_ip: Ipv4Addr,
    dst_ip: Ipv4Addr,
    src_port: u16,
    dst_port: u16,
    payload: &'a [u8],
}

/// Minimal Ethernet + IPv4 + UDP extractor (no VLAN, no IPv6).
fn parse_udp_ipv4(frame: &[u8]) -> Option<UdpDatagram<'_>> {
    if frame.len() < 14 {
        return None;
    }
    let eth_type = u16::from_be_bytes([frame[12], frame[13]]);
    if eth_type != 0x0800 {
        return None;
    }

    let ip = 14;
    if frame.len() < ip + 20 {
        return None;
    }
    let ver_ihl = frame[ip];
    if (ver_ihl >> 4) != 4 {
        return None;
    }
    let ihl = (ver_ihl & 0x0f) as usize * 4;
    if ihl < 20 || frame.len() < ip + ihl {
        return None;
    }

    if frame[ip + 9] != 17 {
        return None;
    }

    let src_ip = Ipv4Addr::new(
        frame[ip + 12],
        frame[ip + 13],
        frame[ip + 14],
        frame[ip + 15],
    );
    let dst_ip = Ipv4Addr::new(
        frame[ip + 16],
        frame[ip + 17],
        frame[ip + 18],
        frame[ip + 19],
    );

    let udp = ip + ihl;
    if frame.len() < udp + 8 {
        return None;
    }

    let src_port = u16::from_be_bytes([frame[udp], frame[udp + 1]]);
    let dst_port = u16::from_be_bytes([frame[udp + 2], frame[udp + 3]]);
    let udp_len = u16::from_be_bytes([frame[udp + 4], frame[udp + 5]]) as usize;
    if udp_len < 8 {
        return None;
    }

    let payload_start = udp + 8;
    let payload_len = udp_len - 8;
    if frame.len() < payload_start + payload_len {
        return None;
    }

    Some(UdpDatagram {
        src_ip,
        dst_ip,
        src_port,
        dst_port,
        payload: &frame[payload_start..payload_start + payload_len],
    })
}

#[cfg(test)]
mod real_data_tests;

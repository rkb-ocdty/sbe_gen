//! Example program demonstrating how to decode CME MDP multicast packets
//! using generated SBE types. The build script uses the local `sbe_gen`
//! crate to translate the XML schema under `schemas/` into Rust modules
//! under `src/generated/`.
use anyhow::{Context, Result};
use clap::Parser;
use pcap::Capture;
use std::mem;
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

pub mod message_header {
    pub use crate::generated::cme_mdp3::message_header::*;
}

use generated::cme_mdp3::channel_reset4 as channel_reset;
use generated::cme_mdp3::md_incremental_refresh_order_book47 as inc_book;
use generated::cme_mdp3::md_incremental_refresh_session_statistics51 as session_stats;
use generated::cme_mdp3::md_incremental_refresh_trade_summary48 as trade_summary;
use generated::cme_mdp3::security_status30 as security_status;
use generated::cme_mdp3::snapshot_full_refresh_order_book53 as snap_book;
use generated::cme_mdp3::snapshot_refresh_top_orders59 as top_orders;
use generated::cme_mdp3::types::{PRICE9, PRICENULL9};
use generated::cme_mdp3::MessageHeader;
use zerocopy::byteorder::little_endian::{U16, U32, U64};
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout, Ref, Unaligned};

/// Packet header used by the CME MDP feed. The sequence number and
/// sending time are present at the start of each UDP packet. These
/// two fields are little-endian, 4-byte and 8-byte integers
/// respectively. We derive the zerocopy traits so that the header
/// can be cast from a byte slice without any copying.
#[repr(C)]
#[derive(Debug, FromBytes, IntoBytes, KnownLayout, Immutable, Unaligned, Clone, Copy)]
pub struct CmePacketHdr {
    pub seq: U32,
    pub sending_time: U64,
}

impl CmePacketHdr {
    zc_parse_prefix!();
}

#[repr(C)]
#[derive(Debug, FromBytes, IntoBytes, KnownLayout, Immutable, Unaligned, Clone, Copy)]
pub struct CmeMessageHeader {
    msg_len: U16,
    sbe_hdr: MessageHeader,
}

impl CmeMessageHeader {
    zc_parse_prefix!();
}

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

        if let Some(limit) = args.limit {
            if dumped >= limit {
                break;
            }
        }
    }

    Ok(())
}

fn passes_filters(args: &Args, u: &UdpDatagram<'_>) -> bool {
    if let Some(p) = args.src_port {
        if u.src_port != p {
            return false;
        }
    }
    if let Some(p) = args.dst_port {
        if u.dst_port != p {
            return false;
        }
    }
    if let Some(p) = args.udp_port {
        if u.src_port != p && u.dst_port != p {
            return false;
        }
    }
    if let Some(ip) = args.src {
        if u.src_ip != ip {
            return false;
        }
    }
    if let Some(ip) = args.dst {
        if u.dst_ip != ip {
            return false;
        }
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

fn dump_cme_packet(packet: &[u8]) {
    let (pkt_hdr, mut rest) = match CmePacketHdr::parse_prefix(packet) {
        Some((hdr, r)) => (hdr, r),
        None => {
            eprintln!("Packet too short for CME header");
            return;
        }
    };

    println!(
        "CME packet seq={} sending_time={} bytes={}",
        pkt_hdr.seq.get(),
        pkt_hdr.sending_time.get(),
        packet.len()
    );

    const HEADER_LEN: usize = mem::size_of::<CmeMessageHeader>();
    let mut msg_index = 0usize;
    while rest.len() >= HEADER_LEN {
        let Some((cme_hdr, tail)) = CmeMessageHeader::parse_prefix(rest) else {
            break;
        };

        let msg_len = cme_hdr.msg_len.get() as usize;
        if msg_len < HEADER_LEN {
            eprintln!("  message {} has invalid length {}", msg_index, msg_len);
            break;
        }
        let body_len = msg_len - HEADER_LEN;
        if tail.len() < body_len {
            eprintln!(
                "  message {} truncated: need {} bytes, have {}",
                msg_index,
                body_len,
                tail.len()
            );
            break;
        }

        let (body, remainder) = tail.split_at(body_len);
        let msg_hdr = &cme_hdr.sbe_hdr;
        let template_id = msg_hdr.template_id.get();

        println!(
            "message {}: template={} block_length={} schema={} version={}",
            msg_index,
            template_id,
            msg_hdr.block_length.get(),
            msg_hdr.schema_id.get(),
            msg_hdr.version.get()
        );

        match template_id {
            4 => dump_channel_reset(msg_hdr, body),
            12 => dump_admin_heartbeat(),
            30 => dump_security_status(msg_hdr, body),
            47 => dump_incremental_order_book(msg_hdr, body),
            48 => dump_trade_summary(msg_hdr, body),
            51 => dump_session_statistics(msg_hdr, body),
            53 => dump_snapshot_order_book(msg_hdr, body),
            59 => dump_top_orders_snapshot(msg_hdr, body),
            _ => {
                println!("  template {} not handled", template_id);
            }
        }

        rest = remainder;
        msg_index += 1;
    }
}

fn dump_channel_reset(msg_hdr: &MessageHeader, body: &[u8]) {
    let Some((view, after_fixed)) = channel_reset::parse_with_header(body, msg_hdr) else {
        eprintln!("  ChannelReset truncated");
        return;
    };
    let Some(entries) = channel_reset::parse_no_md_entries(after_fixed) else {
        eprintln!("  ChannelReset missing NoMDEntries group");
        return;
    };

    println!(
        "  ChannelReset: transact_time={} mei={} entries={}",
        view.body.transact_time.get(),
        view.body.match_event_indicator.0,
        entries.count()
    );

    let mut iter = entries.iter();
    for idx in 0..entries.count() {
        let Some(entry) = iter.next() else {
            eprintln!("    entry {} truncated", idx);
            break;
        };
        println!(
            "    entry {} appl_id={} action={} entry_type={}",
            idx,
            entry.body.appl_id.get(),
            entry.body.md_update_action.0,
            entry.body.md_entry_type.0
        );
    }
}

fn dump_admin_heartbeat() {
    println!("  AdminHeartbeat");
}

fn dump_security_status(msg_hdr: &MessageHeader, body: &[u8]) {
    let Some((view, _)) = security_status::parse_with_header(body, msg_hdr) else {
        eprintln!("  SecurityStatus30 truncated");
        return;
    };
    let msg = &view.body;
    println!(
        "  SecurityStatus30: transact_time={} security_group={} asset={} security_id={:?} trade_date={} mei={} status={} halt={} event={}",
        msg.transact_time.get(),
        decode_ascii(&msg.security_group),
        decode_ascii(&msg.asset),
        decode_i32_null(msg.security_id.get()),
        msg.trade_date.get(),
        msg.match_event_indicator.0,
        msg.security_trading_status.0,
        msg.halt_reason.0,
        msg.security_trading_event.0
    );
}

fn dump_incremental_order_book(msg_hdr: &MessageHeader, body: &[u8]) {
    let Some((view, after_fixed)) = inc_book::parse_with_header(body, msg_hdr) else {
        eprintln!("  template 47 truncated");
        return;
    };
    let Some(entries) = inc_book::parse_no_md_entries(after_fixed) else {
        eprintln!("  template 47 missing NoMDEntries group");
        return;
    };

    println!(
        "  MDIncrementalRefreshOrderBook47: transact_time={} mei={} entries={}",
        view.body.transact_time.get(),
        view.body.match_event_indicator.0,
        entries.count()
    );

    let mut iter = entries.iter();
    for idx in 0..entries.count() {
        let Some(entry) = iter.next() else {
            eprintln!("    entry {} truncated", idx);
            break;
        };
        let body = entry.body;
        println!(
            "    [{}] order_id={:?} priority={:?} price={:?} qty={:?} sec_id={} action={} entry_type={}",
            idx,
            decode_u64_null(body.order_id.get()),
            decode_u64_null(body.md_order_priority.get()),
            decode_price_null(&body.md_entry_px),
            decode_i32_null(body.md_display_qty.get()),
            body.security_id.get(),
            body.md_update_action.0,
            body.md_entry_type.0
        );
    }
}

fn dump_trade_summary(msg_hdr: &MessageHeader, body: &[u8]) {
    let Some((view, after_fixed)) = trade_summary::parse_with_header(body, msg_hdr) else {
        eprintln!("  template 48 truncated");
        return;
    };
    let Some(entries) = trade_summary::parse_no_md_entries(after_fixed) else {
        eprintln!("  template 48 missing NoMDEntries group");
        return;
    };

    println!(
        "  MDIncrementalRefreshTradeSummary48: transact_time={} mei={} entries={}",
        view.body.transact_time.get(),
        view.body.match_event_indicator.0,
        entries.count()
    );

    let mut iter = entries.iter();
    let mut truncated = false;
    for idx in 0..entries.count() {
        let Some(entry) = iter.next() else {
            eprintln!("    entry {} truncated", idx);
            truncated = true;
            break;
        };
        let body = entry.body;
        println!(
            "    [{}] price={} size={} sec_id={} rpt_seq={} orders={} aggressor={} action={} entry_type={} trade_entry_id={:?}",
            idx,
            decode_price(&body.md_entry_px),
            body.md_entry_size.get(),
            body.security_id.get(),
            body.rpt_seq.get(),
            body.number_of_orders.get(),
            body.aggressor_side.0,
            body.md_update_action.0,
            body.md_entry_type.0,
            decode_u32_null(body.md_trade_entry_id.get())
        );
    }
    if truncated {
        return;
    }

    let after_entries = iter.remainder();
    let Some(order_entries) = trade_summary::parse_no_order_id_entries(after_entries) else {
        eprintln!("  template 48 missing NoOrderIDEntries group");
        return;
    };

    let mut order_iter = order_entries.iter();
    for idx in 0..order_entries.count() {
        let Some(entry) = order_iter.next() else {
            eprintln!("    order entry {} truncated", idx);
            break;
        };
        let body = entry.body;
        println!(
            "    order[{}] order_id={} last_qty={}",
            idx,
            body.order_id.get(),
            body.last_qty.get()
        );
    }
}

fn dump_session_statistics(msg_hdr: &MessageHeader, body: &[u8]) {
    let Some((view, after_fixed)) = session_stats::parse_with_header(body, msg_hdr) else {
        eprintln!("  template 51 truncated");
        return;
    };
    let Some(entries) = session_stats::parse_no_md_entries(after_fixed) else {
        eprintln!("  template 51 missing NoMDEntries group");
        return;
    };

    println!(
        "  MDIncrementalRefreshSessionStatistics51: transact_time={} mei={} entries={}",
        view.body.transact_time.get(),
        view.body.match_event_indicator.0,
        entries.count()
    );

    let mut iter = entries.iter();
    for idx in 0..entries.count() {
        let Some(entry) = iter.next() else {
            eprintln!("    entry {} truncated", idx);
            break;
        };
        let body = entry.body;
        println!(
            "    [{}] price={} sec_id={} rpt_seq={} flag={} action={} entry_type={} qty={:?}",
            idx,
            decode_price(&body.md_entry_px),
            body.security_id.get(),
            body.rpt_seq.get(),
            body.open_close_settl_flag.0,
            body.md_update_action.0,
            body.md_entry_type.0,
            decode_i32_null(body.md_entry_size.get())
        );
    }
}

fn dump_snapshot_order_book(msg_hdr: &MessageHeader, body: &[u8]) {
    let Some((view, after_fixed)) = snap_book::parse_with_header(body, msg_hdr) else {
        eprintln!("  template 53 truncated");
        return;
    };
    let Some(entries) = snap_book::parse_no_md_entries(after_fixed) else {
        eprintln!("  template 53 missing NoMDEntries group");
        return;
    };

    println!(
        "  SnapshotFullRefreshOrderBook53: sec_id={} chunk {}/{} last_seq={} transact_time={}",
        view.body.security_id.get(),
        view.body.current_chunk.get(),
        view.body.no_chunks.get(),
        view.body.last_msg_seq_num_processed.get(),
        view.body.transact_time.get()
    );

    let mut iter = entries.iter();
    for idx in 0..entries.count() {
        let Some(entry) = iter.next() else {
            eprintln!("    entry {} truncated", idx);
            break;
        };
        let body = entry.body;
        println!(
            "    [{}] order_id={} priority={:?} price={} qty={} entry_type={}",
            idx,
            body.order_id.get(),
            decode_u64_null(body.md_order_priority.get()),
            decode_price(&body.md_entry_px),
            body.md_display_qty.get(),
            body.md_entry_type.0
        );
    }
}

fn dump_top_orders_snapshot(msg_hdr: &MessageHeader, body: &[u8]) {
    let Some((view, after_fixed)) = top_orders::parse_with_header(body, msg_hdr) else {
        eprintln!("  template 59 truncated");
        return;
    };
    let Some(entries) = top_orders::parse_no_md_entries(after_fixed) else {
        eprintln!("  template 59 missing NoMDEntries group");
        return;
    };

    println!(
        "  SnapshotRefreshTopOrders59: sec_id={} transact_time={} mei={} entries={}",
        view.body.security_id.get(),
        view.body.transact_time.get(),
        view.body.match_event_indicator.0,
        entries.count()
    );

    let mut iter = entries.iter();
    for idx in 0..entries.count() {
        let Some(entry) = iter.next() else {
            eprintln!("    entry {} truncated", idx);
            break;
        };
        let body = entry.body;
        println!(
            "    [{}] order_id={} priority={} price={} qty={} entry_type={}",
            idx,
            body.order_id.get(),
            body.md_order_priority.get(),
            decode_price(&body.md_entry_px),
            body.md_display_qty.get(),
            body.md_entry_type.0
        );
    }
}

fn decode_u64_null(raw: u64) -> Option<u64> {
    if raw == u64::MAX {
        None
    } else {
        Some(raw)
    }
}

fn decode_u32_null(raw: u32) -> Option<u32> {
    if raw == u32::MAX {
        None
    } else {
        Some(raw)
    }
}

fn decode_i32_null(value: i32) -> Option<i32> {
    if value == i32::MAX {
        None
    } else {
        Some(value)
    }
}

fn decode_price(price: &PRICE9) -> f64 {
    let m = price.mantissa.get();
    let e = PRICE9::EXPONENT as i32;
    (m as f64) * 10f64.powi(e)
}

fn decode_price_null(price: &PRICENULL9) -> Option<f64> {
    let null_val: i64 = 9_223_372_036_854_775_807;
    let m = price.mantissa.get();
    if m == null_val {
        return None;
    }
    let e = PRICENULL9::EXPONENT as i32;
    Some((m as f64) * 10f64.powi(e))
}

fn decode_ascii(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end])
        .trim_end()
        .to_string()
}

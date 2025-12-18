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

pub mod message_header {
    pub use crate::generated::cme_mdp3::message_header::*;
}

use std::mem;

use generated::cme_mdp3::channel_reset4 as mbo_reset;
use generated::cme_mdp3::md_incremental_refresh_order_book47 as mbo_inc;
use generated::cme_mdp3::md_incremental_refresh_session_statistics51 as sess_stats;
use generated::cme_mdp3::md_incremental_refresh_trade_summary48 as trd;
use generated::cme_mdp3::security_status30 as sec_status;
use generated::cme_mdp3::snapshot_full_refresh_order_book53 as mbo_snap;
use generated::cme_mdp3::snapshot_refresh_top_orders59 as mbo_top;
use generated::cme_mdp3::types::{
    groupSize, groupSize8Byte, AggressorSide, HaltReason, MDEntryTypeBook,
    MDEntryTypeStatistics, MDEntryTypeTrade, MDUpdateAction, MatchEventIndicator,
    OpenCloseSettlFlag, SecurityTradingEvent, SecurityTradingStatus, PRICE9, PRICENULL9,
};
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

        cme_mbo_dump(udp.payload);
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
    // Ethernet header
    if frame.len() < 14 {
        return None;
    }
    let eth_type = u16::from_be_bytes([frame[12], frame[13]]);
    if eth_type != 0x0800 {
        return None;
    } // IPv4 only

    // IPv4 header start
    let ip = 14;
    if frame.len() < ip + 20 {
        return None;
    }
    let ver_ihl = frame[ip];
    if (ver_ihl >> 4) != 4 {
        return None;
    }
    let ihl = (ver_ihl & 0x0f) as usize * 4;
    if ihl < 20 {
        return None;
    }
    if frame.len() < ip + ihl {
        return None;
    }

    let proto = frame[ip + 9];
    if proto != 17 {
        return None;
    } // UDP

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

    // UDP header
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

    // Payload bounds: UDP length includes header.
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

/// Dump an entire CME MDP3 Market-by-Order multicast packet. The packet
/// starts with a `CmePacketHdr` followed by one or more SBE messages. This
/// helper understands the MBO templates commonly present on the channel:
/// SecurityStatus (30), ChannelReset (4), AdminHeartbeat (12),
/// Incremental Refresh Order Book (47), Incremental Refresh Trade Summary (48),
/// Session Statistics (51), Snapshot Full Refresh Order Book (53), and
/// Snapshot Refresh Top Orders (59).
pub fn cme_mbo_dump(packet: &[u8]) {
    let (pkt_hdr, mut rest) = match CmePacketHdr::parse_prefix(packet) {
        Some((hdr, r)) => (hdr, r),
        None => {
            eprintln!("Packet too short for CME header");
            return;
        }
    };
    let seq = pkt_hdr.seq.get();
    let st = pkt_hdr.sending_time.get();

    println!(
        "CME packet seq={} sending_time={} bytes={}",
        seq,
        st,
        packet.len()
    );

    const HEADER_LEN: usize = mem::size_of::<CmeMessageHeader>();
    let mut msg_index = 0usize;
    while rest.len() >= mem::size_of::<CmeMessageHeader>() {
        let Some((cme_hdr, tail)) = CmeMessageHeader::parse_prefix(rest) else {
            break;
        };
        let msg_hdr = &cme_hdr.sbe_hdr;
        let blen = msg_hdr.block_length.get();
        let template_id = msg_hdr.template_id.get();
        let sid = msg_hdr.schema_id.get();
        let ver = msg_hdr.version.get();

        println!(
            "message {}: template={} block_length={} schema={} version={}",
            msg_index,
            template_id,
            blen,
            sid,
            ver
        );
        let msg_len = cme_hdr.msg_len.get() as usize;
        let body_len = msg_len.saturating_sub(HEADER_LEN);
        if tail.len() < body_len {
            eprintln!(
                "  template {} truncated: need {} bytes, have {}",
                template_id,
                body_len,
                tail.len()
            );
            break;
        }
        let (body, remainder) = tail.split_at(body_len);

        let _next = match template_id {
            30 => dump_security_status(msg_hdr, body),
            4 => dump_channel_reset(msg_hdr, body),
            12 => dump_admin_heartbeat(msg_hdr, body),
            47 => dump_incremental_order_book(msg_hdr, body),
            48 => dump_trade_summary(msg_hdr, body),
            51 => dump_session_statistics(msg_hdr, body),
            53 => dump_snapshot_order_book(msg_hdr, body),
            59 => dump_top_orders_snapshot(msg_hdr, body),
            _ => {
                println!(
                    "  template {} not handled; skipping fixed block {} bytes",
                    template_id,
                    msg_hdr.block_length.get()
                );
                advance(body, msg_hdr.block_length.get() as usize, template_id)
            }
        };

        rest = remainder;
        msg_index += 1;
    }
}

/// Decode a `ChannelReset` message and advance past its payload. The reset
/// contains only the fixed header plus a small `NoMDEntries` group; we print
/// the basic header fields and the group count for visibility.
fn dump_channel_reset<'a>(msg_hdr: &MessageHeader, body: &'a [u8]) -> Option<&'a [u8]> {
    let fixed_len = msg_hdr.block_length.get() as usize;
    if body.len() < fixed_len || fixed_len < 9 {
        eprintln!(
            "  channel reset truncated: need {} bytes of fixed data, have {}",
            fixed_len,
            body.len()
        );
        return None;
    }

    let (ts, _) = Ref::<_, U64>::from_prefix(body).ok()?;
    let mei = MatchEventIndicator(body[8]);
    let group_region = &body[fixed_len..];
    let (header, payload) = Ref::<_, groupSize>::from_prefix(group_region).ok()?;
    let header = Ref::into_ref(header);
    let (entry_block_len, group_bytes) = group_section_bytes(
        header,
        mbo_reset::NoMDEntriesGroupBuilder::BLOCK_LENGTH as usize,
    )?;
    if group_region.len() < group_bytes {
        eprintln!(
            "  channel reset group truncated: need {} bytes, have {}",
            group_bytes,
            group_region.len()
        );
        return None;
    }

    println!(
        "  ChannelReset: transact_time={} mei={} entries={}",
        ts.get(),
        format_match_event_indicator(mei),
        header.num_in_group
    );

    // ApplID lives at offset 0 in the entry; dump it when present.
    let mut remaining = &payload[..group_bytes - mem::size_of::<groupSize>()];
    for idx in 0..header.num_in_group as usize {
        if remaining.len() < entry_block_len {
            eprintln!("    entry {} truncated", idx);
            break;
        }
        let (appl_id, _) =
            Ref::<_, zerocopy::byteorder::little_endian::I16>::from_prefix(remaining).ok()?;
        println!("    entry {} appl_id={}", idx, appl_id.get());
        remaining = &remaining[entry_block_len..];
    }

    advance(body, fixed_len + group_bytes, msg_hdr.template_id.get())
}

fn dump_admin_heartbeat<'a>(msg_hdr: &MessageHeader, body: &'a [u8]) -> Option<&'a [u8]> {
    println!("  AdminHeartbeat");
    advance(
        body,
        msg_hdr.block_length.get() as usize,
        msg_hdr.template_id.get(),
    )
}

fn dump_incremental_order_book<'a>(
    msg_hdr: &MessageHeader,
    body: &'a [u8],
) -> Option<&'a [u8]> {
    let (view, after_fixed) = mbo_inc::parse_with_header(body, msg_hdr)?;
    let entries = mbo_inc::parse_no_md_entries(after_fixed)?;
    let (_entry_block_len, group_bytes) = group_section_bytes(
        entries.header,
        mbo_inc::NoMDEntriesGroupBuilder::BLOCK_LENGTH as usize,
    )?;
    if after_fixed.len() < group_bytes {
        eprintln!(
            "  template 47 group truncated: need {} bytes, have {}",
            group_bytes,
            after_fixed.len()
        );
        return None;
    }

    println!(
        "  MDIncrementalRefreshOrderBook47: transact_time={} mei={} entries={} (block_len={})",
        view.body.transact_time.get(),
        format_match_event_indicator(view.body.match_event_indicator),
        entries.header.num_in_group,
        _entry_block_len
    );

    let mut iter = entries.iter();
    let mut seen = 0usize;
    while seen < entries.count() {
        if let Some(entry) = iter.next() {
            let body = entry.body;
            let order_id = decode_u64_null(body.order_id.get());
            let priority = decode_u64_null(body.md_order_priority.get());
            let price = decode_price_null(&body.md_entry_px);
            let qty = decode_i32_null(body.md_display_qty.get());
            let sec_id = body.security_id.get();
            let action = describe_update_action(body.md_update_action);
            let entry_type = describe_entry_type_book(body.md_entry_type);
            println!(
                "    [{}] order_id={:?} priority={:?} price={:?} qty={:?} sec_id={} action={} entry_type={}",
                seen, order_id, priority, price, qty, sec_id, action, entry_type
            );
            seen += 1;
        } else {
            eprintln!(
                "    truncated NoMDEntries: expected {} entries, saw {}",
                entries.count(),
                seen
            );
            break;
        }
    }

    let total = view.acting_block_length + group_bytes;
    advance(body, total, msg_hdr.template_id.get())
}

fn dump_security_status<'a>(msg_hdr: &MessageHeader, body: &'a [u8]) -> Option<&'a [u8]> {
    let (view, _) = sec_status::parse_with_header(body, msg_hdr)?;
    let msg = &view.body;
    let security_group = decode_ascii(&msg.security_group);
    let asset = decode_ascii(&msg.asset);
    let security_id = decode_i32_null(msg.security_id.get());

    println!(
        "  SecurityStatus30: transact_time={} security_group={} asset={} security_id={:?} trade_date={} mei={} status={} halt={} event={}",
        msg.transact_time.get(),
        security_group,
        asset,
        security_id,
        msg.trade_date.get(),
        format_match_event_indicator(msg.match_event_indicator),
        describe_security_trading_status(msg.security_trading_status),
        describe_halt_reason(msg.halt_reason),
        describe_security_trading_event(msg.security_trading_event)
    );

    advance(body, view.acting_block_length, msg_hdr.template_id.get())
}

fn dump_trade_summary<'a>(msg_hdr: &MessageHeader, body: &'a [u8]) -> Option<&'a [u8]> {
    let (view, after_fixed) = trd::parse_with_header(body, msg_hdr)?;
    let entries = trd::parse_no_md_entries(after_fixed)?;
    let (_entry_block_len, group_bytes) = group_section_bytes(
        entries.header,
        trd::NoMDEntriesGroupBuilder::BLOCK_LENGTH as usize,
    )?;
    if after_fixed.len() < group_bytes {
        eprintln!(
            "  template 48 group truncated: need {} bytes, have {}",
            group_bytes,
            after_fixed.len()
        );
        return None;
    }

    let after_entries = &after_fixed[group_bytes..];
    let order_entries = trd::parse_no_order_id_entries(after_entries)?;
    let (_order_block_len, order_group_bytes) = group_section_bytes_8(
        order_entries.header,
        trd::NoOrderIDEntriesGroupBuilder::BLOCK_LENGTH as usize,
    )?;
    if after_entries.len() < order_group_bytes {
        eprintln!(
            "  template 48 order group truncated: need {} bytes, have {}",
            order_group_bytes,
            after_entries.len()
        );
        return None;
    }

    println!(
        "  MDIncrementalRefreshTradeSummary48: transact_time={} mei={} entries={} order_entries={}",
        view.body.transact_time.get(),
        format_match_event_indicator(view.body.match_event_indicator),
        entries.header.num_in_group,
        order_entries.header.num_in_group
    );

    let mut iter = entries.iter();
    let mut seen = 0usize;
    while seen < entries.count() {
        if let Some(entry) = iter.next() {
            let body = entry.body;
            let price = decode_price(&body.md_entry_px);
            let size = body.md_entry_size.get();
            let sec_id = body.security_id.get();
            let rpt_seq = body.rpt_seq.get();
            let orders = body.number_of_orders.get();
            let aggressor = describe_aggressor_side(body.aggressor_side);
            let action = describe_update_action(body.md_update_action);
            let entry_type = describe_entry_type_trade(body.md_entry_type);
            let trade_entry_id = decode_u32_null(body.md_trade_entry_id.get());
            println!(
                "    [{}] price={} size={} sec_id={} rpt_seq={} orders={} aggressor={} action={} entry_type={} trade_entry_id={:?}",
                seen, price, size, sec_id, rpt_seq, orders, aggressor, action, entry_type, trade_entry_id
            );
            seen += 1;
        } else {
            eprintln!(
                "    truncated NoMDEntries: expected {} entries, saw {}",
                entries.count(),
                seen
            );
            break;
        }
    }

    let mut order_iter = order_entries.iter();
    let mut order_seen = 0usize;
    while order_seen < order_entries.count() {
        if let Some(entry) = order_iter.next() {
            let body = entry.body;
            println!(
                "    order[{}] order_id={} last_qty={}",
                order_seen,
                body.order_id.get(),
                body.last_qty.get()
            );
            order_seen += 1;
        } else {
            eprintln!(
                "    truncated NoOrderIDEntries: expected {} entries, saw {}",
                order_entries.count(),
                order_seen
            );
            break;
        }
    }

    let total = view.acting_block_length + group_bytes + order_group_bytes;
    advance(body, total, msg_hdr.template_id.get())
}

fn dump_session_statistics<'a>(
    msg_hdr: &MessageHeader,
    body: &'a [u8],
) -> Option<&'a [u8]> {
    let (view, after_fixed) = sess_stats::parse_with_header(body, msg_hdr)?;
    let entries = sess_stats::parse_no_md_entries(after_fixed)?;
    let (_entry_block_len, group_bytes) = group_section_bytes(
        entries.header,
        sess_stats::NoMDEntriesGroupBuilder::BLOCK_LENGTH as usize,
    )?;
    if after_fixed.len() < group_bytes {
        eprintln!(
            "  template 51 group truncated: need {} bytes, have {}",
            group_bytes,
            after_fixed.len()
        );
        return None;
    }

    println!(
        "  MDIncrementalRefreshSessionStatistics51: transact_time={} mei={} entries={} (block_len={})",
        view.body.transact_time.get(),
        format_match_event_indicator(view.body.match_event_indicator),
        entries.header.num_in_group,
        _entry_block_len
    );

    let mut iter = entries.iter();
    let mut seen = 0usize;
    while seen < entries.count() {
        if let Some(entry) = iter.next() {
            let body = entry.body;
            let price = decode_price(&body.md_entry_px);
            let sec_id = body.security_id.get();
            let rpt_seq = body.rpt_seq.get();
            let flag = describe_open_close_settl_flag(body.open_close_settl_flag);
            let action = describe_update_action(body.md_update_action);
            let entry_type = describe_entry_type_statistics(body.md_entry_type);
            let qty = decode_i32_null(body.md_entry_size.get());
            println!(
                "    [{}] price={} sec_id={} rpt_seq={} flag={} action={} entry_type={} qty={:?}",
                seen, price, sec_id, rpt_seq, flag, action, entry_type, qty
            );
            seen += 1;
        } else {
            eprintln!(
                "    truncated NoMDEntries: expected {} entries, saw {}",
                entries.count(),
                seen
            );
            break;
        }
    }

    let total = view.acting_block_length + group_bytes;
    advance(body, total, msg_hdr.template_id.get())
}

fn dump_snapshot_order_book<'a>(
    msg_hdr: &MessageHeader,
    body: &'a [u8],
) -> Option<&'a [u8]> {
    let (view, after_fixed) = mbo_snap::parse_with_header(body, msg_hdr)?;
    let entries = mbo_snap::parse_no_md_entries(after_fixed)?;
    let (_entry_block_len, group_bytes) = group_section_bytes(
        entries.header,
        mbo_snap::NoMDEntriesGroupBuilder::BLOCK_LENGTH as usize,
    )?;
    if after_fixed.len() < group_bytes {
        eprintln!(
            "  template 53 group truncated: need {} bytes, have {}",
            group_bytes,
            after_fixed.len()
        );
        return None;
    }

    println!(
        "  SnapshotFullRefreshOrderBook53: sec_id={} chunk {}/{} last_seq={} transact_time={}",
        view.body.security_id.get(),
        view.body.current_chunk.get(),
        view.body.no_chunks.get(),
        view.body.last_msg_seq_num_processed.get(),
        view.body.transact_time.get()
    );

    let mut iter = entries.iter();
    let mut seen = 0usize;
    while seen < entries.count() {
        if let Some(entry) = iter.next() {
            let body = entry.body;
            let order_id = body.order_id.get();
            let priority = decode_u64_null(body.md_order_priority.get());
            let price = decode_price(&body.md_entry_px);
            let qty = body.md_display_qty.get();
            let entry_type = describe_entry_type_book(body.md_entry_type);
            println!(
                "    [{}] order_id={} priority={:?} price={} qty={} entry_type={}",
                seen, order_id, priority, price, qty, entry_type
            );
            seen += 1;
        } else {
            eprintln!(
                "    truncated NoMDEntries: expected {} entries, saw {}",
                entries.count(),
                seen
            );
            break;
        }
    }

    let total = view.acting_block_length + group_bytes;
    advance(body, total, msg_hdr.template_id.get())
}

fn dump_top_orders_snapshot<'a>(
    msg_hdr: &MessageHeader,
    body: &'a [u8],
) -> Option<&'a [u8]> {
    let (view, after_fixed) = mbo_top::parse_with_header(body, msg_hdr)?;
    let entries = mbo_top::parse_no_md_entries(after_fixed)?;
    let (_entry_block_len, group_bytes) = group_section_bytes(
        entries.header,
        mbo_top::NoMDEntriesGroupBuilder::BLOCK_LENGTH as usize,
    )?;
    if after_fixed.len() < group_bytes {
        eprintln!(
            "  template 59 group truncated: need {} bytes, have {}",
            group_bytes,
            after_fixed.len()
        );
        return None;
    }

    println!(
        "  SnapshotRefreshTopOrders59: sec_id={} transact_time={} mei={} entries={} (block_len={})",
        view.body.security_id.get(),
        view.body.transact_time.get(),
        format_match_event_indicator(view.body.match_event_indicator),
        entries.header.num_in_group,
        _entry_block_len
    );

    let mut iter = entries.iter();
    let mut seen = 0usize;
    while seen < entries.count() {
        if let Some(entry) = iter.next() {
            let body = entry.body;
            let order_id = body.order_id.get();
            let priority = body.md_order_priority.get();
            let price = decode_price(&body.md_entry_px);
            let qty = body.md_display_qty.get();
            let entry_type = describe_entry_type_book(body.md_entry_type);
            println!(
                "    [{}] order_id={} priority={} price={} qty={} entry_type={}",
                seen, order_id, priority, price, qty, entry_type
            );
            seen += 1;
        } else {
            eprintln!(
                "    truncated NoMDEntries: expected {} entries, saw {}",
                entries.count(),
                seen
            );
            break;
        }
    }

    let total = view.acting_block_length + group_bytes;
    advance(body, total, msg_hdr.template_id.get())
}

fn advance(body: &[u8], consumed: usize, template_id: u16) -> Option<&[u8]> {
    if body.len() < consumed {
        eprintln!(
            "  template {} truncated: need {} bytes, have {}",
            template_id,
            consumed,
            body.len()
        );
        None
    } else {
        Some(&body[consumed..])
    }
}

fn group_section_bytes(header: &groupSize, declared_block_len: usize) -> Option<(usize, usize)> {
    let block_len = resolve_group_block_len(header.block_length.get() as usize, declared_block_len);
    let count = header.num_in_group as usize;
    let entries_bytes = block_len.checked_mul(count)?;
    let header_bytes = mem::size_of::<groupSize>();
    let total = header_bytes.checked_add(entries_bytes)?;
    Some((block_len, total))
}

fn group_section_bytes_8(
    header: &groupSize8Byte,
    declared_block_len: usize,
) -> Option<(usize, usize)> {
    let block_len = resolve_group_block_len(header.block_length.get() as usize, declared_block_len);
    let count = header.num_in_group as usize;
    let entries_bytes = block_len.checked_mul(count)?;
    let header_bytes = mem::size_of::<groupSize8Byte>();
    let total = header_bytes.checked_add(entries_bytes)?;
    Some((block_len, total))
}

fn resolve_group_block_len(raw: usize, declared: usize) -> usize {
    if raw == 0 {
        declared
    } else {
        raw
    }
}

fn format_match_event_indicator(indicator: MatchEventIndicator) -> String {
    let raw = indicator.0;
    let flags = [
        (MatchEventIndicator::LastTradeMsg.0, "LastTrade"),
        (MatchEventIndicator::LastVolumeMsg.0, "LastVolume"),
        (MatchEventIndicator::LastQuoteMsg.0, "LastQuote"),
        (MatchEventIndicator::LastStatsMsg.0, "LastStats"),
        (MatchEventIndicator::LastImpliedMsg.0, "LastImplied"),
        (MatchEventIndicator::RecoveryMsg.0, "Recovery"),
        (MatchEventIndicator::EndOfEvent.0, "EndOfEvent"),
    ];

    let mut parts = Vec::new();
    for (mask, label) in flags {
        if raw & mask != 0 {
            parts.push(label);
        }
    }
    if parts.is_empty() {
        "0".to_string()
    } else {
        parts.join("|")
    }
}

fn describe_update_action(action: MDUpdateAction) -> String {
    match action.as_enum() {
        Some(enum_val) => format!("{enum_val:?}"),
        None => format!("Unknown({})", action.0),
    }
}

fn describe_halt_reason(reason: HaltReason) -> String {
    match reason.as_enum() {
        Some(enum_val) => format!("{enum_val:?}"),
        None => format!("Unknown({})", reason.0),
    }
}

fn describe_security_trading_status(status: SecurityTradingStatus) -> String {
    match status.as_enum() {
        Some(enum_val) => format!("{enum_val:?}"),
        None => format!("Unknown({})", status.0),
    }
}

fn describe_security_trading_event(event: SecurityTradingEvent) -> String {
    match event.as_enum() {
        Some(enum_val) => format!("{enum_val:?}"),
        None => format!("Unknown({})", event.0),
    }
}

fn describe_open_close_settl_flag(flag: OpenCloseSettlFlag) -> String {
    match flag.as_enum() {
        Some(enum_val) => format!("{enum_val:?}"),
        None => format!("Unknown({})", flag.0),
    }
}

fn describe_aggressor_side(side: AggressorSide) -> String {
    match side.as_enum() {
        Some(enum_val) => format!("{enum_val:?}"),
        None => format!("Unknown({})", side.0),
    }
}

fn describe_entry_type_book(entry_type: MDEntryTypeBook) -> String {
    match entry_type.as_enum() {
        Some(enum_val) => format!("{enum_val:?}"),
        None => format!("Unknown({})", entry_type.0),
    }
}

fn describe_entry_type_statistics(entry_type: MDEntryTypeStatistics) -> String {
    match entry_type.as_enum() {
        Some(enum_val) => format!("{enum_val:?}"),
        None => format!("Unknown({})", entry_type.0),
    }
}

fn describe_entry_type_trade(entry_type: MDEntryTypeTrade) -> String {
    match entry_type.as_enum() {
        Some(enum_val) => format!("{enum_val:?}"),
        None => format!("Unknown({})", entry_type.0),
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

fn decode_ascii(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    let trimmed = &bytes[..end];
    String::from_utf8_lossy(trimmed).trim_end().to_string()
}

/// Convenience wrapper kept for compatibility with earlier examples.
pub fn decode_cme_packet(packet: &[u8]) {
    cme_mbo_dump(packet);
}

/// Helper to convert a price composite into a floating point value. A
/// price composite consists of a `mantissa` (signed 64-bit integer)
/// and an `exponent` (signed 8-bit integer). The actual price is
/// computed as `mantissa * 10^exponent`.
fn decode_price(price: &PRICE9) -> f64 {
    let m = price.mantissa.get();
    let e = PRICE9::EXPONENT as i32;
    (m as f64) * 10f64.powi(e)
}

// Provide alias for the optional price composite used in NULLable
// fields. PRICENULL9 has the same layout as PRICE9 for the
// mantissa/exponent fields.
fn decode_price_null(price: &PRICENULL9) -> Option<f64> {
    let null_val: i64 = 9_223_372_036_854_775_807;
    let m = price.mantissa.get();
    if m == null_val {
        return None;
    }
    let e = PRICENULL9::EXPONENT as i32;
    Some((m as f64) * 10f64.powi(e))
}

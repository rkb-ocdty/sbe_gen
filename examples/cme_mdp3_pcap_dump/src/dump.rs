use crate::generated::cme_mdp3::MessageHeader;
use crate::generated::cme_mdp3::channel_reset4 as channel_reset;
use crate::generated::cme_mdp3::md_incremental_refresh_order_book47 as inc_book;
use crate::generated::cme_mdp3::md_incremental_refresh_session_statistics51 as session_stats;
use crate::generated::cme_mdp3::md_incremental_refresh_trade_summary48 as trade_summary;
use crate::generated::cme_mdp3::security_status30 as security_status;
use crate::generated::cme_mdp3::snapshot_full_refresh_order_book53 as snap_book;
use crate::generated::cme_mdp3::snapshot_refresh_top_orders59 as top_orders;
use crate::generated::cme_mdp3::types::{PRICE9, PRICENULL9};
use std::mem;
use zerocopy::byteorder::little_endian::{U16, U32, U64};
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout, Ref, Unaligned};

macro_rules! message_body_if_full {
    ($view:expr) => {
        if $view.is_fixed_layout() {
            Some(&*$view.body)
        } else {
            None
        }
    };
}

macro_rules! entry_body_if_full {
    ($entry:expr, $ty:path) => {
        if $entry.acting_block_length >= core::mem::size_of::<$ty>() {
            Some(&*$entry.body)
        } else {
            None
        }
    };
}

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
    #[inline]
    pub fn parse_prefix(body: &[u8]) -> Option<(&Self, &[u8])> {
        Ref::<_, Self>::from_prefix(body)
            .ok()
            .map(|(r, b)| (Ref::into_ref(r), b))
    }
}

#[repr(C)]
#[derive(Debug, FromBytes, IntoBytes, KnownLayout, Immutable, Unaligned, Clone, Copy)]
pub struct CmeMessageHeader {
    pub(crate) msg_len: U16,
    pub(crate) sbe_hdr: MessageHeader,
}

impl CmeMessageHeader {
    #[inline]
    pub fn parse_prefix(body: &[u8]) -> Option<(&Self, &[u8])> {
        Ref::<_, Self>::from_prefix(body)
            .ok()
            .map(|(r, b)| (Ref::into_ref(r), b))
    }
}

pub(crate) fn dump_cme_packet(packet: &[u8]) {
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

    let msg = message_body_if_full!(view);
    let transact_time = msg
        .map(|m| m.transact_time.get())
        .or_else(|| view.transact_time_value());
    println!(
        "  ChannelReset: transact_time={:?} mei={:?} entries={}",
        transact_time,
        msg.map(|m| m.match_event_indicator.0),
        entries.count()
    );

    let mut iter = entries.iter();
    for idx in 0..entries.count() {
        let Some(entry) = iter.next() else {
            eprintln!("    entry {} truncated", idx);
            break;
        };
        println!(
            "    entry {} appl_id={:?} action={} entry_type={}",
            idx,
            entry.appl_id().map(|v| v.get()),
            channel_reset::NoMDEntriesEntry::MD_UPDATE_ACTION.0,
            channel_reset::NoMDEntriesEntry::MD_ENTRY_TYPE.0
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
    let msg = message_body_if_full!(view);
    let security_group = msg
        .map(|m| decode_ascii(&m.security_group))
        .or_else(|| view.security_group_str_trimmed().map(str::to_string))
        .unwrap_or_else(|| "<N/A>".to_string());
    let asset = msg
        .map(|m| decode_ascii(&m.asset))
        .or_else(|| view.asset_str_trimmed().map(str::to_string))
        .unwrap_or_else(|| "<N/A>".to_string());
    let transact_time = msg
        .map(|m| m.transact_time.get())
        .or_else(|| view.transact_time_value());
    let security_id = msg
        .and_then(|m| decode_i32_null(m.security_id.get()))
        .or_else(|| view.security_id_value().flatten());
    let trade_date = msg
        .map(|m| m.trade_date.get())
        .or_else(|| view.trade_date_value().flatten());
    println!(
        "  SecurityStatus30: transact_time={:?} security_group={} asset={} security_id={:?} trade_date={:?} mei={:?} status={:?} halt={:?} event={:?}",
        transact_time,
        security_group,
        asset,
        security_id,
        trade_date,
        msg.map(|m| m.match_event_indicator.0),
        msg.map(|m| m.security_trading_status.0),
        msg.map(|m| m.halt_reason.0),
        msg.map(|m| m.security_trading_event.0)
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

    let msg = message_body_if_full!(view);
    let transact_time = msg
        .map(|m| m.transact_time.get())
        .or_else(|| view.transact_time_value());
    println!(
        "  MDIncrementalRefreshOrderBook47: transact_time={:?} mei={:?} entries={}",
        transact_time,
        msg.map(|m| m.match_event_indicator.0),
        entries.count()
    );

    let mut iter = entries.iter();
    for idx in 0..entries.count() {
        let Some(entry) = iter.next() else {
            eprintln!("    entry {} truncated", idx);
            break;
        };
        let body = entry_body_if_full!(entry, inc_book::NoMDEntriesEntry);
        println!(
            "    [{}] order_id={:?} priority={:?} price={:?} qty={:?} sec_id={:?} action=[{:?}] entry_type=[{:?}]",
            idx,
            entry.order_id().and_then(|v| decode_u64_null(v.get())),
            entry
                .md_order_priority()
                .and_then(|v| decode_u64_null(v.get())),
            entry.md_entry_px().and_then(decode_price_null),
            entry
                .md_display_qty()
                .and_then(|v| decode_i32_null(v.get())),
            entry.security_id().map(|v| v.get()),
            body.and_then(|e| e.md_update_action.as_enum()),
            body.and_then(|e| e.md_entry_type.as_enum()),
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

    let msg = message_body_if_full!(view);
    let transact_time = msg
        .map(|m| m.transact_time.get())
        .or_else(|| view.transact_time_value());
    println!(
        "  MDIncrementalRefreshTradeSummary48: transact_time={:?} mei={:?} entries={}",
        transact_time,
        msg.map(|m| m.match_event_indicator.0),
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
        let body = entry_body_if_full!(entry, trade_summary::NoMDEntriesEntry);
        println!(
            "    [{}] price={:?} size={:?} sec_id={:?} rpt_seq={:?} orders={:?} aggressor={:?} action={:?} entry_type={} trade_entry_id={:?}",
            idx,
            entry.md_entry_px().map(decode_price),
            entry.md_entry_size().map(|v| v.get()),
            entry.security_id().map(|v| v.get()),
            entry.rpt_seq().map(|v| v.get()),
            entry.number_of_orders().map(|v| v.get()),
            body.map(|e| e.aggressor_side.0),
            body.map(|e| e.md_update_action.0),
            trade_summary::NoMDEntriesEntry::MD_ENTRY_TYPE.0,
            entry
                .md_trade_entry_id()
                .and_then(|v| decode_u32_null(v.get()))
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
        println!(
            "    order[{}] order_id={:?} last_qty={:?}",
            idx,
            entry.order_id().map(|v| v.get()),
            entry.last_qty().map(|v| v.get())
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

    let msg = message_body_if_full!(view);
    let transact_time = msg
        .map(|m| m.transact_time.get())
        .or_else(|| view.transact_time_value());
    println!(
        "  MDIncrementalRefreshSessionStatistics51: transact_time={:?} mei={:?} entries={}",
        transact_time,
        msg.map(|m| m.match_event_indicator.0),
        entries.count()
    );

    let mut iter = entries.iter();
    for idx in 0..entries.count() {
        let Some(entry) = iter.next() else {
            eprintln!("    entry {} truncated", idx);
            break;
        };
        let body = entry_body_if_full!(entry, session_stats::NoMDEntriesEntry);
        println!(
            "    [{}] price={:?} sec_id={:?} rpt_seq={:?} flag={:?} action=[{:?}] entry_type=[{:?}] qty={:?}",
            idx,
            entry.md_entry_px().map(decode_price),
            entry.security_id().map(|v| v.get()),
            entry.rpt_seq().map(|v| v.get()),
            body.map(|e| e.open_close_settl_flag.0),
            body.and_then(|e| e.md_update_action.as_enum()),
            body.and_then(|e| e.md_entry_type.as_enum()),
            entry.md_entry_size().and_then(|v| decode_i32_null(v.get()))
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

    let msg = message_body_if_full!(view);
    let security_id = msg
        .map(|m| m.security_id.get())
        .or_else(|| view.security_id_value());
    let current_chunk = msg
        .map(|m| m.current_chunk.get())
        .or_else(|| view.current_chunk_value());
    let no_chunks = msg
        .map(|m| m.no_chunks.get())
        .or_else(|| view.no_chunks_value());
    let last_msg_seq_num_processed = msg
        .map(|m| m.last_msg_seq_num_processed.get())
        .or_else(|| view.last_msg_seq_num_processed_value());
    let transact_time = msg
        .map(|m| m.transact_time.get())
        .or_else(|| view.transact_time_value());
    println!(
        "  SnapshotFullRefreshOrderBook53: sec_id={:?} chunk {:?}/{:?} last_seq={:?} transact_time={:?}",
        security_id, current_chunk, no_chunks, last_msg_seq_num_processed, transact_time
    );

    let mut iter = entries.iter();
    for idx in 0..entries.count() {
        let Some(entry) = iter.next() else {
            eprintln!("    entry {} truncated", idx);
            break;
        };
        let body = entry_body_if_full!(entry, snap_book::NoMDEntriesEntry);
        println!(
            "    [{}] order_id={:?} priority={:?} price={:?} qty={:?} entry_type={:?}",
            idx,
            entry.order_id().map(|v| v.get()),
            entry
                .md_order_priority()
                .and_then(|v| decode_u64_null(v.get())),
            entry.md_entry_px().map(decode_price),
            entry.md_display_qty().map(|v| v.get()),
            body.map(|e| e.md_entry_type.0)
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

    let msg = message_body_if_full!(view);
    let security_id = msg
        .map(|m| m.security_id.get())
        .or_else(|| view.security_id_value());
    let transact_time = msg
        .map(|m| m.transact_time.get())
        .or_else(|| view.transact_time_value());
    println!(
        "  SnapshotRefreshTopOrders59: sec_id={:?} transact_time={:?} mei={:?} entries={}",
        security_id,
        transact_time,
        msg.map(|m| m.match_event_indicator.0),
        entries.count()
    );

    let mut iter = entries.iter();
    for idx in 0..entries.count() {
        let Some(entry) = iter.next() else {
            eprintln!("    entry {} truncated", idx);
            break;
        };
        let body = entry_body_if_full!(entry, top_orders::NoMDEntriesEntry);
        println!(
            "    [{}] order_id={:?} priority={:?} price={:?} qty={:?} entry_type={:?}",
            idx,
            entry.order_id().map(|v| v.get()),
            entry.md_order_priority().map(|v| v.get()),
            entry.md_entry_px().map(decode_price),
            entry.md_display_qty().map(|v| v.get()),
            body.map(|e| e.md_entry_type.0)
        );
    }
}

fn decode_u64_null(raw: u64) -> Option<u64> {
    if raw == u64::MAX { None } else { Some(raw) }
}

fn decode_u32_null(raw: u32) -> Option<u32> {
    if raw == u32::MAX { None } else { Some(raw) }
}

fn decode_i32_null(value: i32) -> Option<i32> {
    if value == i32::MAX { None } else { Some(value) }
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

use std::mem;

use crate::dump::{CmeMessageHeader, CmePacketHdr};
use crate::generated::cme_mdp3::MessageHeader;
use crate::generated::cme_mdp3::md_incremental_refresh_order_book47 as inc_book;
use crate::generated::cme_mdp3::md_incremental_refresh_session_statistics51 as session_stats;
use crate::generated::cme_mdp3::md_incremental_refresh_trade_summary48 as trade_summary;
use crate::generated::cme_mdp3::security_status30 as security_status;
use zerocopy::IntoBytes;
use zerocopy::byteorder::little_endian::U16;

const TEMPLATE_30_PACKET: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/template_30.bin"
));
const TEMPLATE_47_PACKET: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/template_47.bin"
));
const TEMPLATE_48_PACKET: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/template_48.bin"
));
const TEMPLATE_51_PACKET: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/template_51.bin"
));

fn parse_single_message(payload: &[u8]) -> (CmePacketHdr, MessageHeader, &[u8]) {
    let (pkt_hdr, rest) = CmePacketHdr::parse_prefix(payload).expect("packet header");
    let (cme_hdr, tail) = CmeMessageHeader::parse_prefix(rest).expect("message header");

    let msg_len = cme_hdr.msg_len.get() as usize;
    let header_len = mem::size_of::<CmeMessageHeader>();
    assert!(
        msg_len >= header_len,
        "message length {} smaller than header {}",
        msg_len,
        header_len
    );
    let body_len = msg_len - header_len;
    assert!(
        tail.len() >= body_len,
        "message body truncated: need {}, have {}",
        body_len,
        tail.len()
    );
    let (body, remainder) = tail.split_at(body_len);
    assert!(
        remainder.is_empty(),
        "fixture packet has unexpected trailing bytes"
    );

    (*pkt_hdr, cme_hdr.sbe_hdr, body)
}

fn group_entry_len(block_length: u16, declared: usize) -> usize {
    if block_length == 0 {
        declared
    } else {
        block_length as usize
    }
}

fn rebuild_payload(payload: &[u8], rebuild_body: impl FnOnce(&MessageHeader, &[u8]) -> Vec<u8>) {
    let (pkt_hdr, msg_hdr, body) = parse_single_message(payload);
    let rebuilt_body = rebuild_body(&msg_hdr, body);
    let msg_len = mem::size_of::<CmeMessageHeader>() + rebuilt_body.len();
    let cme_hdr = CmeMessageHeader {
        msg_len: U16::new(msg_len as u16),
        sbe_hdr: msg_hdr,
    };

    let mut rebuilt = Vec::with_capacity(mem::size_of::<CmePacketHdr>() + msg_len);
    rebuilt.extend_from_slice(pkt_hdr.as_bytes());
    rebuilt.extend_from_slice(cme_hdr.as_bytes());
    rebuilt.extend_from_slice(&rebuilt_body);

    assert_eq!(rebuilt, payload);
}

fn append_fixed_block(out: &mut Vec<u8>, msg_bytes: &[u8], block_len: usize) {
    out.extend_from_slice(msg_bytes);
    if block_len > msg_bytes.len() {
        out.extend(std::iter::repeat_n(0u8, block_len - msg_bytes.len()));
    }
}

fn set_u8(buf: &mut [u8], offset: usize, value: u8) {
    *buf.get_mut(offset).expect("offset in bounds") = value;
}

#[test]
fn template_30_security_status() {
    let (pkt_hdr, msg_hdr, body) = parse_single_message(TEMPLATE_30_PACKET);
    assert_eq!(msg_hdr.template_id.get(), 30);
    assert_eq!(pkt_hdr.seq.get(), 3896);
    assert_eq!(pkt_hdr.sending_time.get(), 1_689_544_800_018_487_637);

    let (view, tail) = security_status::parse_with_header(body, &msg_hdr).expect("parse");
    assert!(tail.is_empty());

    let msg = view.body;
    assert_eq!(msg.transact_time.get(), 1_689_544_800_000_000_000);
    assert_eq!(msg.security_group, [b'E', b'S', 0, 0, 0, 0]);
    assert_eq!(msg.asset, [0u8; 6]);
    assert_eq!(msg.security_id.get(), i32::MAX);
    assert_eq!(msg.trade_date.get(), 19_555);
    assert_eq!(msg.match_event_indicator.0, 0);
    assert_eq!(msg.security_trading_status.0, 15);
    assert_eq!(msg.halt_reason.0, 0);
    assert_eq!(msg.security_trading_event.0, 0);
}

#[test]
fn template_47_incremental_order_book() {
    let (pkt_hdr, msg_hdr, body) = parse_single_message(TEMPLATE_47_PACKET);
    assert_eq!(msg_hdr.template_id.get(), 47);
    assert_eq!(pkt_hdr.seq.get(), 3899);
    assert_eq!(pkt_hdr.sending_time.get(), 1_689_544_800_021_030_601);

    let (view, after_fixed) = inc_book::parse_with_header(body, &msg_hdr).expect("parse");
    assert_eq!(view.body.transact_time.get(), 1_689_544_800_000_000_000);
    assert_eq!(view.body.match_event_indicator.0, 0);

    let entries = inc_book::parse_no_md_entries(after_fixed).expect("entries");
    assert_eq!(entries.count(), 34);
    assert_eq!(entries.header.block_length.get(), 40);

    let first = entries.iter().next().expect("first entry");
    let body = first.body;
    assert_eq!(body.order_id.get(), 6_412_148_621_783);
    assert_eq!(body.md_order_priority.get(), 15_098_154_137);
    assert_eq!(body.md_entry_px.mantissa.get(), 455_450_000_000_000);
    assert_eq!(body.md_display_qty.get(), 1);
    assert_eq!(body.security_id.get(), 3_445);
    assert_eq!(body.md_update_action.0, 2);
    assert_eq!(body.md_entry_type.0, 48);
}

#[test]
fn template_48_trade_summary() {
    let (pkt_hdr, msg_hdr, body) = parse_single_message(TEMPLATE_48_PACKET);
    assert_eq!(msg_hdr.template_id.get(), 48);
    assert_eq!(pkt_hdr.seq.get(), 3298);
    assert_eq!(pkt_hdr.sending_time.get(), 1_689_544_800_019_788_326);

    let (view, after_fixed) = trade_summary::parse_with_header(body, &msg_hdr).expect("parse");
    assert_eq!(view.body.transact_time.get(), 1_689_544_800_000_000_000);
    assert_eq!(view.body.match_event_indicator.0, 1);

    let entries = trade_summary::parse_no_md_entries(after_fixed).expect("entries");
    assert_eq!(entries.count(), 1);
    assert_eq!(entries.header.block_length.get(), 32);

    let mut iter = entries.iter();
    let first = iter.next().expect("first entry");
    let body = first.body;
    assert_eq!(body.md_entry_px.mantissa.get(), 30_500_000_000_000);
    assert_eq!(body.md_entry_size.get(), 2);
    assert_eq!(body.security_id.get(), 5_785);
    assert_eq!(body.rpt_seq.get(), 45);
    assert_eq!(body.number_of_orders.get(), 3);
    assert_eq!(body.aggressor_side.0, 0);
    assert_eq!(body.md_update_action.0, 0);
    assert_eq!(trade_summary::NoMDEntriesEntry::MD_ENTRY_TYPE.0, b'2');
    assert_eq!(body.md_trade_entry_id.get(), 177);

    // Consume the one entry so the remainder is at the next group.
    let after_entries = iter.remainder();
    let order_entries =
        trade_summary::parse_no_order_id_entries(after_entries).expect("order entries");
    assert_eq!(order_entries.count(), 0);
}

#[test]
fn template_51_session_statistics() {
    let (pkt_hdr, msg_hdr, body) = parse_single_message(TEMPLATE_51_PACKET);
    assert_eq!(msg_hdr.template_id.get(), 51);
    assert_eq!(pkt_hdr.seq.get(), 32_356);
    assert_eq!(pkt_hdr.sending_time.get(), 1_689_544_800_096_718_729);

    let (view, after_fixed) = session_stats::parse_with_header(body, &msg_hdr).expect("parse");
    assert_eq!(view.body.transact_time.get(), 1_689_544_800_087_997_437);
    assert_eq!(view.body.match_event_indicator.0, 136);

    let entries = session_stats::parse_no_md_entries(after_fixed).expect("entries");
    assert_eq!(entries.count(), 28);
    assert_eq!(entries.header.block_length.get(), 24);

    let first = entries.iter().next().expect("first entry");
    let body = first.body;
    assert_eq!(body.md_entry_px.mantissa.get(), 875_000_000_000);
    assert_eq!(body.security_id.get(), 4_242_033);
    assert_eq!(body.rpt_seq.get(), 5);
    assert_eq!(body.open_close_settl_flag.0, 255);
    assert_eq!(body.md_update_action.0, 0);
    assert_eq!(body.md_entry_type.0, 78);
    assert_eq!(body.md_entry_size.get(), i32::MAX);
}

#[test]
fn template_30_roundtrip() {
    rebuild_payload(TEMPLATE_30_PACKET, |msg_hdr, body| {
        let (view, _) = security_status::parse_with_header(body, msg_hdr).expect("parse");
        let msg = *view.body;
        msg.as_bytes()[..view.acting_block_length].to_vec()
    });
}

#[test]
fn template_47_roundtrip() {
    rebuild_payload(TEMPLATE_47_PACKET, |msg_hdr, body| {
        let (view, after_fixed) = inc_book::parse_with_header(body, msg_hdr).expect("parse");
        let msg = *view.body;
        let mut out = Vec::new();
        append_fixed_block(&mut out, msg.as_bytes(), view.acting_block_length);

        let entries = inc_book::parse_no_md_entries(after_fixed).expect("entries");
        out.extend_from_slice(entries.header.as_bytes());
        let entry_len = group_entry_len(
            entries.header.block_length.get(),
            inc_book::NoMDEntriesGroupBuilder::BLOCK_LENGTH as usize,
        );
        let mut iter = entries.iter();
        for entry in iter.by_ref() {
            let body = *entry.body;
            append_fixed_block(&mut out, body.as_bytes(), entry_len);
        }
        out.extend_from_slice(iter.remainder());

        out
    });
}

#[test]
fn template_48_roundtrip() {
    rebuild_payload(TEMPLATE_48_PACKET, |msg_hdr, body| {
        let (view, after_fixed) = trade_summary::parse_with_header(body, msg_hdr).expect("parse");
        let msg = *view.body;
        let mut out = Vec::new();
        append_fixed_block(&mut out, msg.as_bytes(), view.acting_block_length);

        let entries = trade_summary::parse_no_md_entries(after_fixed).expect("entries");
        out.extend_from_slice(entries.header.as_bytes());
        let entry_len = group_entry_len(
            entries.header.block_length.get(),
            trade_summary::NoMDEntriesGroupBuilder::BLOCK_LENGTH as usize,
        );
        let mut iter = entries.iter();
        for entry in iter.by_ref() {
            let body = *entry.body;
            append_fixed_block(&mut out, body.as_bytes(), entry_len);
        }

        let after_entries = iter.remainder();
        let order_entries =
            trade_summary::parse_no_order_id_entries(after_entries).expect("order entries");
        out.extend_from_slice(order_entries.header.as_bytes());
        let order_len = group_entry_len(
            order_entries.header.block_length.get(),
            trade_summary::NoOrderIDEntriesGroupBuilder::BLOCK_LENGTH as usize,
        );
        let mut order_iter = order_entries.iter();
        for entry in order_iter.by_ref() {
            let body = *entry.body;
            append_fixed_block(&mut out, body.as_bytes(), order_len);
        }
        out.extend_from_slice(order_iter.remainder());

        out
    });
}

#[test]
fn template_51_roundtrip() {
    rebuild_payload(TEMPLATE_51_PACKET, |msg_hdr, body| {
        let (view, after_fixed) = session_stats::parse_with_header(body, msg_hdr).expect("parse");
        let msg = *view.body;
        let mut out = Vec::new();
        append_fixed_block(&mut out, msg.as_bytes(), view.acting_block_length);

        let entries = session_stats::parse_no_md_entries(after_fixed).expect("entries");
        out.extend_from_slice(entries.header.as_bytes());
        let entry_len = group_entry_len(
            entries.header.block_length.get(),
            session_stats::NoMDEntriesGroupBuilder::BLOCK_LENGTH as usize,
        );
        let mut iter = entries.iter();
        for entry in iter.by_ref() {
            let body = *entry.body;
            append_fixed_block(&mut out, body.as_bytes(), entry_len);
        }
        out.extend_from_slice(iter.remainder());

        out
    });
}

#[test]
fn template_30_builder_roundtrip() {
    rebuild_payload(TEMPLATE_30_PACKET, |msg_hdr, body| {
        let (view, _) = security_status::parse_with_header(body, msg_hdr).expect("parse");
        let msg = *view.body;

        let mut builder = security_status::SecurityStatus30Builder::new();
        builder
            .transact_time(msg.transact_time)
            .security_group(msg.security_group)
            .asset(msg.asset)
            .security_id(msg.security_id)
            .trade_date(msg.trade_date);
        let mut out = builder.finish().to_vec();

        set_u8(
            &mut out,
            security_status::SecurityStatus30::MATCHEVENTINDICATOR_OFFSET as usize,
            msg.match_event_indicator.0,
        );
        set_u8(
            &mut out,
            security_status::SecurityStatus30::SECURITYTRADINGSTATUS_OFFSET as usize,
            msg.security_trading_status.0,
        );
        set_u8(
            &mut out,
            security_status::SecurityStatus30::HALTREASON_OFFSET as usize,
            msg.halt_reason.0,
        );
        set_u8(
            &mut out,
            security_status::SecurityStatus30::SECURITYTRADINGEVENT_OFFSET as usize,
            msg.security_trading_event.0,
        );

        out
    });
}

#[test]
fn template_47_builder_roundtrip() {
    rebuild_payload(TEMPLATE_47_PACKET, |msg_hdr, body| {
        let (view, after_fixed) = inc_book::parse_with_header(body, msg_hdr).expect("parse");
        let msg = *view.body;
        let entries = inc_book::parse_no_md_entries(after_fixed).expect("entries");

        let mut builder =
            inc_book::MDIncrementalRefreshOrderBook47Builder::with_capacity(body.len());
        builder
            .transact_time(msg.transact_time)
            .no_md_entries(|group| {
                let iter = entries.iter();
                for entry in iter {
                    let body = *entry.body;
                    group.entry(|entry| {
                        entry
                            .order_id(body.order_id)
                            .md_order_priority(body.md_order_priority)
                            .md_entry_px(body.md_entry_px)
                            .md_display_qty(body.md_display_qty)
                            .security_id(body.security_id);
                    });
                }
            });

        let mut out = builder.finish().to_vec();
        set_u8(
            &mut out,
            inc_book::MDIncrementalRefreshOrderBook47::MATCHEVENTINDICATOR_OFFSET as usize,
            msg.match_event_indicator.0,
        );

        let entry_len = inc_book::NoMDEntriesGroupBuilder::BLOCK_LENGTH as usize;
        let base = view.acting_block_length + inc_book::NoMDEntriesGroupBuilder::HEADER_SIZE;
        let mut iter = entries.iter();
        for (index, entry) in iter.by_ref().enumerate() {
            let entry_base = base + index * entry_len;
            set_u8(
                &mut out,
                entry_base + inc_book::NoMDEntriesEntry::MDUPDATEACTION_OFFSET as usize,
                entry.body.md_update_action.0,
            );
            set_u8(
                &mut out,
                entry_base + inc_book::NoMDEntriesEntry::MDENTRYTYPE_OFFSET as usize,
                entry.body.md_entry_type.0,
            );
        }
        out.extend_from_slice(iter.remainder());

        out
    });
}

#[test]
fn template_48_builder_roundtrip() {
    rebuild_payload(TEMPLATE_48_PACKET, |msg_hdr, body| {
        let (view, after_fixed) = trade_summary::parse_with_header(body, msg_hdr).expect("parse");
        let msg = *view.body;
        let entries = trade_summary::parse_no_md_entries(after_fixed).expect("entries");

        let mut entry_iter = entries.iter();
        for _ in entry_iter.by_ref() {}
        let after_entries = entry_iter.remainder();
        let order_entries =
            trade_summary::parse_no_order_id_entries(after_entries).expect("order entries");

        let mut builder =
            trade_summary::MDIncrementalRefreshTradeSummary48Builder::with_capacity(body.len());
        builder
            .transact_time(msg.transact_time)
            .no_md_entries(|group| {
                let iter = entries.iter();
                for entry in iter {
                    let body = *entry.body;
                    group.entry(|entry| {
                        entry
                            .md_entry_px(body.md_entry_px)
                            .md_entry_size(body.md_entry_size)
                            .security_id(body.security_id)
                            .rpt_seq(body.rpt_seq)
                            .number_of_orders(body.number_of_orders)
                            .md_trade_entry_id(body.md_trade_entry_id);
                    });
                }
            })
            .no_order_id_entries(|group| {
                let iter = order_entries.iter();
                for entry in iter {
                    let body = *entry.body;
                    group.entry(|entry| {
                        entry.order_id(body.order_id).last_qty(body.last_qty);
                    });
                }
            });

        let mut out = builder.finish().to_vec();
        set_u8(
            &mut out,
            trade_summary::MDIncrementalRefreshTradeSummary48::MATCHEVENTINDICATOR_OFFSET as usize,
            msg.match_event_indicator.0,
        );

        let entry_len = trade_summary::NoMDEntriesGroupBuilder::BLOCK_LENGTH as usize;
        let base = view.acting_block_length + trade_summary::NoMDEntriesGroupBuilder::HEADER_SIZE;
        let mut iter = entries.iter();
        for (index, entry) in iter.by_ref().enumerate() {
            let entry_base = base + index * entry_len;
            set_u8(
                &mut out,
                entry_base + trade_summary::NoMDEntriesEntry::AGGRESSORSIDE_OFFSET as usize,
                entry.body.aggressor_side.0,
            );
            set_u8(
                &mut out,
                entry_base + trade_summary::NoMDEntriesEntry::MDUPDATEACTION_OFFSET as usize,
                entry.body.md_update_action.0,
            );
        }

        let mut order_iter = order_entries.iter();
        for _ in order_iter.by_ref() {}
        out.extend_from_slice(order_iter.remainder());

        out
    });
}

#[test]
fn template_51_builder_roundtrip() {
    rebuild_payload(TEMPLATE_51_PACKET, |msg_hdr, body| {
        let (view, after_fixed) = session_stats::parse_with_header(body, msg_hdr).expect("parse");
        let msg = *view.body;
        let entries = session_stats::parse_no_md_entries(after_fixed).expect("entries");

        let mut builder =
            session_stats::MDIncrementalRefreshSessionStatistics51Builder::with_capacity(
                body.len(),
            );
        builder
            .transact_time(msg.transact_time)
            .no_md_entries(|group| {
                let iter = entries.iter();
                for entry in iter {
                    let body = *entry.body;
                    group.entry(|entry| {
                        entry
                            .md_entry_px(body.md_entry_px)
                            .security_id(body.security_id)
                            .rpt_seq(body.rpt_seq)
                            .md_entry_size(body.md_entry_size);
                    });
                }
            });

        let mut out = builder.finish().to_vec();
        set_u8(
            &mut out,
            session_stats::MDIncrementalRefreshSessionStatistics51::MATCHEVENTINDICATOR_OFFSET
                as usize,
            msg.match_event_indicator.0,
        );

        let entry_len = session_stats::NoMDEntriesGroupBuilder::BLOCK_LENGTH as usize;
        let base = view.acting_block_length + session_stats::NoMDEntriesGroupBuilder::HEADER_SIZE;
        let mut iter = entries.iter();
        for (index, entry) in iter.by_ref().enumerate() {
            let entry_base = base + index * entry_len;
            set_u8(
                &mut out,
                entry_base + session_stats::NoMDEntriesEntry::OPENCLOSESETTLFLAG_OFFSET as usize,
                entry.body.open_close_settl_flag.0,
            );
            set_u8(
                &mut out,
                entry_base + session_stats::NoMDEntriesEntry::MDUPDATEACTION_OFFSET as usize,
                entry.body.md_update_action.0,
            );
            set_u8(
                &mut out,
                entry_base + session_stats::NoMDEntriesEntry::MDENTRYTYPE_OFFSET as usize,
                entry.body.md_entry_type.0,
            );
        }
        out.extend_from_slice(iter.remainder());

        out
    });
}

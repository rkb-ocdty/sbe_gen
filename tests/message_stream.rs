//! The schema's own polymorphic parser: one enum over every message, and a walk over a packet's
//! worth of length-prefixed frames.

use std::fs;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

use sbe_gen::{GeneratorOptions, generate};

fn write_generated(out_dir: &Path, xml: &str) -> TempDir {
    let temp = tempfile::tempdir_in(out_dir).expect("tempdir");
    let src_dir = temp.path().join("src");
    fs::create_dir_all(&src_dir).expect("create src");
    let modules = generate(xml, &GeneratorOptions::default()).expect("generate");
    for module in modules.modules() {
        fs::write(src_dir.join(&module.name), &module.source).expect("write module");
    }

    let main_rs = r#"
        #![allow(dead_code, non_camel_case_types, unused_imports, unused_variables, unused_mut)]

        #[path = "mod.rs"]
        mod schema;

        use schema::{Body, Header, Message, MessageHeader, Order, Template, messages};
        use sbe_support::framed::LengthPrefixed;
        use zerocopy::IntoBytes;
        use zerocopy::byteorder::little_endian::U16;

        fn frame(template_id: u16, block_length: u16, body: &[u8]) -> Vec<u8> {
            let header = MessageHeader::new(block_length, template_id, 1, 0);
            let frame = LengthPrefixed {
                prefix: U16::new((core::mem::size_of::<LengthPrefixed>() + body.len()) as u16),
                header,
            };
            let mut out = frame.as_bytes().to_vec();
            out.extend_from_slice(body);
            out
        }

        fn main() {
            let mut order = vec![0u8; 12];
            order[0..8].copy_from_slice(&7u64.to_le_bytes());
            order[8..12].copy_from_slice(&5u32.to_le_bytes());
            order.extend_from_slice(&12u16.to_le_bytes());
            order.extend_from_slice(&1u16.to_le_bytes());
            order.extend_from_slice(&9u64.to_le_bytes());
            order.extend_from_slice(&3u32.to_le_bytes());

            let mut fill = vec![0u8; 12];
            fill[0..8].copy_from_slice(&11u64.to_le_bytes());
            fill[8..12].copy_from_slice(&13u32.to_le_bytes());

            let mut packet = frame(1, 12, &order);
            packet.extend_from_slice(&frame(2, 12, &fill));
            packet.extend_from_slice(&frame(99, 12, &fill));

            let mut it = messages(&packet);
            let first = it.next().expect("order frame");
            // the frame is one borrow: the feed's length prefix, the schema's header, the bytes
            assert_eq!(first.frame.header.prefix.get() as usize, order.len() + 10);
            assert_eq!(first.frame.header.template_id(), Order::TEMPLATE_ID);
            assert_eq!(first.frame.block.len(), order.len());
            assert_eq!(Template::from(&first.body), Template::Order);
            assert!(first.body.is_order());
            match first.body {
                Body::Order(order) => {
                    assert_eq!(order.id.get(), 7);
                    assert_eq!(order.qty.get(), 5);
                    assert_eq!(order.no_fills[0].fill_id.get(), 9);
                    assert_eq!(order.no_fills[0].fill_qty.get(), 3);
                    let owned = order.to_owned();
                    assert_eq!(owned.no_fills.len(), 1);
                }
                other => panic!("expected Order, got {other:?}"),
            }
            let second = it.next().expect("fill frame");
            match second.body {
                // no groups, so the block is the whole of it
                Body::FillReport(fill) => {
                    assert_eq!(fill.exec_id.get(), 11);
                    assert_eq!(fill.last_qty.get(), 13);
                }
                other => panic!("expected FillReport, got {other:?}"),
            }
            let third = it.next().expect("unknown frame");
            assert_eq!(third.frame.header.template_id(), 99);
            assert_eq!(Template::from(&third.body), Template::Unknown);
            assert!(matches!(third.body, Body::Unknown));
            assert_eq!(third.frame.block.len(), 12);
            assert!(it.next().is_none());
            assert!(it.rest.is_empty());

            // a frame cut short stops the walk with the bytes still there to see
            let truncated = &packet[..packet.len() - 4];
            let mut it = messages(truncated);
            assert!(matches!(it.next().map(|m| m.body), Some(Body::Order(_))));
            assert!(matches!(it.next().map(|m| m.body), Some(Body::FillReport(_))));
            assert!(it.next().is_none());
            assert_eq!(it.rest.len(), core::mem::size_of::<LengthPrefixed>() + 12 - 4);

            // one frame on its own, behind the plain header rather than the feed's
            let plain = {
                let mut out = MessageHeader::new(12, 1, 1, 0).to_bytes().to_vec();
                out.extend_from_slice(&order);
                out
            };
            let one = Message::<MessageHeader>::parse_frame(&plain).expect("single message");
            assert_eq!(one.frame.header.template_id.get(), 1);
            assert!(matches!(one.body, Body::Order(_)));
        }
    "#;
    fs::write(src_dir.join("main.rs"), main_rs).expect("write main");

    let cargo_toml = r#"[package]
name = "message_stream"
version = "0.0.0"
edition = "2024"

[dependencies]
zerocopy = { version = "0.8", features = ["derive"] }
sbe_support = { path = "SBE_SUPPORT_PATH" }

[workspace]
"#;
    fs::write(
        temp.path().join("Cargo.toml"),
        cargo_toml.replace(
            "SBE_SUPPORT_PATH",
            &format!("{}/sbe_support", env!("CARGO_MANIFEST_DIR")),
        ),
    )
    .expect("write Cargo.toml");
    temp
}

#[test]
fn walks_a_packet_of_frames_into_the_message_enum() {
    let xml = r#"
        <messageSchema package="test" id="1" version="0">
            <types>
                <composite name="groupSize">
                    <type name="blockLength" primitiveType="uint16"/>
                    <type name="numInGroup" primitiveType="uint16"/>
                </composite>
            </types>
            <message name="Order" id="1" blockLength="12">
                <field name="id" id="1" type="uint64" offset="0"/>
                <field name="qty" id="2" type="uint32" offset="8"/>
                <group name="NoFills" id="100" blockLength="12" dimensionType="groupSize">
                    <field name="fillId" id="101" type="uint64" offset="0"/>
                    <field name="fillQty" id="102" type="uint32" offset="8"/>
                </group>
            </message>
            <message name="FillReport" id="2" blockLength="12">
                <field name="execId" id="17" type="uint64" offset="0"/>
                <field name="lastQty" id="32" type="uint32" offset="8"/>
            </message>
        </messageSchema>
    "#;

    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"));
    let temp = write_generated(workspace, xml);
    let status = Command::new("cargo")
        .args(["run", "--quiet", "--offline"])
        .current_dir(temp.path())
        .status()
        .expect("spawn cargo");
    assert!(status.success(), "message-stream example should compile and run");
}

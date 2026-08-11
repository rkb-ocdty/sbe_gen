//! A feed that frames its messages behind its own header still parses: the schema's header is a
//! member of the frame, and `parse_message_framed` reads the whole thing in one pass.

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

        mod types;
        mod order;

        use order::*;
        use sbe_support::{MessageHeader, Prefixed, Suffixed};
        use zerocopy::IntoBytes;
        use zerocopy::byteorder::little_endian::{U16, U32};

        // what CME wraps a secdef report in: a length, the schema's header, then the frame's own
        // trailing count
        type Framed = Prefixed<U16, Suffixed<MessageHeader, U32>>;
        // the plain length-prefixed frame comes ready made
        type Plain = sbe_support::framed::LengthPrefixed;

        fn body() -> Vec<u8> {
            let mut body = vec![0u8; Order::BLOCK_LENGTH as usize];
            body[0..8].copy_from_slice(&7u64.to_le_bytes());
            body[8..12].copy_from_slice(&5u32.to_le_bytes());
            body.extend_from_slice(&12u16.to_le_bytes());
            body.extend_from_slice(&1u16.to_le_bytes());
            body.extend_from_slice(&9u64.to_le_bytes());
            body.extend_from_slice(&3u32.to_le_bytes());
            body
        }

        fn header() -> MessageHeader {
            MessageHeader::new(
                Order::BLOCK_LENGTH,
                Order::TEMPLATE_ID,
                Order::SCHEMA_ID,
                Order::SCHEMA_VERSION,
            )
        }

        fn main() {
            let body = body();

            let mut plain = header().to_bytes().to_vec();
            plain.extend_from_slice(&body);
            let msg = OrderRef::parse_message(&plain).expect("plain parse");
            assert_eq!(msg.id.get(), 7);
            assert_eq!(msg.no_fills.len(), 1);
            assert_eq!(msg.no_fills[0].fill_id.get(), 9);

            let frame = Framed {
                prefix: U16::new((MessageHeader::SIZE + 4 + body.len()) as u16),
                header: Suffixed { header: header(), suffix: U32::new(2) },
            };
            let mut framed = frame.as_bytes().to_vec();
            framed.extend_from_slice(&body);
            let msg = OrderRef::parse_message_framed::<Framed>(&framed).expect("framed parse");
            assert_eq!(msg.id.get(), 7);
            assert_eq!(msg.qty.get(), 5);
            assert_eq!(msg.no_fills[0].fill_qty.get(), 3);

            let (view, _) = order::parse_with_framed_header(&body, &frame).expect("framed view");
            assert_eq!(view.id().expect("id").get(), 7);

            // the view walked into one borrow of the whole message
            let whole = view.whole().expect("whole");
            assert_eq!(whole.id.get(), 7);
            assert_eq!(whole.no_fills[0].fill_id.get(), 9);

            let mut prefixed = Plain { prefix: U16::new(body.len() as u16), header: header() }.as_bytes().to_vec();
            prefixed.extend_from_slice(&body);
            let msg = OrderRef::parse_message_framed::<Plain>(&prefixed).expect("length-prefixed parse");
            assert_eq!(msg.id.get(), 7);

            let owned = msg.to_owned();
            assert_eq!(owned.no_fills[0].fill_id.get(), 9);
        }
    "#;
    fs::write(src_dir.join("main.rs"), main_rs).expect("write main");

    let cargo_toml = r#"[package]
name = "framed_header"
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
fn parses_behind_a_caller_supplied_frame() {
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
        </messageSchema>
    "#;

    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"));
    let temp = write_generated(workspace, xml);
    let status = Command::new("cargo")
        .args(["run", "--quiet", "--offline"])
        .current_dir(temp.path())
        .status()
        .expect("spawn cargo");
    assert!(status.success(), "framed-header example should compile and run");
}

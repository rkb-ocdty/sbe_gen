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
    for (name, contents) in modules {
        fs::write(src_dir.join(name), contents).expect("write module");
    }
    let main_rs = r#"
        #![allow(dead_code, non_camel_case_types, unused_imports, unused_variables, unused_mut)]

        mod types;
        mod message_header;
        mod heartbeat;
        mod negotiate500;
        mod trade;

        use heartbeat::*;
        use message_header::MessageHeader;
        use negotiate500::*;
        use trade::*;
        use zerocopy::byteorder::little_endian::U16;

        fn main() {
            // fixed-only borrowed path
            let mut fixed_dst = [0u8; 64];
            let heartbeat_len = Heartbeat::encode_body_into(&mut fixed_dst, |enc| {
                enc.seq(11);
                enc.sending_time(99);
                Ok(())
            }).expect("heartbeat encode");
            assert_eq!(heartbeat_len, Heartbeat::BLOCK_LENGTH as usize);
            let (hb, tail) = Heartbeat::parse_prefix(&fixed_dst[..heartbeat_len]).expect("heartbeat parse");
            assert_eq!(hb.seq.get(), 11);
            assert_eq!(hb.sending_time.get(), 99);
            assert!(tail.is_empty());

            // borrowed header+body equals owned builder framing
            let mut builder = Negotiate500Builder::new();
            builder.seq(42).firm(7);
            builder.credentials(b"secret").expect("credentials");
            let owned_framed = builder.finish_with_header();

            let header = MessageHeader {
                block_length: U16::new(Negotiate500::BLOCK_LENGTH),
                template_id: U16::new(Negotiate500::TEMPLATE_ID),
                schema_id: U16::new(Negotiate500::SCHEMA_ID),
                version: U16::new(Negotiate500::SCHEMA_VERSION),
            };
            let mut framed_dst = [0u8; 128];
            let framed_len = Negotiate500::encode_with_header_into(&mut framed_dst, header, |enc| {
                enc.seq(42).firm(7);
                enc.credentials(b"secret")?;
                Ok(())
            }).expect("framed encode");
            assert_eq!(&framed_dst[..framed_len], owned_framed.as_slice());

            // borrowed body with var-data
            let mut body_dst = [0u8; 128];
            let body_len = Negotiate500::encode_body_into(&mut body_dst, |enc| {
                enc.seq(100).firm(200);
                enc.credentials(b"abc")?;
                Ok(())
            }).expect("body encode");
            let (msg, rest) = Negotiate500::parse_prefix(&body_dst[..body_len]).expect("body parse");
            assert_eq!(msg.seq.get(), 100);
            assert_eq!(msg.firm.get(), 200);
            let (creds, tail) = msg.parse_credentials(rest).expect("credentials parse");
            assert_eq!(creds.as_str().unwrap(), "abc");
            assert!(tail.is_empty());

            // precise buffer-too-small error
            let mut short_dst = [0u8; 8];
            let short_err = Negotiate500::encode_body_into(&mut short_dst, |enc| {
                enc.seq(1).firm(2);
                Ok(())
            }).expect_err("expected buffer-too-small");
            match short_err {
                negotiate500::EncodeIntoError::BufferTooSmall { required, available } => {
                    assert!(required > available);
                    assert_eq!(available, 8);
                }
                other => panic!("unexpected error: {:?}", other),
            }

            // var-data length overflow error (uint8 length prefix)
            let mut overflow_dst = [0u8; 512];
            let payload = [9u8; 300];
            let overflow_err = Negotiate500::encode_body_into(&mut overflow_dst, |enc| {
                enc.seq(1).firm(2);
                enc.credentials(&payload)?;
                Ok(())
            }).expect_err("expected length-overflow");
            match overflow_err {
                negotiate500::EncodeIntoError::LengthOverflow { len, max } => {
                    assert_eq!(len, 300);
                    assert_eq!(max, 255);
                }
                other => panic!("unexpected error: {:?}", other),
            }

            // borrowed groups+var-data parity with owned builder framing
            let mut trade_builder = TradeBuilder::new();
            trade_builder.seq(9).price(555);
            trade_builder.legs(|legs| {
                legs.entry(|entry| {
                    entry.qty(10).side(1);
                    entry.note(b"bid").expect("note");
                });
                legs.entry(|entry| {
                    entry.qty(20).side(2);
                    entry.note(b"ask").expect("note");
                });
            }).expect("trade legs");
            trade_builder.comment(b"ok").expect("comment");
            let owned_trade = trade_builder.finish_with_header();

            let trade_header = MessageHeader {
                block_length: U16::new(Trade::BLOCK_LENGTH),
                template_id: U16::new(Trade::TEMPLATE_ID),
                schema_id: U16::new(Trade::SCHEMA_ID),
                version: U16::new(Trade::SCHEMA_VERSION),
            };
            let mut trade_dst = [0u8; 256];
            let trade_len = Trade::encode_with_header_into(&mut trade_dst, trade_header, |enc| {
                enc.seq(9).price(555);
                enc.legs(|legs| {
                    legs.entry(|entry| {
                        entry.qty(10).side(1);
                        entry.note(b"bid")?;
                        Ok(())
                    })?;
                    legs.entry(|entry| {
                        entry.qty(20).side(2);
                        entry.note(b"ask")?;
                        Ok(())
                    })?;
                    Ok(())
                })?;
                enc.comment(b"ok")?;
                Ok(())
            }).expect("trade encode");
            assert_eq!(&trade_dst[..trade_len], owned_trade.as_slice());
        }
    "#;
    fs::write(src_dir.join("main.rs"), main_rs).expect("write main");
    let cargo_toml = r#"[package]
name = "encode_into_example"
version = "0.0.0"
edition = "2024"

[dependencies]
zerocopy = { version = "0.8", features = ["derive"] }
"#;
    fs::write(temp.path().join("Cargo.toml"), cargo_toml).expect("write Cargo.toml");
    temp
}

#[test]
fn encode_into_roundtrip_and_parity() {
    let xml = r#"
        <messageSchema package="test" schemaId="1" version="1">
            <types>
                <composite name="groupSize">
                    <type name="blockLength" primitiveType="uint16"/>
                    <type name="numInGroup" primitiveType="uint16"/>
                </composite>
                <type name="varStringEncoding" primitiveType="uint8"/>
                <type name="ClientFlowType" presence="constant" length="10" primitiveType="char">IDEMPOTENT</type>
            </types>
            <message name="Heartbeat" id="1" blockLength="12">
                <field name="seq" id="1" type="uint32" offset="0"/>
                <field name="sending_time" id="2" type="uint64" offset="4"/>
            </message>
            <message name="Negotiate500" id="500" blockLength="12">
                <field name="CustomerFlow" id="1" type="ClientFlowType"/>
                <field name="seq" id="2" type="uint32" offset="0"/>
                <field name="firm" id="3" type="uint64" offset="4"/>
                <data name="credentials" id="4" type="varStringEncoding"/>
            </message>
            <message name="Trade" id="2" blockLength="12">
                <field name="seq" id="1" type="uint32" offset="0"/>
                <field name="price" id="2" type="int64" offset="4"/>
                <group name="Legs" id="3" blockLength="5" dimensionType="groupSize">
                    <field name="qty" id="1" type="int32" offset="0"/>
                    <field name="side" id="2" type="uint8" offset="4"/>
                    <data name="note" id="3" type="varStringEncoding"/>
                </group>
                <data name="comment" id="4" type="varStringEncoding"/>
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
    assert!(
        status.success(),
        "encode-into example should compile and run"
    );
}

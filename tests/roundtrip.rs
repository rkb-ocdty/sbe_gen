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
    // small binary that encodes a message and decodes it back
    let main_rs = r#"
        #![allow(dead_code, non_camel_case_types, unused_imports, unused_variables, unused_mut)]

        mod types;
        mod message_header;
        mod trade;

        use message_header::MessageHeader;
        use trade::*;

        fn main() {
            let mut builder = TradeBuilder::new();
            builder.seq(42);
            builder.price(12_345);
            builder.legs(|legs| {
                legs.entry(|entry| {
                    entry.qty(10);
                    entry.side(1);
                    entry.note(b"bid").expect("note");
                });
                legs.entry(|entry| {
                    entry.qty(20);
                    entry.side(2);
                    entry.note(b"ask").expect("note");
                });
            }).expect("legs");
            builder.comment(b"ok").expect("comment");

            let framed = builder.finish_with_header();
            let (hdr, body) = MessageHeader::parse_prefix(&framed).expect("header");
            assert_eq!(hdr.template_id.get(), Trade::TEMPLATE_ID);

            let (view, after_fixed) = parse_with_header(body, hdr).expect("view");
            assert_eq!(view.body.seq.get(), 42);
            assert_eq!(view.body.price.get(), 12_345);

            let legs = parse_legs(after_fixed).expect("legs");
            assert_eq!(legs.count(), 2);

            let mut iter = legs.iter();
            let first = iter.next().expect("first leg");
            assert_eq!(first.body.qty.get(), 10);
            assert_eq!(first.body.side, 1);
            assert_eq!(first.note.as_str().unwrap(), "bid");

            let second = iter.next().expect("second leg");
            assert_eq!(second.body.qty.get(), 20);
            assert_eq!(second.body.side, 2);
            assert_eq!(second.note.as_str().unwrap(), "ask");
            assert!(iter.next().is_none());

            let after_legs = iter.remainder();
            let (comment, tail) = view.body.parse_comment(after_legs).expect("comment");
            assert_eq!(comment.as_str().unwrap(), "ok");
            assert!(tail.is_empty());
        }
    "#;
    fs::write(src_dir.join("main.rs"), main_rs).expect("write main");
    let cargo_toml = r#"[package]
name = "roundtrip_example"
version = "0.0.0"
edition = "2024"

[dependencies]
zerocopy = { version = "0.8", features = ["derive"] }
"#;
    fs::write(temp.path().join("Cargo.toml"), cargo_toml).expect("write Cargo.toml");
    temp
}

#[test]
fn encode_decode_roundtrip() {
    let xml = r#"
        <messageSchema package="test" schemaId="1" version="1">
            <types>
                <composite name="groupSize">
                    <type name="blockLength" primitiveType="uint16"/>
                    <type name="numInGroup" primitiveType="uint16"/>
                </composite>
                <type name="varStringEncoding" primitiveType="uint8"/>
            </types>
            <message name="Trade" id="1" blockLength="12" semanticType="T">
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
    assert!(status.success(), "roundtrip example should compile and run");
}

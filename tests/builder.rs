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
    // simple binary exercising the builders
    let main_rs = r#"
        #![allow(dead_code, non_camel_case_types, unused_imports, unused_variables, unused_mut)]

        mod types;
        mod message_header;
        mod order_book;

        use order_book::*;

        fn main() {
            let mut builder = OrderBookBuilder::new();
            builder.seq(7);
            builder.source(9);
            builder.bids(|bids| {
                bids.entry(|entry| {
                    entry.price(-5);
                    entry.qty(10);
                    entry.note(b"hi");
                    entry.tags(|tags| {
                        tags.entry(|tag| {
                            tag.tag(99);
                        });
                    });
                });
            });
            builder.comment(b"ok");

            let body = builder.finish();
            let (msg, rest) = OrderBook::parse_prefix(&body).expect("parse header");
            assert_eq!(msg.seq.get(), 7);
            assert_eq!(msg.source.get(), 9);

            let bids = parse_bids(rest).expect("parse bids");
            let mut bid_iter = bids.iter();
            let first = bid_iter.next().expect("first bid");
            assert_eq!(first.body.price.get(), -5);
            assert_eq!(first.body.qty.get(), 10);
            assert_eq!(first.note.as_str().unwrap(), "hi");

            let mut tag_iter = first.tags.iter();
            let tag = tag_iter.next().expect("tag");
            assert_eq!(tag.body.tag.get(), 99);
            assert!(tag_iter.next().is_none());

            let after_bids = bid_iter.remainder();
            let (comment, tail) = msg.parse_comment(after_bids).expect("comment");
            assert_eq!(comment.as_str().unwrap(), "ok");
            assert!(tail.is_empty());

            let bad = std::panic::catch_unwind(|| {
                let mut b = OrderBookBuilder::new();
                b.seq(1_000_000_000); // above maxValue
            });
            assert!(bad.is_err(), "value constraints should panic on violation");
        }
    "#;
    fs::write(src_dir.join("main.rs"), main_rs).expect("write main");
    let cargo_toml = r#"[package]
name = "builder_roundtrip"
version = "0.0.0"
edition = "2024"

[dependencies]
zerocopy = { version = "0.8", features = ["derive"] }
"#;
    fs::write(temp.path().join("Cargo.toml"), cargo_toml).expect("write Cargo.toml");
    temp
}

#[test]
fn builders_round_trip_through_decoder() {
    let xml = r#"
        <messageSchema package="test" schemaId="1" version="2">
            <types>
                <composite name="groupSize">
                    <type name="blockLength" primitiveType="uint16"/>
                    <type name="numInGroup" primitiveType="uint16"/>
                </composite>
                <type name="varStringEncoding" primitiveType="uint8"/>
            </types>
            <message name="OrderBook" id="42" blockLength="8" semanticType="d">
                <field name="seq" id="1" type="uint32" offset="0" minValue="0" maxValue="1000000"/>
                <field name="source" id="2" type="uint32" offset="4" minValue="0" maxValue="1000000"/>
                <group name="Bids" id="3" blockLength="12" dimensionType="groupSize">
                    <field name="price" id="1" type="int64"/>
                    <field name="qty" id="2" type="int32"/>
                    <data name="note" id="3" type="varStringEncoding"/>
                    <group name="Tags" id="4" blockLength="4" dimensionType="groupSize">
                        <field name="tag" id="1" type="uint32"/>
                    </group>
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
    assert!(status.success(), "builder example should compile and run");
}

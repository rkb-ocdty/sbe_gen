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
        mod dim_ref;

        use dim_ref::*;

        fn main() {
            let mut builder = DimRefBuilder::new();
            builder.seq(42);
            builder.items(|items| {
                items.entry(|entry| {
                    entry.id(11);
                });
                items.entry(|entry| {
                    entry.id(22);
                });
            }).expect("items");

            let body = builder.finish();
            let (msg, rest) = DimRef::parse_prefix(&body).expect("parse message");
            assert_eq!(msg.seq.get(), 42);

            let group = parse_items(rest).expect("parse group");
            assert_eq!(group.header.blk_len.get(), 2);
            assert_eq!(group.header.entry_cnt, 2);
            assert_eq!(group.count(), 2);

            let mut it = group.iter();
            let first = it.next().expect("first");
            assert_eq!(first.body.id.get(), 11);
            let second = it.next().expect("second");
            assert_eq!(second.body.id.get(), 22);
            assert!(it.next().is_none());
            assert!(it.remainder().is_empty());
        }
    "#;
    fs::write(src_dir.join("main.rs"), main_rs).expect("write main");

    let cargo_toml = r#"[package]
name = "dimension_ref_example"
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
fn group_dimension_composite_refs_are_supported() {
    let xml = r#"
        <messageSchema package="test">
            <types>
                <type name="BL" primitiveType="uint16"/>
                <type name="CNT" primitiveType="uint8"/>
                <composite name="groupSizeRef">
                    <ref name="blkLen" type="BL"/>
                    <ref name="entryCnt" type="CNT"/>
                </composite>
            </types>
            <message name="DimRef" id="1" blockLength="4">
                <field name="seq" id="1" type="uint32" offset="0"/>
                <group name="Items" id="2" blockLength="2" dimensionType="groupSizeRef">
                    <field name="id" id="1" type="uint16" offset="0"/>
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
    assert!(
        status.success(),
        "dimension-ref group example should compile and run"
    );
}

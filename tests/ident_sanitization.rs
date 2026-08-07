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
        mod type_;

        use type_::type_ as type_msg;
        use type_::*;
        use types::{match_, match_Enum};

        fn main() {
            let mut builder = type_Builder::new();
            builder.match_(match_::from(match_Enum::Type));
            builder.self_(|group| {
                group.entry(|entry| {
                    entry.type_(7);
                });
            }).expect("group");
            builder.enum_(b"ok").expect("data");

            let body = builder.finish();
            let (msg, tail) = type_msg::parse_prefix(&body).expect("parse");
            assert_eq!(msg.match_.0, 1u8);

            let group = parse_self_(tail).expect("parse group");
            let mut iter = group.iter();
            let first = iter.next().expect("entry");
            assert_eq!(first.body.type_, 7u8);
            assert!(iter.next().is_none());

            let (data, rest) = msg.parse_enum_(iter.remainder()).expect("data");
            assert_eq!(data.as_str().unwrap(), "ok");
            assert!(rest.is_empty());
        }
    "#;
    fs::write(src_dir.join("main.rs"), main_rs).expect("write main");

    let cargo_toml = r#"[package]
name = "ident_sanitization"
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
fn generated_code_sanitizes_rust_identifiers() {
    let xml = r#"
        <messageSchema package="test">
            <types>
                <enum name="match" encodingType="uint8">
                    <validValue name="type">1</validValue>
                </enum>
                <composite name="group-size">
                    <type name="type" primitiveType="uint16"/>
                    <type name="match" primitiveType="uint16"/>
                </composite>
                <type name="var-data" primitiveType="uint8"/>
            </types>
            <message name="type" id="1" blockLength="1">
                <field name="match" id="1" type="match"/>
                <group name="self" id="2" dimensionType="group-size">
                    <field name="type" id="1" type="uint8"/>
                </group>
                <data name="enum" id="3" type="var-data"/>
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
        "generated keyword-heavy schema should compile and run"
    );
}

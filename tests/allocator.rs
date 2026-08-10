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
        mod order_book;

        use order_book::*;

        use std::alloc::Layout;
        use std::ptr::NonNull;
        use std::sync::atomic::{AtomicUsize, Ordering};

        use allocator_api2::alloc::{AllocError, Allocator, Global};

        static CALLS: AtomicUsize = AtomicUsize::new(0);

        #[derive(Clone, Copy)]
        struct Counting;

        unsafe impl Allocator for Counting {
            fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
                CALLS.fetch_add(1, Ordering::Relaxed);
                Global.allocate(layout)
            }
            unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
                unsafe { Global.deallocate(ptr, layout) }
            }
        }

        // written twice rather than through a generic helper: the group builder hands the
        // entry closure a `Builder<'_>` that borrows the buffer, so a caller generic over the
        // allocator has to prove `A: 'static` to name it. Picking the allocator at the call
        // site, which is what anyone does, has no such problem.
        macro_rules! fill {
            ($builder:expr) => {{
                let builder = &mut $builder;
                builder.seq(7);
                builder.source(9);
                builder
                    .bids(|bids| {
                        bids.entry(|entry| {
                            entry.price(-5);
                            entry.qty(10);
                            entry.note(b"hi").expect("note");
                        });
                    })
                    .expect("bids");
                builder.comment(b"ok").expect("comment");
            }};
        }

        fn main() {
            let mut theirs = OrderBookBuilder::new_in(Counting);
            fill!(theirs);
            let theirs = theirs.finish();

            let mut ours = OrderBookBuilder::new();
            fill!(ours);
            let ours = ours.finish();

            assert!(CALLS.load(Ordering::Relaxed) > 0, "the allocator was never asked");
            assert_eq!(&theirs[..], &ours[..], "same bytes whoever allocated them");

            let (msg, rest) = OrderBook::parse_prefix(&theirs).expect("parse block");
            assert_eq!(msg.seq.get(), 7);
            let bids = parse_bids(rest).expect("parse bids");
            let first = bids.iter().next().expect("first bid");
            assert_eq!(first.body.price.get(), -5);
            assert_eq!(first.note.as_str().unwrap(), "hi");
        }
    "#;
    fs::write(src_dir.join("main.rs"), main_rs).expect("write main");
    let cargo_toml = r#"[package]
name = "allocator_roundtrip"
version = "0.0.0"
edition = "2024"

[dependencies]
zerocopy = { version = "0.8", features = ["derive"] }
allocator-api2 = "0.4"
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
fn builders_take_a_caller_supplied_allocator() {
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
                <field name="seq" id="1" type="uint32" offset="0"/>
                <field name="source" id="2" type="uint32" offset="4"/>
                <group name="Bids" id="3" blockLength="12" dimensionType="groupSize">
                    <field name="price" id="1" type="int64"/>
                    <field name="qty" id="2" type="int32"/>
                    <data name="note" id="3" type="varStringEncoding"/>
                </group>
                <data name="comment" id="4" type="varStringEncoding"/>
            </message>
        </messageSchema>
    "#;
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"));
    let temp = write_generated(workspace, xml);
    let status = Command::new("cargo")
        .args(["run", "--quiet"])
        .current_dir(temp.path())
        .status()
        .expect("spawn cargo");
    assert!(status.success(), "allocator example should compile and run");
}

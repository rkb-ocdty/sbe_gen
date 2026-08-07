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
        mod security_definition_response561;
        mod party_details_definition_request_ack519;

        use sbe_support::MessageHeader;
        use party_details_definition_request_ack519::*;
        use security_definition_response561::*;
        use zerocopy::byteorder::little_endian::U16;

        fn main() {
            assert_eq!(
                core::mem::size_of::<SecurityDefinitionResponse561>(),
                SecurityDefinitionResponse561::BLOCK_LENGTH as usize
            );
            assert_eq!(core::mem::size_of::<NoLegsEntry>(), 19);

            let mut sdr = vec![0u8; SecurityDefinitionResponse561::BLOCK_LENGTH as usize];
            sdr[396..399].copy_from_slice(b"USD");
            sdr[399] = 12;
            let tail = 0x0102_0304_0506_0708u64;
            sdr[422..430].copy_from_slice(&tail.to_le_bytes());

            let (msg, rest) = SecurityDefinitionResponse561::parse_prefix(&sdr).expect("sdr parse");
            assert_eq!(msg.maturity_month_year.month, 12);
            assert_eq!(msg.tail.get(), tail);
            assert!(rest.is_empty());
            assert_eq!(msg.security_id_source(), [b'8']);
            assert_eq!(SecurityDefinitionResponse561::SECURITY_ID_SOURCE, [b'8']);

            let header_v1 = MessageHeader {
                block_length: U16::new(SecurityDefinitionResponse561::BLOCK_LENGTH),
                template_id: U16::new(SecurityDefinitionResponse561::TEMPLATE_ID),
                schema_id: U16::new(SecurityDefinitionResponse561::SCHEMA_ID),
                version: U16::new(1),
            };
            let (view_v1, after_fixed_v1) = security_definition_response561::parse_with_header(&sdr, &header_v1)
                .expect("sdr view v1");
            assert!(!view_v1.has_security_id_source());
            assert!(view_v1.security_id_source().is_none());
            assert_eq!(view_v1.maturity_month_year().expect("month v1").month, 12);
            assert_eq!(view_v1.tail().expect("tail v1").get(), tail);

            let header_v2 = MessageHeader {
                block_length: U16::new(SecurityDefinitionResponse561::BLOCK_LENGTH),
                template_id: U16::new(SecurityDefinitionResponse561::TEMPLATE_ID),
                schema_id: U16::new(SecurityDefinitionResponse561::SCHEMA_ID),
                version: U16::new(2),
            };
            let (view_v2, after_fixed_v2) = security_definition_response561::parse_with_header(&sdr, &header_v2)
                .expect("sdr view v2");
            assert!(view_v2.has_security_id_source());
            assert_eq!(view_v2.security_id_source().expect("const v2"), [b'8']);
            assert_eq!(view_v2.maturity_month_year().expect("month v2").month, 12);
            assert_eq!(view_v2.tail().expect("tail v2").get(), tail);
            assert_eq!(after_fixed_v1, after_fixed_v2);

            let mut no_legs_bytes = Vec::new();
            no_legs_bytes.extend_from_slice(&19u16.to_le_bytes());
            no_legs_bytes.extend_from_slice(&1u16.to_le_bytes());
            let mut entry = [0u8; 19];
            entry[0..8].copy_from_slice(&11u64.to_le_bytes());
            entry[8..16].copy_from_slice(&22u64.to_le_bytes());
            entry[16] = 3;
            entry[17..19].copy_from_slice(&44u16.to_le_bytes());
            no_legs_bytes.extend_from_slice(&entry);

            let group = parse_no_legs(&no_legs_bytes).expect("parse no_legs");
            assert_eq!(group.count(), 1);
            let mut it = group.iter();
            let leg = it.next().expect("first leg");
            assert!(leg.has_leg_security_id());
            assert!(leg.has_leg_ratio_qty());
            assert!(leg.has_leg_side());
            assert!(leg.has_leg_price());
            assert_eq!(leg.leg_security_id().expect("leg security id").get(), 11);
            assert_eq!(leg.leg_ratio_qty().expect("leg ratio qty").get(), 22);
            assert_eq!(leg.leg_side().expect("leg side"), &3u8);
            assert_eq!(leg.leg_price().expect("leg price").get(), 44);
            assert_eq!(leg.body.leg_security_id.get(), 11);
            assert_eq!(leg.body.leg_ratio_qty.get(), 22);
            assert_eq!(leg.body.leg_side, 3);
            assert_eq!(leg.body.leg_price.get(), 44);
            assert_eq!(leg.body.leg_security_id_source(), [b'8']);
            assert!(it.next().is_none());
            assert!(it.remainder().is_empty());

            let mut short_no_legs_bytes = Vec::new();
            short_no_legs_bytes.extend_from_slice(&17u16.to_le_bytes());
            short_no_legs_bytes.extend_from_slice(&1u16.to_le_bytes());
            let mut short_entry = [0u8; 17];
            short_entry[0..8].copy_from_slice(&111u64.to_le_bytes());
            short_entry[8..16].copy_from_slice(&222u64.to_le_bytes());
            short_entry[16] = 4;
            short_no_legs_bytes.extend_from_slice(&short_entry);

            let short_group = parse_no_legs(&short_no_legs_bytes).expect("parse short no_legs");
            let mut short_it = short_group.iter();
            let short_leg = short_it.next().expect("first short leg");
            assert!(short_leg.has_leg_security_id());
            assert!(short_leg.has_leg_ratio_qty());
            assert!(short_leg.has_leg_side());
            assert!(!short_leg.has_leg_price());
            assert!(short_leg.leg_price().is_none());
            assert_eq!(
                short_leg.leg_security_id().expect("short leg security id").get(),
                111
            );
            assert_eq!(NoLegsEntry::LEG_SECURITY_ID_SOURCE, [b'8']);
            assert!(short_it.next().is_none());
            assert!(short_it.remainder().is_empty());

            assert_eq!(
                core::mem::size_of::<PartyDetailsDefinitionRequestAck519>(),
                PartyDetailsDefinitionRequestAck519::BLOCK_LENGTH as usize
            );
            let mut ack = vec![0u8; PartyDetailsDefinitionRequestAck519::BLOCK_LENGTH as usize];
            ack[155..159].copy_from_slice(&777u32.to_le_bytes());
            let (ack_msg, ack_tail) = PartyDetailsDefinitionRequestAck519::parse_prefix(&ack).expect("ack parse");
            assert!(ack_tail.is_empty());
            assert_eq!(ack_msg.request_result.get(), 777);
            assert_eq!(ack_msg.no_party_updates(), 2u8);
        }
    "#;
    fs::write(src_dir.join("main.rs"), main_rs).expect("write main");

    let cargo_toml = r#"[package]
name = "constant_presence_regressions"
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
fn constant_presence_does_not_shift_layout_or_group_entries() {
    let xml = r#"
        <messageSchema package="test" schemaId="1" version="2">
            <types>
                <composite name="groupSize">
                    <type name="blockLength" primitiveType="uint16"/>
                    <type name="numInGroup" primitiveType="uint16"/>
                </composite>
                <type name="Currency3" primitiveType="char" length="3"/>
                <type name="SecurityIDSource" primitiveType="char" length="1" presence="constant">8</type>
                <composite name="MaturityMonthYear">
                    <type name="month" primitiveType="uint8"/>
                </composite>
                <type name="Pad155" primitiveType="char" length="155"/>
                <type name="NoPartyUpdates" primitiveType="uint8" presence="constant">2</type>
            </types>
            <message name="SecurityDefinitionResponse561" id="561" blockLength="430">
                <field name="Currency" id="15" type="Currency3" offset="396"/>
                <field name="SecurityIDSource" id="22" type="SecurityIDSource" sinceVersion="2"/>
                <field name="MaturityMonthYear" id="200" type="MaturityMonthYear" offset="399"/>
                <field name="Tail" id="999" type="uint64" offset="422"/>
                <group name="NoLegs" id="555" blockLength="19" dimensionType="groupSize">
                    <field name="LegSecurityID" id="602" type="uint64" offset="0"/>
                    <field name="LegSecurityIDSource" id="603" type="SecurityIDSource"/>
                    <field name="LegRatioQty" id="623" type="uint64" offset="8"/>
                    <field name="LegSide" id="624" type="uint8" offset="16"/>
                    <field name="LegPrice" id="566" type="uint16" offset="17"/>
                </group>
            </message>
            <message name="PartyDetailsDefinitionRequestAck519" id="519" blockLength="159">
                <field name="PartyText" id="1000" type="Pad155" offset="0"/>
                <field name="NoPartyUpdates" id="1676" type="NoPartyUpdates"/>
                <field name="RequestResult" id="1511" type="uint32"/>
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
        "constant-presence regression example should compile and run"
    );
}

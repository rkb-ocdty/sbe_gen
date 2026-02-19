use std::collections::HashMap;

use sbe_gen::{GeneratorOptions, generate};

#[test]
fn generates_basic_schema() {
    let xml = r#"
        <messageSchema package="test">
            <types>
                <enum name="Side" encodingType="char">
                    <validValue name="Buy">B</validValue>
                    <validValue name="Sell">S</validValue>
                </enum>
            </types>
            <message name="Order" id="1" blockLength="12">
                <field name="id" id="1" type="uint64" />
                <field name="side" id="2" type="Side" />
            </message>
        </messageSchema>
    "#;

    let modules = generate(xml, &GeneratorOptions::default()).expect("schema should parse");
    let module_map: HashMap<_, _> = modules.into_iter().collect();

    let mod_rs = module_map.get("mod.rs").expect("mod.rs emitted");
    assert!(mod_rs.contains("pub mod order;\n"));
    assert!(mod_rs.contains("pub use order::Order;"));

    let types_rs = module_map.get("types.rs").expect("types.rs emitted");
    assert!(types_rs.contains("pub struct Side"));
    assert!(types_rs.contains("pub const Buy: Self"));
    assert!(types_rs.contains("pub const Sell: Self"));

    let order_rs = module_map.get("order.rs").expect("order.rs emitted");
    assert!(order_rs.contains("pub struct Order"));
    assert!(order_rs.contains("pub id: U64"));
    assert!(order_rs.contains("pub side: Side"));
    assert!(order_rs.contains("pub fn parse_prefix"));
}

#[test]
fn generates_groups_and_var_data() {
    let xml = r#"
        <messageSchema package="test">
            <types>
                <composite name="groupSize">
                    <type name="blockLength" primitiveType="uint16"/>
                    <type name="numInGroup" primitiveType="uint16"/>
                </composite>
                <type name="varStringEncoding" primitiveType="uint8"/>
            </types>
            <message name="Book" id="1" blockLength="4">
                <field name="seq" id="1" type="uint32" />
                <group name="Levels" id="2" blockLength="16" dimensionType="groupSize">
                    <field name="price" id="1" type="int64" />
                    <field name="qty" id="2" type="int64" />
                    <data name="note" id="3" type="varStringEncoding" />
                </group>
                <data name="raw" id="4" type="varStringEncoding" />
            </message>
        </messageSchema>
    "#;

    let modules = generate(xml, &GeneratorOptions::default()).expect("schema should parse");
    let module_map: HashMap<_, _> = modules.into_iter().collect();

    let book_rs = module_map.get("book.rs").expect("book.rs emitted");
    assert!(book_rs.contains("pub fn parse_levels"));
    assert!(book_rs.contains("pub struct LevelsIter"));
    assert!(book_rs.contains("pub struct LevelsEntry"));
    assert!(book_rs.contains("LevelsEntryView"));
    assert!(book_rs.contains("VarData<'a>"));
    assert!(book_rs.contains("pub fn parse_raw"));
    assert!(module_map.contains_key("message_header.rs"));
}

#[test]
fn optional_fields_expose_option_helpers() {
    let xml = r#"
        <messageSchema package="test">
            <types>
                <composite name="Comp">
                    <type name="a" primitiveType="int32" />
                    <type name="b" primitiveType="int32" />
                </composite>
            </types>
            <message name="Opt" id="1" blockLength="12">
                <field name="req" id="1" type="uint32" presence="required" />
                <field name="maybe_price" id="2" type="int64" presence="optional" />
            </message>
        </messageSchema>
    "#;

    let modules = generate(xml, &GeneratorOptions::default()).expect("schema should parse");
    let module_map: HashMap<_, _> = modules.into_iter().collect();
    let opt_rs = module_map.get("opt.rs").expect("opt.rs emitted");
    assert!(opt_rs.contains("pub maybe_price: I64"));
    assert!(opt_rs.contains("pub fn maybe_price_opt"));
    assert!(opt_rs.contains("#[inline]\n    pub fn maybe_price_opt"));
    assert!(opt_rs.contains("#[inline]\n    pub fn has_req"));
    assert!(opt_rs.contains("#[inline]\n    pub fn req(&self) -> Option<&U32>"));
    assert!(opt_rs.contains("MAYBE_PRICE_SINCE_VERSION"));
}

#[test]
fn generates_nested_groups() {
    let xml = r#"
        <messageSchema package="test">
            <types>
                <composite name="groupSize">
                    <type name="blockLength" primitiveType="uint16"/>
                    <type name="numInGroup" primitiveType="uint16"/>
                </composite>
            </types>
            <message name="Outer" id="1" blockLength="4">
                <field name="seq" id="1" type="uint32" />
                <group name="Parents" id="2" blockLength="8" dimensionType="groupSize">
                    <field name="id" id="1" type="uint64" />
                    <group name="Children" id="3" blockLength="4" dimensionType="groupSize">
                        <field name="child_id" id="1" type="uint32" />
                    </group>
                </group>
            </message>
        </messageSchema>
    "#;

    let modules = generate(xml, &GeneratorOptions::default()).expect("schema should parse");
    let module_map: HashMap<_, _> = modules.into_iter().collect();
    let outer_rs = module_map.get("outer.rs").expect("outer.rs emitted");
    assert!(outer_rs.contains("parse_parents"));
    assert!(outer_rs.contains("ParentsGroup"));
    assert!(outer_rs.contains("parse_children"));
    assert!(outer_rs.contains("ChildrenGroup"));
    assert!(outer_rs.contains("skip_children"));
    assert!(outer_rs.contains("ParentsEntryView"));
    assert!(outer_rs.contains("pub children: ChildrenGroup"));
    assert!(outer_rs.contains("SINCE_VERSION"));
}

#[test]
fn parse_fallback_uses_borrowed_raw_slices_without_heap_allocations() {
    let xml = r#"
        <messageSchema package="test">
            <types>
                <composite name="groupSize">
                    <type name="blockLength" primitiveType="uint16"/>
                    <type name="numInGroup" primitiveType="uint8"/>
                </composite>
            </types>
            <message name="Evolving" id="1" blockLength="8">
                <field name="seq" id="1" type="uint32" offset="0" />
                <field name="price" id="2" type="uint32" offset="4" />
                <group name="Entries" id="3" blockLength="8" dimensionType="groupSize">
                    <field name="qty" id="1" type="uint32" offset="0" />
                    <field name="px" id="2" type="uint32" offset="4" />
                </group>
            </message>
        </messageSchema>
    "#;

    let modules = generate(xml, &GeneratorOptions::default()).expect("schema should parse");
    let module_map: HashMap<_, _> = modules.into_iter().collect();
    let msg_rs = module_map.get("evolving.rs").expect("evolving.rs emitted");

    assert!(msg_rs.contains("pub struct EvolvingBody<'a>"));
    assert!(msg_rs.contains("pub struct EntriesEntryBody<'a>"));
    assert!(msg_rs.contains("let raw = if acting_block_length >= needed"));
    assert!(msg_rs.contains("let raw = if block_length >= needed"));
    assert!(!msg_rs.contains("Owned(Vec<u8>)"));
    assert!(!msg_rs.contains("vec![0u8; needed]"));
    assert!(msg_rs.contains("message body shorter than current layout; use accessor methods"));
    assert!(msg_rs.contains("group entry shorter than current layout; use accessor methods"));
}

#[test]
fn byte_order_and_offsets() {
    let xml = r#"
        <messageSchema package="test">
            <message name="Endian" id="1" blockLength="8">
                <field name="a" id="1" type="uint16" offset="0" />
                <field name="b" id="2" type="uint16" offset="2" byteOrder="big" />
                <field name="c" id="3" type="uint32" offset="4" />
            </message>
        </messageSchema>
    "#;

    let modules = generate(xml, &GeneratorOptions::default()).expect("schema should parse");
    let module_map: HashMap<_, _> = modules.into_iter().collect();
    let endian_rs = module_map.get("endian.rs").expect("endian.rs emitted");
    assert!(endian_rs.contains("pub a: U16"));
    assert!(endian_rs.contains("pub b: zerocopy::byteorder::big_endian::U16"));
    assert!(endian_rs.contains("pub c: U32"));
    assert!(endian_rs.contains("A_OFFSET: u32 = 0"));
    assert!(endian_rs.contains("B_OFFSET: u32 = 2"));
    assert!(endian_rs.contains("C_OFFSET: u32 = 4"));
}

#[test]
fn type_level_constants_are_not_encoded() {
    let xml = r#"
        <messageSchema package="test">
            <types>
                <type name="ConstU16" primitiveType="uint16" presence="constant">7</type>
            </types>
            <message name="ConstMsg" id="1">
                <field name="magic" id="1" type="ConstU16" />
                <field name="seq" id="2" type="uint32" />
            </message>
        </messageSchema>
    "#;

    let modules = generate(xml, &GeneratorOptions::default()).expect("schema should parse");
    let module_map: HashMap<_, _> = modules.into_iter().collect();
    let msg_rs = module_map
        .get("const_msg.rs")
        .expect("const_msg.rs emitted");
    assert!(!msg_rs.contains("pub magic:"));
    assert!(msg_rs.contains("pub const MAGIC: ConstU16"));
    assert!(msg_rs.contains("U16::new(7)"));
    assert!(msg_rs.contains("pub const BLOCK_LENGTH: u16 = 4"));
}

#[test]
fn byte_order_override_applies_to_type_aliases() {
    let xml = r#"
        <messageSchema package="test">
            <types>
                <type name="Price" primitiveType="uint16"/>
            </types>
            <message name="AliasEndian" id="1" blockLength="2">
                <field name="price" id="1" type="Price" byteOrder="big" />
            </message>
        </messageSchema>
    "#;

    let modules = generate(xml, &GeneratorOptions::default()).expect("schema should parse");
    let module_map: HashMap<_, _> = modules.into_iter().collect();
    let alias_rs = module_map
        .get("alias_endian.rs")
        .expect("alias_endian.rs emitted");
    assert!(alias_rs.contains("pub price: zerocopy::byteorder::big_endian::U16"));
}

#[test]
fn generates_borrowed_encode_into_api() {
    let xml = r#"
        <messageSchema package="test">
            <types>
                <type name="varStringEncoding" primitiveType="uint8"/>
            </types>
            <message name="Negotiate500" id="500" blockLength="12">
                <field name="seq" id="1" type="uint32" offset="0" />
                <field name="firm" id="2" type="uint64" offset="4" />
                <data name="credentials" id="3" type="varStringEncoding" />
            </message>
        </messageSchema>
    "#;

    let modules = generate(xml, &GeneratorOptions::default()).expect("schema should parse");
    let module_map: HashMap<_, _> = modules.into_iter().collect();
    let msg_rs = module_map
        .get("negotiate500.rs")
        .expect("negotiate500.rs emitted");
    assert!(msg_rs.contains("pub struct Negotiate500Encoder<'a>"));
    assert!(msg_rs.contains("pub enum EncodeIntoError"));
    assert!(msg_rs.contains("pub fn encode_body_into"));
    assert!(msg_rs.contains("pub fn encode_with_header_into"));
    assert!(!msg_rs.contains("does not support group encoding"));
}

#[test]
fn constant_fields_are_not_writable_in_builder_or_encoder() {
    let xml = r#"
        <messageSchema package="test">
            <types>
                <type name="ClientFlowType" presence="constant" length="10" primitiveType="char">IDEMPOTENT</type>
            </types>
            <message name="Establish503" id="503" blockLength="4">
                <field name="CustomerFlow" id="1" type="ClientFlowType"/>
                <field name="seq" id="2" type="uint32" offset="0" />
            </message>
        </messageSchema>
    "#;

    let modules = generate(xml, &GeneratorOptions::default()).expect("schema should parse");
    let module_map: HashMap<_, _> = modules.into_iter().collect();
    let msg_rs = module_map
        .get("establish503.rs")
        .expect("establish503.rs emitted");
    assert!(!msg_rs.contains("pub fn customer_flow(&mut self, value:"));
    assert!(msg_rs.contains("pub fn customer_flow(&self) -> ClientFlowType"));
    assert!(msg_rs.contains("#[inline]\n    pub fn customer_flow(&self) -> ClientFlowType"));
    assert!(msg_rs.contains("pub const CUSTOMER_FLOW: ClientFlowType"));
}

#[test]
fn field_level_constant_literals_are_not_encoded() {
    let xml = r#"
        <messageSchema package="test">
            <message name="ConstLiteral" id="1" blockLength="4">
                <field name="mode" id="1" type="uint8" presence="constant">7</field>
                <field name="seq" id="2" type="uint32" offset="0" />
            </message>
        </messageSchema>
    "#;

    let modules = generate(xml, &GeneratorOptions::default()).expect("schema should parse");
    let module_map: HashMap<_, _> = modules.into_iter().collect();
    let msg_rs = module_map
        .get("const_literal.rs")
        .expect("const_literal.rs emitted");
    assert!(!msg_rs.contains("pub mode:"));
    assert!(!msg_rs.contains("pub fn mode(&mut self, value:"));
    assert!(msg_rs.contains("pub const MODE: u8 = 7;"));
    assert!(msg_rs.contains("#[inline]\n    pub fn mode(&self) -> u8"));
    assert!(msg_rs.contains("pub const BLOCK_LENGTH: u16 = 4"));
}

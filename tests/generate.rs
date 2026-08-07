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
    let module_map: HashMap<_, _> = modules
        .modules()
        .map(|m| (m.name.clone(), m.source.clone()))
        .collect();

    let mod_rs = module_map.get("mod.rs").expect("mod.rs emitted");
    assert!(mod_rs.contains("pub mod order;\n"));
    assert!(mod_rs.contains("pub use order::Order;"));

    let types_rs = module_map.get("types.rs").expect("types.rs emitted");
    assert!(types_rs.contains("pub struct Side"));
    assert!(types_rs.contains("pub const BUY: Self"));
    assert!(types_rs.contains("pub const SELL: Self"));

    let order_rs = module_map.get("order.rs").expect("order.rs emitted");
    assert!(order_rs.contains("pub struct Order"));
    assert!(order_rs.contains("pub id: U64"));
    assert!(order_rs.contains("pub side: crate::types::Side"));
    assert!(order_rs.contains("#[sbe_gen("));
}

#[test]
fn char_encoded_enum_values_use_byte_literals() {
    let xml = r#"
        <messageSchema package="test">
            <types>
                <enum name="MDEntryTypeBook" encodingType="char">
                    <validValue name="Bid" description="Bid">0</validValue>
                    <validValue name="Offer" description="Offer">1</validValue>
                    <validValue name="ImpliedBid" description="Implied Bid">E</validValue>
                    <validValue name="ImpliedOffer" description="Implied Offer">F</validValue>
                    <validValue name="BookReset" description="Book Reset">J</validValue>
                </enum>
            </types>
            <message name="Tick" id="1" blockLength="1">
                <field name="kind" id="1" type="MDEntryTypeBook" />
            </message>
        </messageSchema>
    "#;

    let modules = generate(xml, &GeneratorOptions::default()).expect("schema should parse");
    let module_map: HashMap<_, _> = modules
        .modules()
        .map(|m| (m.name.clone(), m.source.clone()))
        .collect();
    let types_rs = module_map.get("types.rs").expect("types.rs emitted");

    // Variants are emitted using byte-literal syntax.
    assert!(types_rs.contains("Bid = b'0'"));
    assert!(types_rs.contains("Offer = b'1'"));
    assert!(types_rs.contains("ImpliedBid = b'E'"));
    assert!(types_rs.contains("BookReset = b'J'"));

    // Associated constants and the as_enum match arms switch over the same form.
    assert!(types_rs.contains("pub const BID: Self = Self((b'0') as u8);"));
    assert!(types_rs.contains("b'0' => Some(MDEntryTypeBookEnum::Bid)"));
    assert!(types_rs.contains("b'J' => Some(MDEntryTypeBookEnum::BookReset)"));

    // Sanity: the old numeric form for these printable chars is gone.
    assert!(!types_rs.contains("Bid = 48u8"));
    assert!(!types_rs.contains("48u8 => Some(MDEntryTypeBookEnum::Bid)"));
}

#[test]
fn rejects_unsupported_field_types_instead_of_silent_drops() {
    let xml = r#"
        <messageSchema package="test">
            <message name="BadMsg" id="1">
                <field name="broken" id="1" type="uint128" />
            </message>
        </messageSchema>
    "#;

    let err = generate(xml, &GeneratorOptions::default()).expect_err("unsupported type must fail");
    let msg = err.to_string();
    assert!(msg.contains("unsupported field type 'uint128'"));
    assert!(msg.contains("message 'BadMsg' field 'broken'"));
}

#[test]
fn rejects_unsupported_var_data_length_types() {
    let xml = r#"
        <messageSchema package="test">
            <message name="BadData" id="1">
                <data name="blob" id="1" type="UnknownLenType" />
            </message>
        </messageSchema>
    "#;

    let err = generate(xml, &GeneratorOptions::default()).expect_err("unsupported data type");
    let msg = err.to_string();
    assert!(msg.contains("unsupported var-data length type 'UnknownLenType'"));
    assert!(msg.contains("message 'BadData' data 'blob'"));
}

#[test]
fn rejects_unknown_group_dimension_types() {
    let xml = r#"
        <messageSchema package="test">
            <message name="BadGroup" id="1">
                <group name="Items" id="1" dimensionType="UnknownSize">
                    <field name="x" id="1" type="uint8" />
                </group>
            </message>
        </messageSchema>
    "#;

    let err =
        generate(xml, &GeneratorOptions::default()).expect_err("unknown dimension type must fail");
    let msg = err.to_string();
    assert!(msg.contains("dimensionType 'UnknownSize'"));
    assert!(msg.contains("is not declared"));
    assert!(msg.contains("message 'BadGroup' group 'Items'"));
}

#[test]
fn rejects_non_composite_group_dimension_types() {
    let xml = r#"
        <messageSchema package="test">
            <types>
                <type name="NotComposite" primitiveType="uint16"/>
            </types>
            <message name="BadGroup" id="1">
                <group name="Items" id="1" dimensionType="NotComposite">
                    <field name="x" id="1" type="uint8" />
                </group>
            </message>
        </messageSchema>
    "#;

    let err = generate(xml, &GeneratorOptions::default())
        .expect_err("non-composite dimension type must fail");
    let msg = err.to_string();
    assert!(msg.contains("dimensionType 'NotComposite'"));
    assert!(msg.contains("must be a composite"));
}

#[test]
fn rejects_group_dimension_types_with_too_few_fields() {
    let xml = r#"
        <messageSchema package="test">
            <types>
                <composite name="ShortDim">
                    <type name="blockLength" primitiveType="uint16"/>
                </composite>
            </types>
            <message name="BadGroup" id="1">
                <group name="Items" id="1" dimensionType="ShortDim">
                    <field name="x" id="1" type="uint8" />
                </group>
            </message>
        </messageSchema>
    "#;

    let err = generate(xml, &GeneratorOptions::default())
        .expect_err("dimension headers with one field must fail");
    let msg = err.to_string();
    assert!(msg.contains("dimensionType 'ShortDim'"));
    assert!(msg.contains("must expose at least two usable integer fields"));
}

#[test]
fn rejects_message_name_collisions_after_sanitization() {
    let xml = r#"
        <messageSchema package="test">
            <message name="type" id="1"/>
            <message name="type_" id="2"/>
        </messageSchema>
    "#;

    let err = generate(xml, &GeneratorOptions::default()).expect_err("colliding names must fail");
    let msg = err.to_string();
    assert!(msg.contains("identifier collision"));
    assert!(msg.contains("type_'"));
}

#[test]
fn rejects_view_helper_collisions_after_sanitization() {
    let xml = r#"
        <messageSchema package="test">
            <message name="Collision" id="1" blockLength="8">
                <field name="qty" id="1" type="uint32" offset="0"/>
                <field name="qty_value" id="2" type="uint32" offset="4"/>
            </message>
        </messageSchema>
    "#;

    let err = generate(xml, &GeneratorOptions::default()).expect_err("helper collision must fail");
    let msg = err.to_string();
    assert!(msg.contains("identifier collision"));
    assert!(msg.contains("qty"));
    assert!(msg.contains("qty_value"));
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
    let module_map: HashMap<_, _> = modules
        .modules()
        .map(|m| (m.name.clone(), m.source.clone()))
        .collect();

    let book_rs = module_map.get("book.rs").expect("book.rs emitted");
    assert!(book_rs.contains("dimension = crate::types::groupSize"));
    assert!(book_rs.contains("dimension = crate::types::groupSize"));
    assert!(book_rs.contains("pub struct LevelsEntry"));
    assert!(book_rs.contains("pub struct LevelsEntry"));
    assert!(book_rs.contains("VarData<'a>"));
    assert!(book_rs.contains("pub fn parse_raw"));
    // prettyplease wraps the arguments, so the alias is matched a piece at a time
    assert!(book_rs.contains("ty = Levels,"));
    assert!(book_rs.contains("crate::types::groupSize,"));
    assert!(book_rs.contains("pub struct LevelsEntry"));
    // the type lives in sbe_support, but the module stays as a re-export because consumers
    // import it by path
    assert!(
        module_map
            .get("message_header.rs")
            .expect("message_header.rs emitted")
            .contains("pub use sbe_support::MessageHeader;")
    );
}

#[test]
fn generates_8_byte_aligned_group_size_composites() {
    let xml = r#"
        <messageSchema package="test">
            <types>
                <composite name="groupSize8Byte">
                    <type name="blockLength" primitiveType="uint16"/>
                    <type name="numInGroup" primitiveType="uint8" offset="7"/>
                </composite>
            </types>
            <message name="Counted" id="1" blockLength="0">
                <group name="Items" id="2" blockLength="1" dimensionType="groupSize8Byte">
                    <field name="x" id="1" type="uint8" />
                </group>
            </message>
        </messageSchema>
    "#;

    let modules = generate(xml, &GeneratorOptions::default()).expect("schema should parse");
    let module_map: HashMap<_, _> = modules
        .modules()
        .map(|m| (m.name.clone(), m.source.clone()))
        .collect();
    let types_rs = module_map.get("types.rs").expect("types.rs emitted");
    let counted_rs = module_map.get("counted.rs").expect("counted.rs emitted");

    assert!(types_rs.contains("pub struct groupSize8Byte"));
    assert!(types_rs.contains("__padding0: [u8; 5usize]"));
    assert!(types_rs.contains("pub num_in_group: u8"));
    // where the two members sit is the dimension's own business now, so it is asserted there
    assert!(types_rs.contains("core::mem::offset_of!(Self, num_in_group)"));
    assert!(counted_rs.contains("dimension = crate::types::groupSize8Byte"));
    assert!(counted_rs.contains("crate::types::groupSize8Byte,"));
    assert!(types_rs.contains("impl sbe_support::Dimension for groupSize8Byte"));
}

#[test]
fn rejects_overlapping_composite_offsets() {
    let xml = r#"
        <messageSchema package="test">
            <types>
                <composite name="BadLayout">
                    <type name="blockLength" primitiveType="uint16"/>
                    <type name="numInGroup" primitiveType="uint8" offset="1"/>
                </composite>
            </types>
            <message name="Counted" id="1" blockLength="0">
                <group name="Items" id="2" blockLength="1" dimensionType="BadLayout">
                    <field name="x" id="1" type="uint8" />
                </group>
            </message>
        </messageSchema>
    "#;

    let err = generate(xml, &GeneratorOptions::default()).expect_err("invalid layout must fail");
    let msg = err.to_string();
    assert!(msg.contains("composite 'BadLayout'"));
    assert!(msg.contains("starts at 1"));
    assert!(msg.contains("previous layout ends at 2"));
}

#[test]
fn var_data_length_composite_refs_are_supported() {
    let xml = r#"
        <messageSchema package="test">
            <types>
                <type name="LenRef" primitiveType="uint16"/>
                <composite name="VarRefEncoding">
                    <ref name="length" type="LenRef"/>
                    <type name="varData" primitiveType="uint8" length="0"/>
                </composite>
            </types>
            <message name="HasData" id="1" blockLength="0">
                <data name="payload" id="1" type="VarRefEncoding" />
            </message>
        </messageSchema>
    "#;

    let modules = generate(xml, &GeneratorOptions::default()).expect("schema should parse");
    let module_map: HashMap<_, _> = modules
        .modules()
        .map(|m| (m.name.clone(), m.source.clone()))
        .collect();
    let msg_rs = module_map.get("has_data.rs").expect("has_data.rs emitted");
    assert!(msg_rs.contains("parse_var_data::<U16>(buf)"));
    assert!(msg_rs.contains("write_var_data::<U16>(&mut self.buf, bytes)"));
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
    let module_map: HashMap<_, _> = modules
        .modules()
        .map(|m| (m.name.clone(), m.source.clone()))
        .collect();
    let opt_rs = module_map.get("opt.rs").expect("opt.rs emitted");
    assert!(opt_rs.contains("pub maybe_price: I64"));
    // the accessors are the macro's, off this metadata
    // the accessors are the macro's, off this metadata
    assert!(opt_rs.contains("optional(null = i64::MIN)"));
    assert!(opt_rs.contains("pub req: U32"));
    // the MAYBE_PRICE_* constants are the macro's, off this field's own metadata
    // an optional field's setter takes the host integer, so the generator keeps that one
    assert!(opt_rs.contains(r#"value(ty = "i64", read = "get")"#));
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
    let module_map: HashMap<_, _> = modules
        .modules()
        .map(|m| (m.name.clone(), m.source.clone()))
        .collect();
    let outer_rs = module_map.get("outer.rs").expect("outer.rs emitted");
    assert!(outer_rs.contains("ty = Parents,"));
    assert!(outer_rs.contains("dimension = "));
    assert!(outer_rs.contains("ty = Children,"));
    assert!(outer_rs.contains("ChildrenGroup"));
    assert!(outer_rs.contains("ty = Children,"));
    assert!(outer_rs.contains("pub struct ParentsEntry"));
    assert!(outer_rs.contains("ty = Children,"));
    assert!(outer_rs.contains("at = "));
}

#[test]
fn value_ref_constants_use_generated_enum_constant_names() {
    let xml = r#"
        <messageSchema package="test">
            <types>
                <enum name="Side" encodingType="char">
                    <validValue name="Buy">B</validValue>
                    <validValue name="Sell">S</validValue>
                </enum>
            </types>
            <message name="Order" id="1" blockLength="0">
                <field name="ConstSide" id="1" type="Side" valueRef="Side.Buy"/>
            </message>
        </messageSchema>
    "#;

    let modules = generate(xml, &GeneratorOptions::default()).expect("schema should parse");
    let module_map: HashMap<_, _> = modules
        .modules()
        .map(|m| (m.name.clone(), m.source.clone()))
        .collect();
    let order_rs = module_map.get("order.rs").expect("order.rs emitted");
    assert!(order_rs.contains("ident = CONST_SIDE"));
}

#[test]
fn constant_type_alias_paths_are_emitted_verbatim() {
    let xml = r#"
        <messageSchema package="test">
            <types>
                <type name="FooConst" primitiveType="uint8" presence="constant">7</type>
            </types>
            <message name="Order" id="1" blockLength="0">
                <field name="foo" id="1" type="FooConst"/>
            </message>
        </messageSchema>
    "#;

    let mut opts = GeneratorOptions::default();
    opts.constant_type_aliases
        .insert("FooConst".into(), "crate::ext::Type".into());
    let modules = generate(xml, &opts).expect("schema should parse");
    let module_map: HashMap<_, _> = modules
        .modules()
        .map(|m| (m.name.clone(), m.source.clone()))
        .collect();
    let types_rs = module_map.get("types.rs").expect("types.rs emitted");
    assert!(types_rs.contains("pub type FooConst = crate::ext::Type;"));
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
    let module_map: HashMap<_, _> = modules
        .modules()
        .map(|m| (m.name.clone(), m.source.clone()))
        .collect();
    let msg_rs = module_map.get("evolving.rs").expect("evolving.rs emitted");

    assert!(msg_rs.contains("#[sbe_gen(\n    message,"));
    assert!(msg_rs.contains("dimension = "));
    assert!(msg_rs.contains(
        "offset = 0u32"
    ));
    assert!(msg_rs.contains("size = 8usize"));
    assert!(!msg_rs.contains("Owned(Vec<u8>)"));
    assert!(!msg_rs.contains("vec![0u8; needed]"));
    // the fallback is sbe_support::EntryBody's now, so the module must not hand-roll one
    assert!(!msg_rs.contains("parsed: Option<&'a Evolving>"));
    assert!(!msg_rs.contains("parsed: Option<&'a EntriesEntry>"));
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
    let module_map: HashMap<_, _> = modules
        .modules()
        .map(|m| (m.name.clone(), m.source.clone()))
        .collect();
    let msg_rs = module_map
        .get("const_msg.rs")
        .expect("const_msg.rs emitted");
    assert!(!msg_rs.contains("pub magic:"));
    assert!(msg_rs.contains("ident = MAGIC"));
    assert!(msg_rs.contains("ident = MAGIC"));
    assert!(msg_rs.contains("block_length = 4"));
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
    let module_map: HashMap<_, _> = modules
        .modules()
        .map(|m| (m.name.clone(), m.source.clone()))
        .collect();
    let msg_rs = module_map
        .get("negotiate500.rs")
        .expect("negotiate500.rs emitted");
    // Negotiate500Encoder comes off the struct now
    assert!(msg_rs.contains("#[sbe_gen(\n    message,"));
    assert!(msg_rs.contains(
        "MessageEncode, ParsePrefix"
    ));
    // both are sbe_support::MessageEncode's, provided off the three items the macro writes
    assert!(msg_rs.contains("MessageEncode"));
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
    let module_map: HashMap<_, _> = modules
        .modules()
        .map(|m| (m.name.clone(), m.source.clone()))
        .collect();
    let msg_rs = module_map
        .get("establish503.rs")
        .expect("establish503.rs emitted");
    assert!(!msg_rs.contains("pub fn customer_flow(&mut self, value:"));
    assert!(msg_rs.contains("name = customer_flow"));
    assert!(msg_rs.contains("ident = CUSTOMER_FLOW"));
    assert!(msg_rs.contains("ident = CUSTOMER_FLOW"));
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
    let module_map: HashMap<_, _> = modules
        .modules()
        .map(|m| (m.name.clone(), m.source.clone()))
        .collect();
    let msg_rs = module_map
        .get("const_literal.rs")
        .expect("const_literal.rs emitted");
    assert!(!msg_rs.contains("pub mode:"));
    assert!(!msg_rs.contains("pub fn mode(&mut self, value:"));
    assert!(msg_rs.contains(r#"ident = MODE, ty = "u8", value = "7""#));
    assert!(msg_rs.contains("name = mode"));
    assert!(msg_rs.contains("block_length = 4"));
}

#[test]
fn view_generates_fallback_value_required_enum_composite_and_string_helpers() {
    let xml = r#"
        <messageSchema package="test">
            <types>
                <type name="CHAR" primitiveType="char"/>
                <type name="Symbol6" primitiveType="char" length="6"/>
                <type name="uInt32NULL" primitiveType="uint32" nullValue="4294967295"/>
                <enum name="SecurityUpdateAction" encodingType="CHAR">
                    <validValue name="Add">A</validValue>
                    <validValue name="Delete">D</validValue>
                </enum>
                <composite name="PRICENULL9">
                    <type name="mantissa" primitiveType="int64" nullValue="9223372036854775807"/>
                </composite>
            </types>
            <message name="Def" id="1" blockLength="23">
                <field name="SecurityUpdateAction" id="1" type="SecurityUpdateAction" offset="0"/>
                <field name="Symbol" id="2" type="Symbol6" offset="1" semanticType="String"/>
                <field name="Qty" id="3" type="uInt32NULL" offset="7" presence="optional"/>
                <field name="TradingReferencePrice" id="4" type="PRICENULL9" offset="11"/>
                <field name="RequiredMaybe" id="5" type="uInt32NULL" offset="19" presence="required"/>
            </message>
        </messageSchema>
    "#;

    let modules = generate(xml, &GeneratorOptions::default()).expect("schema should parse");
    let module_map: HashMap<_, _> = modules
        .modules()
        .map(|m| (m.name.clone(), m.source.clone()))
        .collect();
    let def_rs = module_map.get("def.rs").expect("def.rs emitted");

    assert!(def_rs.contains(
        "MessageEncode, ParsePrefix"
    ));
    assert!(def_rs.contains("#[sbe_gen(\n    message,"));
    assert!(def_rs.contains("semantic_type = String"));
    // the plain getter is the macro's now; the value helper is still the generator's
    assert!(def_rs.contains("pub security_update_action: crate::types::SecurityUpdateAction"));
    // the value helper is the macro's, off this field's own read
    assert!(def_rs.contains(r#"value(ty = "crate :: types :: SecurityUpdateAction", read = "plain")"#));
    assert!(def_rs.contains("required"));
    // the string helpers are the macro's, off the array length
    assert!(def_rs.contains("string = 6usize"));


    assert!(def_rs.contains("optional(null = 4294967295)"));
    assert!(
        def_rs.contains("pub fn required_maybe_required(&self) -> Result<u32, DecodeFieldError>")
    );
    assert!(def_rs.contains("pub fn trading_reference_price_mantissa_opt(&self) -> Option<i64>"));
}

#[test]
fn group_count_overflow_is_reported_without_panics() {
    let xml = r#"
        <messageSchema package="test">
            <types>
                <composite name="groupSize8">
                    <type name="blockLength" primitiveType="uint16"/>
                    <type name="numInGroup" primitiveType="uint8"/>
                </composite>
            </types>
            <message name="Counted" id="1" blockLength="0">
                <group name="Items" id="2" blockLength="1" dimensionType="groupSize8">
                    <field name="x" id="1" type="uint8" offset="0"/>
                </group>
            </message>
        </messageSchema>
    "#;

    let modules = generate(xml, &GeneratorOptions::default()).expect("schema should parse");
    let module_map: HashMap<_, _> = modules
        .modules()
        .map(|m| (m.name.clone(), m.source.clone()))
        .collect();
    let counted_rs = module_map.get("counted.rs").expect("counted.rs emitted");
    let types_rs = module_map.get("types.rs").expect("types.rs emitted");

    // the ceiling is the dimension's integer width, and the check that enforces it is
    // sbe_support::GroupBuilder's
    assert!(types_rs.contains("const MAX_COUNT: usize = u8::MAX as usize;"));
    // the group method is the macro's, off the block's own list
    assert!(counted_rs.contains("ty = Items,"));
    assert!(!counted_rs.contains("count fits in u8"));
}

#[test]
fn message_modules_use_qualified_schema_types() {
    let xml = r#"
        <messageSchema package="test">
            <types>
                <composite name="MessageHeader">
                    <type name="marker" primitiveType="uint8"/>
                </composite>
            </types>
            <message name="Order" id="1" blockLength="1">
                <field name="hdr" id="1" type="MessageHeader" offset="0"/>
            </message>
        </messageSchema>
    "#;

    let modules = generate(xml, &GeneratorOptions::default()).expect("schema should parse");
    let module_map: HashMap<_, _> = modules
        .modules()
        .map(|m| (m.name.clone(), m.source.clone()))
        .collect();
    let order_rs = module_map.get("order.rs").expect("order.rs emitted");

    assert!(!order_rs.contains("use crate::types::*;"));
    // the setter is the derive's, off this declaration, so the qualified path has to be here
    assert!(order_rs.contains("pub hdr: crate::types::MessageHeader"));
    // parse_with_header is the macro's, off the message marker
    assert!(order_rs.contains("#[sbe_gen(\n    message,"));
}

#[test]
fn rejects_types_nested_past_the_size_resolver_guard() {
    // a composite chain deeper than the resolver's recursion guard. every type resolves, but
    // the size does not, so nothing downstream can be given an offset. the composite layout
    // check reaches this before field placement does, which is why placement's own
    // "cannot compute the encoded size" error is unreachable in practice.
    let xml = r#"
        <messageSchema package="test">
            <types>
                <composite name="C0"><ref name="inner" type="C1"/></composite>
                <composite name="C1"><ref name="inner" type="C2"/></composite>
                <composite name="C2"><ref name="inner" type="C3"/></composite>
                <composite name="C3"><ref name="inner" type="C4"/></composite>
                <composite name="C4"><ref name="inner" type="C5"/></composite>
                <composite name="C5"><ref name="inner" type="C6"/></composite>
                <composite name="C6"><ref name="inner" type="C7"/></composite>
                <composite name="C7"><ref name="inner" type="C8"/></composite>
                <composite name="C8"><ref name="inner" type="C9"/></composite>
                <composite name="C9"><ref name="inner" type="C10"/></composite>
                <composite name="C10"><ref name="inner" type="C11"/></composite>
                <composite name="C11"><ref name="inner" type="C12"/></composite>
                <composite name="C12"><type name="v" primitiveType="uint8"/></composite>
            </types>
            <message name="TooDeep" id="1">
                <field name="deep" id="1" type="C0"/>
            </message>
        </messageSchema>
    "#;

    let err = generate(xml, &GeneratorOptions::default()).expect_err("unsizeable field must fail");
    let msg = err.to_string();
    assert!(msg.contains("composite 'C0' field 'inner'"));
    assert!(msg.contains("has unsupported layout"));
}

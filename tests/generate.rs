use std::collections::HashMap;

use sbe_gen::{generate, GeneratorOptions};

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
    assert!(order_rs.contains("zc_parse_prefix!();"));
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
}

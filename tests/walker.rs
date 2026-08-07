use std::collections::HashMap;

use sbe_gen::{GeneratorOptions, generate};

#[test]
fn walks_schema_groups() {
    let xml = r#"
        <messageSchema package="test">
            <types>
                <enum name="Side" encodingType="char">
                    <validValue name="Buy">B</validValue>
                    <validValue name="Sell">S</validValue>
                </enum>
                <composite name="groupSize" description="Repeating group dimensions" semanticType="NumInGroup">
                    <type name="blockLength" primitiveType="uint16"/>
                    <type name="numInGroup" primitiveType="uint8"/>
                </composite>
            </types>
            <message name="Order" id="1" blockLength="12">
                <field name="id" id="1" type="uint64" />
                <field name="side" id="2" type="Side" />
                <group name="Group1" id="37705" description="Number of OrderID entries" blockLength="16" dimensionType="groupSize">
                    <field name="OrderID" id="37" type="uint64" description="Unique order identifier as assigned by the exchange" offset="0" semanticType="int"/>
                    <field name="LastQty" id="32" type="int32" description="Quantity bought or sold on this last fill" offset="8" semanticType="Qty"/>
                </group>
                <group name="Group2" id="37705" description="Number of OrderID entries" blockLength="16" dimensionType="groupSize">
                    <field name="OrderID" id="37" type="uint64" description="Unique order identifier as assigned by the exchange" offset="0" semanticType="int"/>
                    <field name="LastQty" id="32" type="int32" description="Quantity bought or sold on this last fill" offset="8" semanticType="Qty"/>
                </group>
            </message>
        </messageSchema>
    "#;

    let modules = generate(xml, &GeneratorOptions::default()).expect("schema should parse");
    let module_map: HashMap<_, _> = modules
        .modules()
        .map(|m| (m.name.clone(), m.source.clone()))
        .collect();

    let mod_rs = module_map.get("order.rs").expect("mod.rs emitted");

    println!("{mod_rs}");
}

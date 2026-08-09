use super::*;
use std::collections::HashSet;

/// A schema with the groups that more than one message declares picked out.
///
/// This runs before anything is resolved or placed, because the decision is about the XML: two
/// groups holding the same members generate the same code, so they should be one definition.
/// The parse tree is plain data, which is what lets `==` answer that.
#[derive(derive_more::Deref)]
pub struct DedupedSchema {
    #[deref]
    pub schema: Schema,
    /// declared by more than one message, so emitted once into `groups.rs`
    pub shared: Vec<Group>,
}

impl DedupedSchema {
    pub(crate) fn new(schema: Schema) -> Self {
        let all = Groups::default().visit_schema(&schema).all;
        // CME declares two different NoOrderIDEntries, and both of them repeat. Every name a
        // group generates is built from the group's name, so lifting both would put two
        // `parse_no_order_id_entries` in one module — they stay where they are.
        let ambiguous: HashSet<&Name> = all
            .iter()
            .enumerate()
            .flat_map(|(i, a)| all[i + 1..].iter().map(move |b| (a, b)))
            .filter(|(a, b)| a.name == b.name && a != b)
            .map(|(a, _)| &a.name)
            .collect();
        let mut shared = Vec::new();
        for g in &all {
            let repeats = all.iter().filter(|o| *o == g).count() > 1;
            if repeats && !ambiguous.contains(&g.name) && !shared.contains(g) {
                shared.push(g.clone());
            }
        }
        Self { schema, shared }
    }

    /// whether this group is one of the shared definitions
    pub(crate) fn is_shared(&self, group: &Group) -> bool {
        self.shared.contains(group)
    }
}

/// Finds the groups that repeat. Top-level only: a nested group's cluster calls its parent's
/// private `skip_` and `parse_*_entry_body`, so it cannot be lifted out on its own — and since
/// equality covers members recursively, a shared parent takes its children with it anyway.
#[derive(Default)]
struct Groups {
    all: Vec<Group>,
}

impl Groups {
    fn visit_schema(mut self, schema: &Schema) -> Self {
        for msg in &schema.messages {
            self.visit_groups(&msg.groups());
        }
        self
    }

    fn visit_groups(&mut self, groups: &[&Group]) {
        for group in groups {
            self.visit_group(group);
        }
    }

    fn visit_group(&mut self, group: &Group) {
        self.all.push(group.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shares_the_groups_the_instrument_definitions_repeat() {
        let xml = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/schemas/032_cme_mdp3.xml"
        ))
        .expect("cme fixture");
        let deduped = DedupedSchema::new(crate::parser::parse_schema(&xml).expect("parse"));

        let mut names: Vec<_> = deduped.shared.iter().map(|g| g.name.to_string()).collect();
        names.sort();
        assert_eq!(
            names,
            [
                "NoEvents",
                "NoInstAttrib",
                "NoLotTypeRules",
                "NoMDFeedTypes"
            ]
        );

        // the three NoMDEntries hold different fields, so none of them is shared
        assert!(!deduped.shared.iter().any(|g| &*g.name == "NoMDEntries"));
    }
}

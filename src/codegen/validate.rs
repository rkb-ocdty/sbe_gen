use super::*;
use derive_generic_visitor::{Continue, ControlFlow, Visit, Visitor};

/// Walks a type and everything it references, rejecting unknown types and cycles.
///
/// `Drive` generates the structural half — into a composite and over its members. The only
/// edge it cannot generate is a `ref`, which names a type rather than containing one; that
/// lookup is followed here, and the in-progress set that makes it terminate is the visitor's
/// state.
#[derive(Visit)]
#[visit(drive(CompositeField))]
#[visit(drive(for<T> Vec<T>, for<T> Option<T>))]
#[visit(skip(Name, Primitive, Presence, String, u32, usize, bool))]
pub(crate) struct TypeRefs<'sc> {
    schema: &'sc Schema,
    visiting: Vec<Name>,
    scope: String,
}

/// A schema that cannot be walked is invalid, so the walk breaks with the error.
impl Visitor for TypeRefs<'_> {
    type Break = CodegenError;
}

impl<'sc> TypeRefs<'sc> {
    pub(crate) fn new(schema: &'sc Schema) -> Self {
        Self {
            schema,
            visiting: Vec::new(),
            scope: String::new(),
        }
    }

    pub(crate) fn check(&mut self, ty: &Name, scope: &str) -> Result<(), CodegenError> {
        self.scope = scope.to_string();
        match self.walk(ty) {
            ControlFlow::Continue(()) => Ok(()),
            ControlFlow::Break(e) => Err(e),
        }
    }

    fn walk(&mut self, ty: &Name) -> ControlFlow<CodegenError> {
        if Primitive::parse(ty).is_some() {
            return Continue(());
        }
        if self.visiting.contains(ty) {
            let cycle: Vec<_> = self
                .visiting
                .iter()
                .chain([ty])
                .map(Name::to_string)
                .collect();
            return ControlFlow::Break(CodegenError::CyclicType {
                scope: self.scope.clone(),
                cycle: cycle.join(" -> "),
            });
        }
        let Some(def) = self.schema.types.get(&**ty) else {
            return ControlFlow::Break(CodegenError::UnknownType {
                ty: ty.clone(),
                scope: self.scope.clone(),
            });
        };

        self.visiting.push(ty.clone());
        let result = match def {
            // a declared primitiveType was resolved when the schema was parsed
            TypeDef::Primitive(PrimitiveDef { .. }) => Continue(()),
            TypeDef::Enum(EnumDef { encoding, .. }) | TypeDef::Set(SetDef { encoding, .. }) => {
                match encoding_primitive(encoding, &self.schema.types) {
                    Some(_) => Continue(()),
                    None => ControlFlow::Break(CodegenError::UnsupportedEncoding {
                        encoding: encoding.clone(),
                        ty: ty.clone(),
                        scope: self.scope.clone(),
                    }),
                }
            }
            TypeDef::Composite(CompositeDef { fields, .. }) => self.visit(fields),
        };
        self.visiting.pop();
        result
    }
}

/// The one edge `Drive` cannot follow: a member that names another type rather than
/// containing it. Written out rather than an `enter` hook because those return `()`, which
/// would silently drop the error this can produce.
impl<'a, 'sc> Visit<'a, CompositeKind> for TypeRefs<'sc> {
    fn visit(&mut self, kind: &'a CompositeKind) -> ControlFlow<CodegenError> {
        match kind {
            CompositeKind::Ref { ty } => self.walk(ty),
            CompositeKind::Type { .. } => Continue(()),
        }
    }
}

/// A schema whose declared types all resolve and whose references terminate.
///
/// Only obtainable by passing those checks, so holding one is the proof they ran. It carries
/// the options too, because every later pass needs both and never one without the other.
#[derive(derive_more::Deref)]
pub struct ValidatedSchema {
    #[deref]
    schema: DedupedSchema,
    pub opts: &'static GeneratorOptions,
}

impl ValidatedSchema {
    pub(crate) fn new(
        schema: DedupedSchema,
        opts: &'static GeneratorOptions,
    ) -> Result<Self, CodegenError> {
        let mut refs = TypeRefs::new(&schema);
        CodegenError::all(schema.types.values().filter_map(|def| match def {
            TypeDef::Composite(CompositeDef { name, .. }) => {
                refs.check(name, &format!("composite '{name}'")).err()
            }
            _ => None,
        }))?;
        Ok(Self { schema, opts })
    }
}

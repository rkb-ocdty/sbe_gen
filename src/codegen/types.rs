use super::*;
use derive_generic_visitor::{Continue, ControlFlow, Visit, Visitor};
use std::collections::BTreeMap;
use syn::{File, Item};

/// The `types.rs` items, kept in per-kind buckets so the template below decides the layout
/// rather than the schema's alphabetical order deciding it for us.
#[derive(Default)]
pub(crate) struct Rendered {
    pub(crate) primitives: Vec<Item>,
    pub(crate) enums: Vec<Item>,
    pub(crate) sets: Vec<Item>,
    pub(crate) composites: Vec<Item>,
}

/// A rendered declaration is usually several items (a struct plus its impls), so parse the
/// group and hand back the items rather than pretending each is one.
fn items(tokens: TokenStream) -> Vec<Item> {
    syn::parse2::<File>(tokens)
        .expect("sbe_gen rendered a malformed type declaration")
        .items
}

/// Walks the schema's type table, rendering each declaration into its bucket. The table is a
/// `BTreeMap`, so declaration order is already deterministic and needs no sorting here.
#[derive(Visit)]
#[visit(drive(Schema))]
#[visit(drive(for<T> Vec<T>, for<T> Option<T>))]
#[visit(skip(
    Name,
    Message,
    String,
    u32,
    usize,
    bool,
    CompositeField,
    Primitive,
    Presence,
    PrimitiveDef,
    EnumDef,
    SetDef,
    CompositeDef,
    NamedValue,
    Docs
))]
pub(crate) struct Types<'sc> {
    schema: &'sc ValidatedSchema,
    opts: &'sc GeneratorOptions,
    derivations: &'sc [&'sc dyn Derivation],
    used: HashSet<Name>,
    out: Rendered,
}

/// A derivation that rejects what it is handed stops the render, so the walk breaks with it.
impl Visitor for Types<'_> {
    type Break = CodegenError;
}

/// The only hook that has to be written out: `enter` drops the control flow, and a derivation
/// refusing a type has to stop the walk.
impl Visit<'_, TypeDef> for Types<'_> {
    fn visit(&mut self, def: &TypeDef) -> ControlFlow<CodegenError> {
        // the render pushes onto the end of its bucket, so what it just added is the tail
        let before = self.bucket(def).len();
        match def {
            TypeDef::Primitive(def) => self.visit_primitive(def),
            TypeDef::Enum(def) => self.visit_enum(def),
            TypeDef::Set(def) => self.visit_set(def),
            TypeDef::Composite(def) => self.visit_composite(def),
        }
        let derivations = self.derivations;
        let rendered = &mut self.bucket(def)[before..];
        for d in derivations {
            if let Err(e) = d.type_items(def, rendered) {
                return ControlFlow::Break(e);
            }
        }
        Continue(())
    }
}

impl<'sc> Types<'sc> {
    pub(crate) fn new(
        schema: &'sc ValidatedSchema,
        opts: &'sc GeneratorOptions,
        derivations: &'sc [&'sc dyn Derivation],
    ) -> Self {
        Self {
            schema,
            opts,
            derivations,
            used: UsedTypes::default().visit_by_val_infallible(&***schema).0,
            out: Rendered::default(),
        }
    }

    pub(crate) fn collect(self) -> Result<Rendered, CodegenError> {
        let schema: &Schema = self.schema;
        match self.visit_by_val(schema) {
            ControlFlow::Continue(types) => Ok(types.out),
            ControlFlow::Break(e) => Err(e),
        }
    }

    fn bucket(&mut self, def: &TypeDef) -> &mut Vec<Item> {
        match def {
            TypeDef::Primitive(_) => &mut self.out.primitives,
            TypeDef::Enum(_) => &mut self.out.enums,
            TypeDef::Set(_) => &mut self.out.sets,
            TypeDef::Composite(_) => &mut self.out.composites,
        }
    }

    fn visit_primitive(&mut self, def: &PrimitiveDef) {
        let PrimitiveDef {
            name,
            primitive,
            length,
            presence,
            null_value,
            constant,
            description,
        } = def;
        let type_name = name.type_ident();
        let prim = *primitive;
        let rust_type = Resolved::Scalar(prim);
        let rust_ty = rust_type.to_token_stream();
        let doc = description;
        let is_constant = *presence == Presence::Constant;
        let is_optional = *presence == Presence::Optional;
        let mut has_alias = false;
        if let Some(len) = length {
            let len = *len;
            self.out.primitives.extend(items(
                quote! { #doc pub type #type_name = [#rust_ty; #len]; },
            ));
            has_alias = true;
        } else if is_constant {
            if self.used.contains(name) {
                let alias = if let Some(path) = self.opts.constant_type_aliases.get(&**name) {
                    frag(path)
                } else if let Some(schema_ty) = self.schema.constant_type_alias_from_schema(name) {
                    schema_ty.type_ident().to_token_stream()
                } else {
                    rust_ty.clone()
                };
                self.out
                    .primitives
                    .extend(items(quote! { #doc pub type #type_name = #alias; }));
                has_alias = true;
            } else if let Some(value) = constant {
                let expr = rust_type.const_expr(value);
                self.out.primitives.extend(items(
                    quote! { #doc pub const #type_name: #rust_ty = #expr; },
                ));
            } else {
                let default = prim.default_literal();
                self.out.primitives.push(
                    parse_quote! { #doc pub const #type_name: #rust_ty = #default as #rust_ty; },
                );
            }
        } else {
            self.out
                .primitives
                .extend(items(quote! { #doc pub type #type_name = #rust_ty; }));
            has_alias = true;
        }

        if has_alias && !is_constant && (null_value.is_some() || is_optional) {
            let null_raw = null_value
                .as_deref()
                .or_else(|| (is_optional && length.is_none()).then(|| prim.null_name()));
            if let Some(null_raw) = null_raw {
                let cname = format_ident!("{}_NULL", name.const_ident());
                let expr = match length {
                    Some(len) => rust_type.const_array(null_raw, *len).map(|(_, expr)| expr),
                    None => rust_type.const_expr(null_raw),
                };
                if let Some(expr) = expr {
                    self.out
                        .primitives
                        .extend(items(quote! { pub const #cname: #type_name = #expr; }));
                }
            }
        }
    }

    fn visit_enum(&mut self, def: &EnumDef) {
        let EnumDef {
            name,
            encoding,
            values,
            description,
        } = def;
        let enum_ty = name.type_ident();
        let primitive =
            encoding_primitive(encoding, &self.schema.types).unwrap_or(Primitive::Uint8);
        let rust_type = Resolved::Scalar(primitive);
        let rust_ty = rust_type.to_token_stream();
        let repr_ty = primitive.host();
        let literal_of = |val: &str| primitive.enum_literal(val);
        let doc = description;
        let parse_prefix = parse_prefix_method();
        self.out.enums.extend(items(quote! {
            #doc
            #[repr(transparent)]
            #[derive(Debug, ::zerocopy::FromBytes, ::zerocopy::IntoBytes, ::zerocopy::KnownLayout, ::zerocopy::Immutable, ::zerocopy::Unaligned, Clone, Copy, PartialEq, Eq)]
            pub struct #enum_ty(pub #rust_ty);
            impl #enum_ty { #parse_prefix }
        }));
        if !values.is_empty() {
            let enum_name = format_ident!("{}Enum", name.type_ident());
            let variants = values.iter().map(
                |NamedValue {
                     name: vname,
                     value: val,
                     description: vdesc,
                 }| {
                    let variant_name = vname.variant_ident();
                    let literal = literal_of(val);
                    let vdoc = vdesc;
                    quote! { #vdoc #variant_name = #literal, }
                },
            );
            self.out.enums.extend(items(quote! {
                #doc
                #[repr(#repr_ty)]
                #[derive(Debug, Clone, Copy, PartialEq, Eq)]
                pub enum #enum_name { #(#variants)* }
            }));
            let consts = values.iter().map(
                |NamedValue {
                     name: vname,
                     value: val,
                     description: vdesc,
                 }| {
                    let const_name = vname.const_ident();
                    let encoded = rust_type.encode(primitive.enum_literal(val));
                    let vdoc = vdesc;
                    quote! { #vdoc pub const #const_name: Self = Self(#encoded); }
                },
            );
            let as_enum = match rust_type.read(quote!(self.0)) {
                Some(raw_expr) => {
                    let arms = values.iter().map(
                        |NamedValue {
                             name: vname,
                             value: val,
                             ..
                         }| {
                            let variant_name = vname.variant_ident();
                            let literal = literal_of(val);
                            quote! { #literal => Some(#enum_name::#variant_name), }
                        },
                    );
                    quote! {
                        #[inline]
                        pub fn as_enum(self) -> Option<#enum_name> {
                            let raw = #raw_expr;
                            match raw {
                                #(#arms)*
                                _ => None,
                            }
                        }
                    }
                }
                None => quote! {},
            };
            let encoded = rust_type.encode(quote!(raw));
            self.out.enums.extend(items(quote! {
                impl #enum_ty {
                    #(#consts)*
                    #as_enum
                }
                impl From<#enum_name> for #enum_ty {
                    fn from(v: #enum_name) -> Self {
                        let raw: #repr_ty = v as #repr_ty;
                        let encoded = #encoded;
                        Self(encoded)
                    }
                }
            }));
        }
    }

    fn visit_set(&mut self, def: &SetDef) {
        let SetDef {
            name,
            encoding,
            choices,
            description,
        } = def;
        let set_ty = name.type_ident();
        let primitive =
            encoding_primitive(encoding, &self.schema.types).unwrap_or(Primitive::Uint8);
        let rust_type = Resolved::Scalar(primitive);
        let rust_ty = rust_type.to_token_stream();
        let doc = description;
        let parse_prefix = parse_prefix_method();
        self.out.sets.extend(items(quote! {
            #doc
            #[repr(transparent)]
            #[derive(Debug, ::zerocopy::FromBytes, ::zerocopy::IntoBytes, ::zerocopy::KnownLayout, ::zerocopy::Immutable, ::zerocopy::Unaligned, Clone, Copy, PartialEq, Eq)]
            pub struct #set_ty(pub #rust_ty);
            impl #set_ty { #parse_prefix }
        }));
        if !choices.is_empty() {
            let consts = choices.iter().map(
                |NamedValue {
                     name: cname,
                     value: bit,
                     description: cdesc,
                 }| {
                    let choice_name = cname.const_ident();
                    // bit may be decimal or integer string
                    let bit_idx: u32 = bit.parse().unwrap_or(0);
                    let value = rust_type.bit(bit_idx);
                    let cdoc = cdesc;
                    quote! { #cdoc pub const #choice_name: Self = Self(#value); }
                },
            );
            self.out
                .sets
                .extend(items(quote! { impl #set_ty { #(#consts)* } }));
        }
    }

    fn visit_composite(&mut self, def: &CompositeDef) {
        let CompositeDef {
            name,
            fields,
            description,
        } = def;
        let composite_ty = name.type_ident();
        let doc = description;
        let mut struct_fields = TokenStream::new();
        let mut const_fields = Vec::new();
        let mut cur_offset: Option<usize> = Some(0);
        let mut pad_idx = 0usize;
        for f in fields {
            match f {
                CompositeField {
                    name: fname,
                    offset,
                    kind:
                        CompositeKind::Type {
                            primitive,
                            length,
                            presence,
                            constant,
                            description,
                            ..
                        },
                    ..
                } => {
                    if *presence == Presence::Constant {
                        if let Some(val) = constant {
                            const_fields.push((fname.clone(), *primitive, *length, val.clone()));
                        }
                        continue;
                    }
                    let field_offset = offset.map(|o| o as usize);
                    if let (Some(target), Some(cur)) = (field_offset, cur_offset)
                        && target > cur
                    {
                        let pad_name = format_ident!("__padding{}", pad_idx);
                        let pad = target - cur;
                        struct_fields.extend(quote! { #pad_name: [u8; #pad], });
                        pad_idx += 1;
                        cur_offset = Some(target);
                    }
                    if let Some(rust_type) = Some(Resolved::Scalar(*primitive)) {
                        let rust_ty = rust_type.to_token_stream();
                        let field_name = fname.snake_ident();
                        let fdoc = description;
                        if let Some(len) = length {
                            let len = *len;
                            struct_fields
                                .extend(quote! { #fdoc pub #field_name: [#rust_ty; #len], });
                        } else {
                            struct_fields.extend(quote! { #fdoc pub #field_name: #rust_ty, });
                        }
                        if let Some(sz) = composite_field_size(f, self.schema, 0, true) {
                            if let Some(cur) = cur_offset {
                                cur_offset = Some(cur + sz);
                            }
                        } else {
                            cur_offset = None;
                        }
                    }
                }
                CompositeField {
                    name: fname,
                    offset,
                    kind: CompositeKind::Ref { ty, .. },
                    ..
                } => {
                    let field_offset = offset.map(|o| o as usize);
                    if let (Some(target), Some(cur)) = (field_offset, cur_offset)
                        && target > cur
                    {
                        let pad_name = format_ident!("__padding{}", pad_idx);
                        let pad = target - cur;
                        struct_fields.extend(quote! { #pad_name: [u8; #pad], });
                        pad_idx += 1;
                        cur_offset = Some(target);
                    }
                    let field_name = fname.snake_ident();
                    let field_ty = ty.type_ident();
                    struct_fields.extend(quote! { pub #field_name: #field_ty, });
                    if let Some(sz) = composite_field_size(f, self.schema, 0, true) {
                        if let Some(cur) = cur_offset {
                            cur_offset = Some(cur + sz);
                        }
                    } else {
                        cur_offset = None;
                    }
                }
            }
        }
        let parse_prefix = parse_prefix_method();
        let dimension_impl = self.dimension_impl(&def.name, &composite_ty);
        self.out.composites.extend(items(quote! {
            #doc
            #[repr(C)]
            #[derive(Debug, ::zerocopy::FromBytes, ::zerocopy::IntoBytes, ::zerocopy::KnownLayout, ::zerocopy::Immutable, ::zerocopy::Unaligned, Clone, Copy)]
            #[derive(PartialEq, Eq)]
            pub struct #composite_ty { #struct_fields }
            impl #composite_ty { #parse_prefix }
            #dimension_impl
        }));
        let mut impl_body = TokenStream::new();
        impl_body.extend(
            const_fields
                .into_iter()
                .filter_map(|(fname, prim, len, val)| {
                    let rust_type = Resolved::Scalar(prim);
                    let cname = fname.const_ident();
                    let (ty, expr) = match len {
                        Some(len) if len > 1 => rust_type.const_array(&val, len)?,
                        _ => (rust_type.to_token_stream(), rust_type.const_expr(&val)?),
                    };
                    Some(quote! { pub const #cname: #ty = #expr; })
                }),
        );
        impl_body.extend(composite_null_constants(fields, self.schema));
        impl_body.extend(optional_methods_for_composite_fields(
            fields,
            self.schema,
            "self",
        ));
        if !impl_body.is_empty() {
            self.out
                .composites
                .extend(items(quote! { impl #composite_ty { #impl_body } }));
        }
    }

    /// A composite that some group names as its `dimensionType` gets the trait `Group` reads it
    /// through. It has to be written here, once, next to the type: the same composite is the
    /// dimension for groups in a dozen different modules, and a second impl anywhere would be a
    /// coherence error.
    fn dimension_impl(&self, name: &Name, ty: &Ident) -> Option<TokenStream> {
        fn names_it(groups: &[&Group], name: &Name) -> bool {
            groups
                .iter()
                .any(|g| g.dimension_type == *name || names_it(&g.groups(), name))
        }
        let used = self
            .schema
            .messages
            .iter()
            .any(|m| names_it(&m.groups(), name));
        let dim = used
            .then(|| dimension_fields(self.schema, name, "dimension").ok())
            .flatten()?;
        let (block_field, count_field) = (&dim.block_field, &dim.count_field);
        let block = dim.block_field_ty.read_usize(quote!(self.#block_field));
        let count = dim.count_field_ty.read_usize(quote!(self.#count_field));
        let max_count = dim.count_field_ty.max_count();
        let block_value = dim.block_field_ty.count_value(quote!(block_length));
        let count_value = dim.count_field_ty.count_value(quote!(count));
        Some(quote! {
            impl ::sbe_support::Dimension for #ty {
                const SIZE: usize = core::mem::size_of::<Self>();
                const MAX_COUNT: usize = #max_count;
                fn parse_header(body: &[u8]) -> Option<(&Self, &[u8])> { Self::parse_prefix(body) }
                fn count(&self) -> usize { #count }
                fn block_length(&self) -> usize { #block }
                fn write_block(buf: &mut Vec<u8>, at: usize, block_length: usize) {
                    ::sbe_support::write_bytes_at(buf, at + core::mem::offset_of!(Self, #block_field), &(#block_value));
                }
                fn write_count(buf: &mut Vec<u8>, at: usize, count: usize) {
                    ::sbe_support::write_bytes_at(buf, at + core::mem::offset_of!(Self, #count_field), &(#count_value));
                }
                fn write_block_into(dst: &mut [u8], at: usize, block_length: usize) -> Result<(), ::sbe_support::EncodeIntoError> {
                    ::sbe_support::write_bytes_into(dst, at + core::mem::offset_of!(Self, #block_field), &(#block_value))
                }
                fn write_count_into(dst: &mut [u8], at: usize, count: usize) -> Result<(), ::sbe_support::EncodeIntoError> {
                    ::sbe_support::write_bytes_into(dst, at + core::mem::offset_of!(Self, #count_field), &(#count_value))
                }
            }
        })
    }
}

// the crate drives Vec and Option but not maps
impl<'a, 'sc> Visit<'a, BTreeMap<Name, TypeDef>> for Types<'sc> {
    fn visit(&mut self, types: &'a BTreeMap<Name, TypeDef>) -> ControlFlow<Self::Break> {
        for def in types.values() {
            self.visit(def)?;
        }
        Continue(())
    }
}

fn optional_methods_for_composite_fields(
    fields: &[CompositeField],
    schema: &ValidatedSchema,
    self_expr: &str,
) -> TokenStream {
    let mut code = TokenStream::new();
    for field in fields {
        match field {
            CompositeField {
                name,
                kind:
                    CompositeKind::Type {
                        primitive,
                        length,
                        presence,
                        null_value,
                        ..
                    },
                ..
            } => {
                if field.is_constant() {
                    continue;
                }
                let is_optional = match presence {
                    Presence::Optional => true,
                    Presence::Required | Presence::Constant => false,
                    Presence::Inherited => null_value.is_some(),
                };
                if !is_optional {
                    continue;
                }
                if let Some(len) = length
                    && *len > 1
                {
                    continue;
                }
                let prim = *primitive;
                let read = frag(&format!("{}.{}", self_expr, name.snake_ident()));
                let Some(val_expr) = Resolved::Scalar(prim).read(read) else {
                    continue;
                };
                let Some(cond) = prim.null_cond(null_value.as_deref(), true) else {
                    continue;
                };
                let method_name = name.snake_ident().suffix_snake("opt");
                let host_type = prim.host();
                code.extend(quote! {
                    #[inline]
                    pub fn #method_name(&self) -> Option<#host_type> {
                        let raw = #val_expr;
                        if #cond { None } else { Some(raw) }
                    }
                });
            }
            CompositeField {
                name,
                kind: CompositeKind::Ref { ty, .. },
                ..
            } => {
                if !schema.type_is_optional(ty) {
                    continue;
                }
                if let Some(len) = schema.type_length(ty)
                    && len > 1
                {
                    continue;
                }
                let prim = match schema.type_primitive(ty) {
                    Some(p) => p,
                    None => continue,
                };
                let read = frag(&format!("{}.{}", self_expr, name.snake_ident()));
                let Some(val_expr) = Resolved::Scalar(prim).read(read) else {
                    continue;
                };
                let Some(cond) = prim.null_cond(schema.type_null_value(ty), true) else {
                    continue;
                };
                let method_name = name.snake_ident().suffix_snake("opt");
                let host_type = prim.host();
                code.extend(quote! {
                    #[inline]
                    pub fn #method_name(&self) -> Option<#host_type> {
                        let raw = #val_expr;
                        if #cond { None } else { Some(raw) }
                    }
                });
            }
        }
    }
    code
}

fn composite_null_constants(fields: &[CompositeField], schema: &ValidatedSchema) -> TokenStream {
    let mut code = TokenStream::new();
    for field in fields {
        match field {
            CompositeField {
                name,
                kind:
                    CompositeKind::Type {
                        primitive,
                        length,
                        presence,
                        null_value,
                        ..
                    },
                ..
            } => {
                if field.is_constant() {
                    continue;
                }
                let null_raw = null_value.as_deref().or_else(|| {
                    if *presence == Presence::Optional && length.is_none() {
                        Some(primitive.null_name())
                    } else {
                        None
                    }
                });
                let null_raw = match null_raw {
                    Some(v) => v,
                    None => continue,
                };
                let Some(rust_type) = Some(Resolved::Scalar(*primitive)) else {
                    continue;
                };
                let cname = format_ident!("{}_NULL", name.const_ident());
                if let Some(len) = length {
                    if null_value.is_none() {
                        continue;
                    }
                    if let Some((ty, expr)) = rust_type.const_array(null_raw, *len) {
                        code.extend(quote! { pub const #cname: #ty = #expr; });
                    }
                } else if let Some(expr) = rust_type.const_expr(null_raw) {
                    let ty = &rust_type;
                    code.extend(quote! { pub const #cname: #ty = #expr; });
                }
            }
            CompositeField {
                name,
                kind: CompositeKind::Ref { ty, .. },
                ..
            } => {
                let (prim, length, presence, null_value) =
                    if let Some(TypeDef::Primitive(PrimitiveDef {
                        primitive,
                        length,
                        presence,
                        null_value,
                        ..
                    })) = schema.types.get(&**ty)
                    {
                        (*primitive, *length, *presence, null_value.as_deref())
                    } else {
                        continue;
                    };
                let is_optional = presence == Presence::Optional || null_value.is_some();
                if !is_optional {
                    continue;
                }
                let Some(null_raw) = null_value.or_else(|| {
                    (presence == Presence::Optional && length.is_none()).then(|| prim.null_name())
                }) else {
                    continue;
                };
                let rust_type = Resolved::Scalar(prim);
                let cname = format_ident!("{}_NULL", name.const_ident());
                let alias = ty.type_ident();
                if let Some(len) = length {
                    if null_value.is_none() {
                        continue;
                    }
                    if let Some((_, expr)) = rust_type.const_array(null_raw, len) {
                        code.extend(quote! { pub const #cname: #alias = #expr; });
                    }
                } else if let Some(expr) = rust_type.const_expr(null_raw) {
                    code.extend(quote! { pub const #cname: #alias = #expr; });
                }
            }
        }
    }
    code
}

use super::*;

/// A schema with every type reference resolved to the Rust type that represents it.
///
/// This pass answers *what type is this*. It says nothing about where anything sits — that is
/// the next pass, and keeping them apart is why neither can quietly depend on the other.
#[derive(derive_more::Deref)]
pub struct LoweredSchema {
    #[deref]
    pub schema: ValidatedSchema,
    pub messages: Vec<LoweredMessage>,
}

pub struct LoweredMessage {
    pub msg: Message,
    /// every declared field in order, constants included
    pub fields: Vec<LoweredField>,
    pub data: Vec<DataLayout>,
    pub groups: Vec<LoweredGroup>,
}

pub struct LoweredGroup {
    pub group: Group,
    /// the composite carrying this group's count and block length, resolved here so nothing
    /// downstream has to go back to the schema to spell it
    pub dimension: Path,
    pub fields: Vec<LoweredField>,
    pub data: Vec<DataLayout>,
    pub groups: Vec<LoweredGroup>,
}

#[derive(Drive)]
pub struct LoweredField {
    pub field: Field,
    #[drive(skip)]
    pub name: Ident,
    #[drive(skip)]
    pub resolved: Resolved,
    #[drive(skip)]
    pub ty: Type,
    #[drive(skip)]
    pub view: FieldView,
    /// the primitive the field encodes as, through an enum, set or alias where it has one. Not
    /// read by the emitter, which was given the decisions this fed; it is here for a derivation
    /// that wants to swap the field's type for one of its own.
    #[allow(dead_code)]
    #[drive(skip)]
    pub primitive: Option<Primitive>,
    /// the prologue a setter runs before writing: binds `raw` and asserts the declared bounds
    #[drive(skip)]
    pub setter_checks: TokenStream,
}

/// What the view exposes for a field: how its value reads, and which of the optional / enum /
/// composite / fixed-string helpers it qualifies for.
///
/// Every one of these is decided here, so emission is a template with nothing left to work
/// out. The method names are derived from the field name at render time — they are a spelling,
/// not a decision.
pub struct FieldView {
    pub value_ty: TokenStream,
    pub value_expr: TokenStream,
    /// the inner type when the value is itself an `Option`, which `_required` unwraps twice
    pub nullable_inner: Option<TokenStream>,
    /// how `#[sbe_gen]` reaches the value, when the macro has a spelling for this shape
    pub value_read: Option<Read>,
    /// the enum a `_enum` helper returns, when the field is an enum with values
    pub as_enum: Option<Path>,
    /// the field's type is a newtype over its encoding rather than an alias to it
    pub newtype: bool,
    /// `_opt` helpers reached through a composite member, and what each hands back
    pub composite_opts: Vec<(Ident, Type)>,
    /// length of the byte array, when the field is one
    pub fixed_string: Option<usize>,
    pub required: bool,
    /// the type and value of the constant this field carries, when it carries one
    pub constant: Option<(TokenStream, TokenStream)>,
    /// the name the constant is emitted under
    pub const_name: Ident,
    /// the `_opt` accessor's ingredients, when the field qualifies for one
    pub opt_accessor: Option<OptAccessor>,
}

/// How a value is reached through whatever the schema's type wraps it in.
#[derive(Clone, Copy, PartialEq)]
pub enum Read {
    Plain,
    Get,
    Inner,
    InnerGet,
}

impl Read {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Get => "get",
            Self::Inner => "inner",
            Self::InnerGet => "inner_get",
        }
    }
}

/// An `_opt` accessor reads a field, compares it against its null and hands back either the
/// value or the wrapper it came in. Only the read itself depends on where the field is being
/// accessed from, so everything else is settled here.
pub struct OptAccessor {
    pub ret_ty: TokenStream,
    /// an enum or set hands back the wrapper it read through, a primitive the raw value; either
    /// way the raw integer is reached through `.0` first
    pub wrapper: bool,
    /// the raw integer is behind a byteorder wrapper and needs `.get()`
    pub get: bool,
    /// the sentinel the raw integer holds when the field is absent, or `None` for a float,
    /// where absence is NaN
    pub null: Option<TokenStream>,
}

impl LoweredSchema {
    pub(crate) fn new(schema: ValidatedSchema) -> Result<Self, CodegenError> {
        Lower { schema }.visit_schema()
    }
}

/// Resolves every type reference in the schema.
struct Lower {
    schema: ValidatedSchema,
}

impl Lower {
    fn visit_schema(self) -> Result<LoweredSchema, CodegenError> {
        let messages = self
            .schema
            .messages
            .iter()
            .map(|msg| self.visit_message(msg))
            .collect::<Result<_, _>>()?;
        Ok(LoweredSchema {
            schema: self.schema,
            messages,
        })
    }

    fn visit_message(&self, msg: &Message) -> Result<LoweredMessage, CodegenError> {
        let scope = format!("message '{}'", msg.name);
        Ok(LoweredMessage {
            msg: msg.clone(),
            fields: self.visit_fields(&msg.fields(), &scope)?,
            data: self.visit_data(&msg.data(), &scope)?,
            groups: self.visit_groups(&msg.groups(), &scope)?,
        })
    }

    fn visit_groups(
        &self,
        groups: &[&Group],
        outer: &str,
    ) -> Result<Vec<LoweredGroup>, CodegenError> {
        groups
            .iter()
            .map(|group| self.visit_group(group, outer))
            .collect()
    }

    fn visit_group(&self, group: &Group, outer: &str) -> Result<LoweredGroup, CodegenError> {
        let scope = format!("{outer} group '{}'", group.name);
        // for the error, not the answer: the dimension's shape is read again where the
        // `Dimension` impl is written, and that call cannot report because it is inside a
        // render. A group naming a type that is not a usable dimension has to fail here.
        dimension_fields(&self.schema, &group.dimension_type, &scope)?;
        Ok(LoweredGroup {
            group: group.clone(),
            dimension: schema_path(&group.dimension_type),
            fields: self.visit_fields(&group.fields(), &scope)?,
            data: self.visit_data(&group.data(), &scope)?,
            groups: self.visit_groups(&group.groups(), &scope)?,
        })
    }

    fn visit_fields(
        &self,
        fields: &[&Field],
        scope: &str,
    ) -> Result<Vec<LoweredField>, CodegenError> {
        fields
            .iter()
            .map(|field| self.visit_field(field, scope))
            .collect()
    }

    fn visit_field(&self, field: &Field, scope: &str) -> Result<LoweredField, CodegenError> {
        let where_ = format!("{scope} field '{}'", field.name);
        let mapped = match self.schema.opts.type_map.get(&*field.ty) {
            // the replacement is laid out the same, so it stands in wherever the schema's own
            // type would have: the struct member, the setter, the value the view hands back
            Some(path) => Some(Resolved::Named(syn::parse_str(path)?)),
            None => None,
        };
        let Some(resolved) = mapped.or_else(|| resolve_message_type(&field.ty, &self.schema))
        else {
            return Err(CodegenError::UnsupportedFieldType {
                ty: field.ty.clone(),
                scope: where_,
            });
        };
        TypeRefs::new(&self.schema).check(&field.ty, &where_)?;
        let primitive = self.schema.field_primitive(field);
        Ok(LoweredField {
            name: field.name.snake_ident(),
            view: field_view(field, &resolved, primitive, &self.schema),
            setter_checks: setter_checks(field, primitive, &self.schema),
            primitive,
            field: field.clone(),
            ty: parse_quote!(#resolved),
            resolved,
        })
    }

    fn visit_data(
        &self,
        data: &[&VarDataField],
        scope: &str,
    ) -> Result<Vec<DataLayout>, CodegenError> {
        data.iter()
            .map(|d| {
                let Some(length_ty) = try_length_kind_for_data(&d.ty, &self.schema) else {
                    return Err(CodegenError::UnsupportedVarDataLength {
                        ty: d.ty.clone(),
                        scope: scope.to_string(),
                        field: d.name.clone(),
                    });
                };
                let name = d.name.snake_ident();
                Ok(DataLayout {
                    parse_fn: format_ident!("parse_{}", name),
                    length_ty: parse_quote!(#length_ty),
                    name,
                })
            })
            .collect()
    }
}

/// Decide the view surface for a field. An array reads as itself; an enum or set reads through
/// its wrapper; a plain primitive reads as its host type. Nullability is folded in here so the
/// emitter never re-asks.
fn field_view(
    field: &Field,
    resolved: &Resolved,
    primitive: Option<Primitive>,
    schema: &ValidatedSchema,
) -> FieldView {
    let explicit_null = schema.field_null_value(field);
    let is_optional = schema.field_is_optional(field);
    let nullable = is_optional || explicit_null.is_some();
    let use_default = explicit_null.is_none() && is_optional;

    // the fourth element is how the macro spells the read, when it has a spelling for it. It
    // is decided here rather than recovered from `value_expr` later: the expression is tokens,
    // and matching on how they happen to print is a silent break waiting for a reformat.
    let (value_ty, value_expr, nullable_inner, value_read) = 'value: {
        let plain = |ty: TokenStream| (ty, quote!(*value), None, Some(Read::Plain));
        if matches!(resolved, Resolved::Array(..)) {
            break 'value plain(resolved.to_token_stream());
        }
        if let Some(
            TypeDef::Enum(EnumDef { encoding, .. }) | TypeDef::Set(SetDef { encoding, .. }),
        ) = schema.types.get(&field.ty)
        {
            let field_ty = schema_path(&field.ty).to_token_stream();
            let Some(prim) = encoding_primitive(encoding, &schema.types) else {
                break 'value plain(field_ty);
            };
            if nullable
                && let Some(raw) = Resolved::Scalar(prim).read(quote!(value.0))
                && let Some(cond) = prim.null_cond(explicit_null, use_default)
            {
                break 'value (
                    quote!(Option<#field_ty>),
                    quote!({ let raw = #raw; if #cond { None } else { Some(*value) } }),
                    Some(field_ty),
                    // the null test makes this the `_opt` accessor's shape, not a plain read
                    None,
                );
            }
            break 'value plain(field_ty);
        }
        if let Some(prim) = primitive
            && let Some(raw) = Resolved::Scalar(prim).read(quote!((*value)))
        {
            let host = prim.host().to_token_stream();
            if nullable && let Some(cond) = prim.null_cond(explicit_null, use_default) {
                break 'value (
                    quote!(Option<#host>),
                    quote!({ let raw = #raw; if #cond { None } else { Some(raw) } }),
                    Some(host),
                    None,
                );
            }
            let read = match Resolved::Scalar(prim).is_wrapped() {
                true => Read::Get,
                false => Read::Plain,
            };
            break 'value (host, raw, None, Some(read));
        }
        plain(resolved.to_token_stream())
    };

    FieldView {
        value_ty,
        value_expr,
        nullable_inner,
        value_read,
        as_enum: match schema.types.get(&field.ty) {
            Some(TypeDef::Enum(EnumDef { values, .. })) if !values.is_empty() => {
                let ident = format_ident!("{}Enum", field.ty.type_ident());
                Some(parse_quote!(super::types::#ident))
            }
            _ => None,
        },
        // an enum or set is a newtype over its encoding, so the raw value is one `.0` deeper
        newtype: matches!(
            schema.types.get(&field.ty),
            Some(TypeDef::Enum(_) | TypeDef::Set(_))
        ),
        composite_opts: composite_optional_view_helpers(&field.ty, schema),
        fixed_string: resolved.u8_array_len(),
        required: schema.field_is_required(field),
        constant: constant_field_value_expr(field, schema),
        const_name: field.name.const_ident(),
        opt_accessor: (is_optional && schema.type_length(&field.ty).unwrap_or(1) <= 1)
            .then(|| opt_accessor(field, explicit_null, primitive, schema))
            .flatten(),
    }
}

/// An enum or set reads its null through the wrapper and hands the wrapper back; a plain
/// primitive reads and hands back the host value. Normalising that difference here leaves one
/// template at every site that renders the accessor.
fn opt_accessor(
    field: &Field,
    explicit_null: Option<&str>,
    primitive: Option<Primitive>,
    schema: &ValidatedSchema,
) -> Option<OptAccessor> {
    let (prim, ret_ty, wrapper) = match schema.types.get(&field.ty) {
        Some(TypeDef::Enum(EnumDef { encoding, .. }) | TypeDef::Set(SetDef { encoding, .. })) => {
            let ty = schema_path(&field.ty);
            (
                encoding_primitive(encoding, &schema.types)?,
                quote!(#ty),
                true,
            )
        }
        _ => {
            let prim = primitive?;
            let host = prim.host();
            (prim, quote!(#host), false)
        }
    };
    Some(OptAccessor {
        ret_ty,
        wrapper,
        get: Resolved::Scalar(prim).is_wrapped(),
        null: prim.null_sentinel(explicit_null, true)?,
    })
}

/// What a setter checks before it writes. Enums and sets validate against their encoding, so
/// the raw value reads through the wrapper; everything else validates the value as given.
fn setter_checks(
    field: &Field,
    primitive: Option<Primitive>,
    schema: &ValidatedSchema,
) -> TokenStream {
    let nothing_to_check = field.min_value.is_none()
        && field.max_value.is_none()
        && (field.presence == Presence::Optional || field.null_value.is_none());
    let raw = match schema.types.get(&field.ty) {
        Some(TypeDef::Enum(EnumDef { encoding, .. }) | TypeDef::Set(SetDef { encoding, .. })) => {
            encoding_primitive(encoding, &schema.types)
                .and_then(|prim| Resolved::Scalar(prim).read(quote!(value.0)))
        }
        _ => Some(quote!(value)),
    };
    let (Some(prim), Some(raw), false) = (primitive, raw, nothing_to_check) else {
        return TokenStream::new();
    };
    let host = prim.host();

    let min = field.min_value.as_deref().map(|v| {
        let (msg, bound) = (format!("{} below minValue", field.name), prim.literal(v));
        quote! { let min: #host = #bound; assert!(raw >= min, #msg); }
    });
    let max = field.max_value.as_deref().map(|v| {
        let (msg, bound) = (format!("{} above maxValue", field.name), prim.literal(v));
        quote! { let max: #host = #bound; assert!(raw <= max, #msg); }
    });
    let null = field
        .null_value
        .as_deref()
        .filter(|_| field.presence != Presence::Optional)
        .and_then(|nullv| {
            let msg = format!("{} uses nullValue but field is required", field.name);
            let cond = prim.null_cond(Some(nullv), false)?;
            Some(quote! { assert!(!(#cond), #msg); })
        });

    quote! { let raw: #host = #raw; #min #max #null }
}

fn constant_field_value_expr(
    field: &Field,
    schema: &ValidatedSchema,
) -> Option<(TokenStream, TokenStream)> {
    if let Some(value_ref) = &field.value_ref
        && let Some((type_name, variant)) = value_ref.split_once('.')
    {
        let ty = schema_path(type_name);
        let variant = variant.const_ident();
        return Some((quote!(#ty), quote!(#ty::#variant)));
    }

    if let Some(val) = field.constant.as_deref() {
        let const_ty = resolve_message_type(&field.ty, schema)
            .map(|r| r.to_token_stream())
            .unwrap_or_else(|| field.ty.type_ident().to_token_stream());
        if let Some(td) = schema.types.get(&field.ty) {
            match td {
                TypeDef::Primitive(PrimitiveDef {
                    primitive, length, ..
                }) => {
                    let rust_type = Resolved::Scalar(*primitive);
                    if let Some(len) = length {
                        let (_, expr) = rust_type.const_array(val, *len)?;
                        return Some((const_ty, expr));
                    }
                    return Some((const_ty, rust_type.const_expr(val)?));
                }
                TypeDef::Enum(EnumDef { encoding, .. }) | TypeDef::Set(SetDef { encoding, .. }) => {
                    let rust_type = Primitive::parse(encoding).map(Resolved::Scalar)?;
                    let raw_expr = rust_type.const_expr(val)?;
                    let field_ty = schema_path(&field.ty);
                    return Some((quote!(#field_ty), quote!(#field_ty(#raw_expr))));
                }
                TypeDef::Composite(CompositeDef { .. }) => {}
            }
        } else if let Some(rust_type) = Primitive::parse(&field.ty).map(Resolved::Scalar) {
            let raw_expr = rust_type.const_expr(val)?;
            return Some((const_ty, raw_expr));
        }
    }

    if let Some(TypeDef::Primitive(PrimitiveDef {
        constant: Some(val),
        primitive,
        length,
        ..
    })) = schema.types.get(&field.ty)
    {
        let rust_type = Resolved::Scalar(*primitive);
        let field_ty = schema_path(&field.ty).to_token_stream();
        if let Some(len) = length {
            let (_, expr) = rust_type.const_array(val, *len)?;
            return Some((field_ty, expr));
        }

        let raw_expr = rust_type.const_expr(val)?;
        let alias_target = schema
            .opts
            .constant_type_aliases
            .get(&*field.ty)
            .map(Name::from)
            .or_else(|| schema.constant_type_alias_from_schema(&field.ty))
            .unwrap_or_else(|| Name::from(rust_type.to_token_stream().to_string()));
        let expr = if matches!(
            schema.types.get(&alias_target),
            Some(TypeDef::Enum(EnumDef { .. }) | TypeDef::Set(SetDef { .. }))
        ) {
            let alias = schema_path(&alias_target);
            quote!(#alias(#raw_expr))
        } else {
            raw_expr
        };
        return Some((field_ty, expr));
    }

    None
}

/// The `_opt` helpers a composite's members qualify for. An inline member carries its own
/// presence and null value; a `ref` member takes them from the type it points at — normalising
/// that difference first lets one set of filters decide both.
fn composite_optional_view_helpers(ty: &str, schema: &ValidatedSchema) -> Vec<(Ident, Type)> {
    struct Member<'a> {
        name: &'a Name,
        prim: Primitive,
        length: Option<usize>,
        null_value: Option<&'a str>,
        optional: bool,
    }

    let Some(TypeDef::Composite(CompositeDef { fields, .. })) = schema.types.get(ty) else {
        return Vec::new();
    };
    fields
        .iter()
        .filter(|field| !field.is_constant())
        .filter_map(|field| match &field.kind {
            CompositeKind::Type {
                primitive,
                length,
                presence,
                null_value,
                ..
            } => Some(Member {
                name: &field.name,
                prim: Some(*primitive)?,
                length: *length,
                null_value: null_value.as_deref(),
                optional: match presence {
                    Presence::Optional => true,
                    Presence::Required | Presence::Constant => false,
                    Presence::Inherited => null_value.is_some(),
                },
            }),
            CompositeKind::Ref { ty } => Some(Member {
                name: &field.name,
                prim: schema.type_primitive(ty)?,
                length: schema.type_length(ty),
                null_value: schema.type_null_value(ty),
                optional: schema.type_is_optional(ty),
            }),
        })
        .filter(|m| m.optional)
        .filter(|m| m.length.unwrap_or(1) <= 1)
        .filter(|m| m.prim.null_cond(m.null_value, true).is_some())
        .map(|m| {
            let host = m.prim.host();
            (
                m.name.snake_ident().suffix_snake("opt"),
                parse_quote!(#host),
            )
        })
        .collect()
}

/// The view accessors for a field. Which of these exist was decided during lowering; this is
/// the spelling.
pub(crate) fn view_field_helpers(lowered: &PlacedField) -> TokenStream {
    let FieldView {
        value_ty,
        value_expr,
        nullable_inner,
        as_enum,
        composite_opts,
        required,
        ..
    } = &lowered.view;
    let accessor = &lowered.name;
    let value_name = accessor.suffix_snake("value");
    // the error carries the field name as data, not as an identifier
    let reported = accessor.to_string();

    // the value helpers are `#[sbe_gen]`'s wherever it has a spelling for the read
    let derived = lowered.value_is_derived();
    let mut code = match derived {
        true => TokenStream::new(),
        false => quote! {
            #[inline]
            pub fn #value_name(&self) -> Option<#value_ty> {
                let value = self.#accessor()?;
                Some(#value_expr)
            }
        },
    };

    if *required && !derived {
        let required_name = accessor.suffix_snake("required");
        code.extend(match nullable_inner {
            Some(inner) => quote! {
                #[inline]
                pub fn #required_name(&self) -> Result<#inner, DecodeFieldError> {
                    let value = self.#value_name().ok_or(DecodeFieldError::MissingField(#reported))?;
                    value.ok_or(DecodeFieldError::NullValue(#reported))
                }
            },
            None => quote! {
                #[inline]
                pub fn #required_name(&self) -> Result<#value_ty, DecodeFieldError> {
                    self.#value_name().ok_or(DecodeFieldError::MissingField(#reported))
                }
            },
        });
    }

    if let Some(enum_name) = as_enum {
        let enum_method = accessor.suffix_snake("enum");
        let body = if nullable_inner.is_some() {
            quote! {
                let value = self.#value_name()?;
                value.and_then(|raw| raw.as_enum())
            }
        } else {
            quote! { self.#value_name().and_then(|raw| raw.as_enum()) }
        };
        code.extend(quote! {
            #[inline]
            pub fn #enum_method(&self) -> Option<#enum_name> { #body }
        });
    }

    code.extend(composite_opts.iter().map(|(sub_method, ret_ty)| {
        let method_name = accessor.suffix_snake(sub_method);
        quote! {
            #[inline]
            pub fn #method_name(&self) -> Option<#ret_ty> {
                self.#accessor().and_then(|value| value.#sub_method())
            }
        }
    }));

    code
}

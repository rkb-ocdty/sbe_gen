use super::*;

pub(crate) fn composite_field_size(
    field: &CompositeField,
    schema: &ValidatedSchema,
    depth: usize,
    respect_constant: bool,
) -> Option<usize> {
    match field {
        CompositeField {
            kind:
                CompositeKind::Type {
                    primitive,
                    length,
                    presence,
                    ..
                },
            ..
        } => {
            if respect_constant && *presence == Presence::Constant {
                return Some(0);
            }
            let base = Some(primitive.size())?;
            Some(base * length.unwrap_or(1))
        }
        CompositeField {
            kind: CompositeKind::Ref { ty, .. },
            ..
        } => type_size_bytes(ty, schema, depth + 1, respect_constant),
    }
}

pub(crate) fn type_size_bytes(
    ty: &str,
    schema: &ValidatedSchema,
    depth: usize,
    respect_constant: bool,
) -> Option<usize> {
    if depth > 8 {
        return None;
    }
    if let Some(prim) = Primitive::parse(ty) {
        return Some(prim.size());
    }
    if let Some(td) = schema.types.get(ty) {
        match td {
            TypeDef::Primitive(PrimitiveDef {
                primitive,
                length,
                presence,
                ..
            }) => {
                if respect_constant && *presence == Presence::Constant {
                    return Some(0);
                }
                let base = Some(primitive.size())?;
                Some(base * length.unwrap_or(1))
            }
            TypeDef::Enum(EnumDef { encoding, .. }) | TypeDef::Set(SetDef { encoding, .. }) => {
                encoding_primitive(encoding, &schema.types).map(Primitive::size)
            }
            TypeDef::Composite(CompositeDef { fields, .. }) => {
                let mut total = 0usize;
                for f in fields {
                    let start = f.offset.map_or(total, |o| o as usize);
                    let size = composite_field_size(f, schema, depth, respect_constant)?;
                    total = start.checked_add(size)?;
                }
                Some(total)
            }
        }
    } else {
        resolve_type(ty, schema).and_then(|resolved| resolved.size())
    }
}

pub(crate) fn field_size_bytes(field: &Field, schema: &ValidatedSchema) -> Option<usize> {
    // constant fields are not encoded
    if schema.field_is_constant(field) {
        return Some(0);
    }
    if field.presence != Presence::Inherited
        && let Some(TypeDef::Primitive(PrimitiveDef {
            primitive,
            length,
            presence: Presence::Constant,
            ..
        })) = schema.types.get(&field.ty)
    {
        return Some(primitive.size() * length.unwrap_or(1));
    }
    type_size_bytes(&field.ty, schema, 0, true)
}

/// The `#[repr(C)]` fields of the block, with the padding lowering already worked out. This is
/// a map, not a walk: the cursor lives in `lower_fields` and nothing recomputes it here.
pub struct DimensionFields {
    pub block_field: Ident,
    pub block_field_ty: Resolved,
    pub count_field: Ident,
    pub count_field_ty: Resolved,
}

pub(crate) fn dimension_fields(
    schema: &ValidatedSchema,
    dim_type: &str,
    scope: &str,
) -> Result<DimensionFields, CodegenError> {
    let problem = |problem| CodegenError::Dimension {
        ty: Name::from(dim_type),
        scope: scope.to_string(),
        problem,
    };
    let Some(def) = schema.types.get(dim_type) else {
        return Err(problem(DimensionProblem::Unknown));
    };
    let TypeDef::Composite(CompositeDef { fields, .. }) = def else {
        return Err(problem(DimensionProblem::NotComposite));
    };

    TypeRefs::new(schema).check(&Name::from(dim_type), scope)?;

    // both member shapes answer the same question — what integer type does this resolve to —
    // so they normalise to a `Resolved` first and share one check
    let dim_fields = fields
        .iter()
        .filter(|f| !f.is_constant())
        .map(|f| {
            let unusable = || problem(DimensionProblem::NotAnInteger(f.name.clone()));
            let resolved = match &f.kind {
                CompositeKind::Type { primitive, .. } => Resolved::Scalar(*primitive),
                CompositeKind::Ref { ty } => match schema.types.get(&**ty) {
                    Some(TypeDef::Primitive(PrimitiveDef {
                        primitive,
                        presence,
                        ..
                    })) if *presence != Presence::Constant => Resolved::Scalar(*primitive),
                    Some(
                        TypeDef::Enum(EnumDef { encoding, .. })
                        | TypeDef::Set(SetDef { encoding, .. }),
                    ) => Primitive::parse(encoding)
                        .map(Resolved::Scalar)
                        .ok_or_else(unusable)?,
                    Some(_) => return Err(unusable()),
                    None => Primitive::parse(ty)
                        .map(Resolved::Scalar)
                        .ok_or_else(unusable)?,
                },
            };
            resolved
                .is_int()
                .then(|| (f.name.snake_ident(), resolved))
                .ok_or_else(unusable)
        })
        .collect::<Result<Vec<_>, CodegenError>>()?;

    if dim_fields.len() < 2 {
        return Err(problem(DimensionProblem::TooFewFields));
    }

    let norm = |name: &dyn Display| name.to_string().replace('_', "").to_lowercase();
    let block = dim_fields
        .iter()
        .find(|(n, _)| norm(n) == "blocklength")
        .cloned()
        .or_else(|| {
            dim_fields
                .iter()
                .find(|(n, _)| norm(n).contains("blocklength"))
                .cloned()
        })
        .or_else(|| dim_fields.first().cloned())
        .ok_or_else(|| problem(DimensionProblem::NoBlockLength))?;
    let count = dim_fields
        .iter()
        .find(|(n, _)| {
            *n != block.0 && {
                let nn = norm(n);
                nn == "numingroup" || nn == "count"
            }
        })
        .cloned()
        .or_else(|| {
            dim_fields
                .iter()
                .find(|(n, _)| {
                    *n != block.0 && {
                        let nn = norm(n);
                        nn.contains("numingroup") || nn.contains("count")
                    }
                })
                .cloned()
        })
        .or_else(|| dim_fields.iter().find(|(n, _)| *n != block.0).cloned())
        .ok_or_else(|| problem(DimensionProblem::NoCount))?;
    Ok(DimensionFields {
        block_field: block.0,
        block_field_ty: block.1,
        count_field: count.0,
        count_field_ty: count.1,
    })
}

/// A variable-length data member after lowering.
#[derive(Drive)]
#[drive(skip)]
pub struct DataLayout {
    pub name: Ident,
    pub parse_fn: Ident,
    pub length_ty: Type,
}

/// A lowered schema with every field given its place in the fixed block.
///
/// This pass answers *where does it sit*, and it is the only pass that talks about bytes. It
/// consumes the lowered tree rather than sitting beside it, so there is no way to read a
/// placement that was never computed.
#[derive(derive_more::Deref, Drive)]
pub struct LaidOutSchema {
    #[deref]
    #[drive(skip)]
    pub schema: ValidatedSchema,
    pub messages: Vec<PlacedMessage>,
}

#[derive(Drive)]
pub struct PlacedMessage {
    pub msg: Message,
    #[drive(skip)]
    pub names: MessageNames,
    #[drive(skip)]
    pub block_length: usize,
    /// bytes the packed struct occupies, which a derivation swapping a field type must preserve
    #[drive(skip)]
    pub struct_size: usize,
    pub fields: Vec<PlacedField>,
    pub data: Vec<DataLayout>,
    pub groups: Vec<PlacedGroup>,
}

/// Every Rust name spelled out of the message name, worked out once and carried along.
pub struct MessageNames {
    pub msg: Ident,
    pub builder: Ident,
    pub encoder: Ident,
    pub view: Ident,
    pub msg_ref: Ident,
}

#[derive(Drive)]
pub struct PlacedGroup {
    pub group: Group,
    /// declared by more than one message, so it is written into `groups.rs` once
    #[drive(skip)]
    pub shared: bool,
    #[drive(skip)]
    pub dimension: Path,
    #[drive(skip)]
    pub block_length: usize,
    /// bytes the packed entry struct occupies
    #[drive(skip)]
    pub struct_size: usize,
    pub fields: Vec<PlacedField>,
    pub data: Vec<DataLayout>,
    pub groups: Vec<PlacedGroup>,
}

/// Derefs to what it was lowered from, so the type and name are read straight off it.
#[derive(derive_more::Deref, Drive)]
pub struct PlacedField {
    #[deref]
    pub lowered: LoweredField,
    /// bytes of padding the struct needs before this field to honour a declared offset, and
    /// the name of the member that holds them
    #[drive(skip)]
    pub padding: usize,
    #[drive(skip)]
    pub pad_name: Option<Ident>,
    #[drive(skip)]
    pub placement: Placement,
}

impl PlacedField {
    /// Whether `#[sbe_gen]` writes this field's setter.
    ///
    /// The macro can only do the plain case: take the field's own type and store it where
    /// `offset_of!` says. A field that checks its value first, or whose setter takes the host
    /// integer behind an alias, is the generator's to write.
    pub(crate) fn setter_is_derivable(&self) -> bool {
        self.at().is_some()
            && self.setter_checks.is_empty()
            // `encode` hands the value straight through only where there is no primitive to
            // wrap or cast it into
            && self.resolved.primitive().is_none()
            && self.resolved.param_ty() == self.ty
    }

    /// whether `#[sbe_gen]` writes the value helpers for this field
    pub(crate) fn value_is_derived(&self) -> bool {
        self.view.opt_accessor.is_some() || self.view.value_read.is_some()
    }
}

/// Where a field sits in the fixed block.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    At {
        offset: usize,
        size: usize,
    },
    /// carries its value in the schema, so it occupies no bytes and does not move the cursor
    Constant,
}

impl PlacedField {
    /// carries its value in the schema, so it occupies no wire bytes
    pub(crate) fn is_constant(&self) -> bool {
        self.placement == Placement::Constant
    }

    /// offset and size, for the fields that actually occupy bytes
    pub(crate) fn at(&self) -> Option<(usize, usize)> {
        match self.placement {
            Placement::At { offset, size } => Some((offset, size)),
            Placement::Constant => None,
        }
    }
}

impl LaidOutSchema {
    pub(crate) fn new(lowered: LoweredSchema) -> Result<Self, CodegenError> {
        let LoweredSchema { schema, messages } = lowered;
        Place { schema }.visit_schema(messages)
    }
}

/// Gives every field its place in the fixed block.
struct Place {
    schema: ValidatedSchema,
}

/// A declared `blockLength` wins; otherwise the block is as long as its fields reach.
fn declared_or_placed(declared: Option<u32>, fields_end: usize) -> usize {
    declared.map_or(fields_end, |b| b as usize)
}

/// How long the packed struct is: as far as the last field reaches. A declared `blockLength`
/// can be longer than this, but the struct itself stops where the fields do.
fn fields_end(fields: &[PlacedField]) -> usize {
    fields
        .iter()
        .filter_map(PlacedField::at)
        .map(|(offset, size)| offset + size)
        .max()
        .unwrap_or(0)
}

/// Composite members are laid out in declaration order. An explicit offset may leave a gap
/// but must not point back into a member already placed, because the generated struct lays
/// them out sequentially with padding and cannot express an overlap.
fn composite_layout(
    composite: &Name,
    fields: &[CompositeField],
    schema: &ValidatedSchema,
) -> Result<(), CodegenError> {
    let mut end = 0usize;
    for field in fields {
        if field.is_constant() {
            continue;
        }
        let start = field.offset.map_or(end, |o| o as usize);
        if start < end {
            return Err(CodegenError::OverlappingComposite {
                composite: composite.clone(),
                field: field.name.clone(),
                start,
                end,
            });
        }
        let size = composite_field_size(field, schema, 0, true).ok_or_else(|| {
            CodegenError::UnsupportedCompositeLayout {
                composite: composite.clone(),
                field: field.name.clone(),
            }
        })?;
        end = start + size;
    }
    Ok(())
}

impl Place {
    /// Composite layout is checked here rather than during validation because it is a
    /// statement about bytes, which is what this pass is about.
    fn visit_schema(self, messages: Vec<LoweredMessage>) -> Result<LaidOutSchema, CodegenError> {
        CodegenError::all(self.schema.types.values().filter_map(|def| match def {
            TypeDef::Composite(CompositeDef { name, fields, .. }) => {
                composite_layout(name, fields, &self.schema).err()
            }
            _ => None,
        }))?;
        let messages = messages
            .into_iter()
            .map(|msg| self.visit_message(msg))
            .collect::<Result<_, _>>()?;
        Ok(LaidOutSchema {
            schema: self.schema,
            messages,
        })
    }

    fn visit_message(&self, msg: LoweredMessage) -> Result<PlacedMessage, CodegenError> {
        let scope = format!("message '{}'", msg.msg.name);
        let fields = self.visit_fields(msg.fields, &scope)?;
        let name = msg.msg.name.type_ident();
        Ok(PlacedMessage {
            names: MessageNames {
                builder: format_ident!("{}Builder", name),
                encoder: format_ident!("{}Encoder", name),
                view: format_ident!("{}View", name),
                msg_ref: format_ident!("{}Ref", name),
                msg: name,
            },
            block_length: declared_or_placed(msg.msg.block_length, fields_end(&fields)),
            struct_size: fields_end(&fields),
            fields,
            groups: self.visit_groups(msg.groups, &scope)?,
            data: msg.data,
            msg: msg.msg,
        })
    }

    fn visit_groups(
        &self,
        groups: Vec<LoweredGroup>,
        outer: &str,
    ) -> Result<Vec<PlacedGroup>, CodegenError> {
        groups
            .into_iter()
            .map(|group| self.visit_group(group, outer))
            .collect()
    }

    fn visit_group(&self, g: LoweredGroup, outer: &str) -> Result<PlacedGroup, CodegenError> {
        let scope = format!("{outer} group '{}'", g.group.name);
        let fields = self.visit_fields(g.fields, &scope)?;
        Ok(PlacedGroup {
            shared: self.schema.is_shared(&g.group),
            dimension: g.dimension,
            block_length: declared_or_placed(g.group.block_length, fields_end(&fields)),
            struct_size: fields_end(&fields),
            fields,
            groups: self.visit_groups(g.groups, &scope)?,
            data: g.data,
            group: g.group,
        })
    }

    /// The cursor lives here rather than on the visitor because each block starts its own.
    fn visit_fields(
        &self,
        fields: Vec<LoweredField>,
        scope: &str,
    ) -> Result<Vec<PlacedField>, CodegenError> {
        let (mut end, mut pads) = (0usize, 0usize);
        fields
            .into_iter()
            .map(|lowered| self.visit_field(lowered, scope, &mut end, &mut pads))
            .collect()
    }

    fn visit_field(
        &self,
        lowered: LoweredField,
        scope: &str,
        end: &mut usize,
        pads: &mut usize,
    ) -> Result<PlacedField, CodegenError> {
        if self.schema.field_is_constant(&lowered.field) {
            return Ok(PlacedField {
                lowered,
                padding: 0,
                pad_name: None,
                placement: Placement::Constant,
            });
        }
        // a field whose size cannot be computed cannot be given an offset, and everything
        // after it would be placed wrongly, so the schema is rejected rather than mislaid
        let size = field_size_bytes(&lowered.field, &self.schema).ok_or_else(|| {
            CodegenError::UncomputableSize {
                ty: lowered.field.ty.clone(),
                scope: format!("{scope} field '{}'", lowered.field.name),
            }
        })?;
        let offset = lowered.field.offset.map_or(*end, |o| o as usize);
        let padding = offset.saturating_sub(*end);
        *end = offset + size;
        let pad_name = (padding > 0).then(|| {
            *pads += 1;
            format_ident!("__padding{}", *pads - 1)
        });
        Ok(PlacedField {
            lowered,
            padding,
            pad_name,
            placement: Placement::At { offset, size },
        })
    }
}

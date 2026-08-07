use super::*;

pub(crate) fn parse_prefix_method() -> TokenStream {
    quote! {
        #[inline]
        pub fn parse_prefix(body: &[u8]) -> Option<(&Self, &[u8])> {
            Ref::<_, Self>::from_prefix(body).ok().map(|(r, b)| (Ref::into_ref(r), b))
        }
    }
}

/// Schema names are arbitrary XML strings; these turn them into the four Rust
/// casings the generator emits. Sanitization runs last so the case conversion
/// can't reintroduce a keyword or a leading digit.
pub(crate) trait SbeName {
    fn sanitized(&self) -> String;
    fn type_ident(&self) -> Ident;
    fn const_ident(&self) -> Ident;
    fn variant_ident(&self) -> Ident;
    fn suffix_snake(&self, suffix: impl Display) -> Ident;
}

// generator-built type and expression fragments; a malformed one fails at the
// syn::parse2 gate in render_file rather than silently emitting broken source
pub(crate) fn frag(src: &str) -> TokenStream {
    src.parse().unwrap_or_else(|e| {
        let msg = format!("sbe_gen produced an unparseable fragment {src:?}: {e}");
        quote! { compile_error!(#msg) }
    })
}

// every name a group generates, stated once
macro_rules! group_names {
    ($($method:ident = $fmt:literal from $case:ident;)+) => {
        impl Group {
            $(pub(crate) fn $method(&self) -> Ident { format_ident!($fmt, self.name.$case()) })+
        }
    };
}

group_names! {
    view_struct    = "{}Group"            from type_ident;
    iter_struct    = "{}Iter"             from type_ident;
    entry_struct   = "{}Entry"            from type_ident;
    entry_body     = "{}EntryBody"        from type_ident;
    entry_view     = "{}EntryView"        from type_ident;
    builder        = "{}GroupBuilder"     from type_ident;
    entry_builder  = "{}EntryBuilder"     from type_ident;
    encoder        = "{}GroupEncoder"     from type_ident;
    entry_encoder  = "{}EntryEncoder"     from type_ident;
    parse_fn       = "parse_{}"           from snake_ident;
}

impl Primitive {
    /// enum/set values declared under a char encoding read better as byte literals; a
    /// non-printable or multi-character value has no byte-literal spelling so it stays numeric
    pub(crate) fn enum_literal(self, val: &str) -> TokenStream {
        let single = || match *val.chars().collect::<Vec<_>>() {
            [c] => Some(c),
            _ => None,
        };
        match self {
            Primitive::Char => match single() {
                Some(c @ ('\\' | '\'')) => frag(&format!("b'\\{c}'")),
                Some(c) if (0x20..=0x7E).contains(&u32::from(c)) => frag(&format!("b'{c}'")),
                Some(c) => frag(&format!("{}u8", u32::from(c))),
                None => frag(val),
            },
            _ => frag(val),
        }
    }

    /// schema literals arrive as text; render the host-typed Rust literal for one
    pub(crate) fn literal(self, raw: &str) -> TokenStream {
        let t = raw.trim();
        match self {
            Primitive::Char => {
                Literal::u8_suffixed(t.as_bytes().first().copied().unwrap_or(0)).to_token_stream()
            }
            Primitive::Boolean if t == "true" => quote!(1u8),
            Primitive::Boolean if t == "false" => quote!(0u8),
            _ if self.is_float() && t.eq_ignore_ascii_case("nan") => {
                let host = self.host();
                quote!(#host::NAN)
            }
            _ => frag(t),
        }
    }

    pub(crate) fn default_literal(self) -> TokenStream {
        if self.is_float() {
            quote!(0.0)
        } else {
            quote!(0)
        }
    }

    pub(crate) fn null_cond(
        self,
        null_value: Option<&str>,
        use_default: bool,
    ) -> Option<TokenStream> {
        let defaulting_to_nan = null_value.is_none() && use_default;
        let explicit_nan = null_value.is_some_and(|v| v.trim().eq_ignore_ascii_case("nan"));
        if self.is_float() && (defaulting_to_nan || explicit_nan) {
            return Some(quote!(raw.is_nan()));
        }
        let raw = null_value.or_else(|| use_default.then(|| self.null_name()))?;
        let expr = self.literal(raw);
        Some(quote!(raw == #expr))
    }

    /// The same test, as the two things it is made of. `==` cannot go through an attribute:
    /// the printer splits the token in two and what comes out no longer parses.
    pub(crate) fn null_sentinel(
        self,
        null_value: Option<&str>,
        use_default: bool,
    ) -> Option<Option<TokenStream>> {
        let defaulting_to_nan = null_value.is_none() && use_default;
        let explicit_nan = null_value.is_some_and(|v| v.trim().eq_ignore_ascii_case("nan"));
        if self.is_float() && (defaulting_to_nan || explicit_nan) {
            return Some(None);
        }
        let raw = null_value.or_else(|| use_default.then(|| self.null_name()))?;
        Some(Some(self.literal(raw)))
    }
}

/// Resolve a field's type name to the appropriate Rust type.  This
/// consults the type map for user defined types or falls back to
/// primitive mapping.  Unsupported types return `None`, causing the
/// field to be skipped.
pub(crate) fn resolve_type(name: &str, schema: &Schema) -> Option<Resolved> {
    if let Some(prim) = Primitive::parse(name) {
        return Some(Resolved::Scalar(prim));
    }
    match schema.types.get(name)? {
        TypeDef::Primitive(PrimitiveDef {
            name: tname,
            primitive,
            length,
            ..
        }) => {
            let scalar = Resolved::Scalar(*primitive);
            Some(match length {
                Some(len) => Resolved::Array(Box::new(scalar), *len),
                None => Resolved::Named(local_path(tname)),
            })
        }
        TypeDef::Enum(EnumDef { name: tname, .. })
        | TypeDef::Set(SetDef { name: tname, .. })
        | TypeDef::Composite(CompositeDef { name: tname, .. }) => {
            Some(Resolved::Named(local_path(tname)))
        }
    }
}

pub(crate) fn local_path(name: &str) -> Path {
    let ident = name.type_ident();
    parse_quote!(#ident)
}



/// Inside a message module the schema types live one level up.
/// A schema type, from the crate root. Absolute, so it reads the same at any depth in the
/// generated tree.
pub(crate) fn schema_path(name: &str) -> Path {
    let ident = name.type_ident();
    parse_quote!(crate::types::#ident)
}

pub(crate) fn resolve_message_type(name: &str, schema: &Schema) -> Option<Resolved> {
    let resolved = resolve_type(name, schema)?;
    Some(match schema.types.get(name) {
        Some(TypeDef::Primitive(PrimitiveDef { name, length, .. })) if length.is_none() => {
            Resolved::Named(schema_path(name))
        }
        Some(TypeDef::Enum(EnumDef { name, .. }))
        | Some(TypeDef::Set(SetDef { name, .. }))
        | Some(TypeDef::Composite(CompositeDef { name, .. })) => Resolved::Named(schema_path(name)),
        _ => resolved,
    })
}

impl Primitive {
    /// The Rust type a var-data length prefix is encoded as. The generated helpers are
    /// generic over it, so the width is carried by the type rather than dispatched on.
    pub(crate) fn length_type(self) -> Option<Resolved> {
        (!self.is_float()).then(|| {
            Resolved::Scalar(match self.size() {
                1 => Primitive::Uint8,
                2 => Primitive::Uint16,
                4 => Primitive::Uint32,
                _ => Primitive::Uint64,
            })
        })
    }
}

pub(crate) fn primitive_length_kind(prim: &str) -> Option<Resolved> {
    Primitive::parse(prim).and_then(Primitive::length_type)
}

pub(crate) fn try_length_kind_for_data(ty: &str, schema: &Schema) -> Option<Resolved> {
    let mut visiting = HashSet::new();
    try_length_kind_for_data_inner(ty, schema, &mut visiting)
}

pub(crate) fn try_length_kind_for_data_inner(
    ty: &str,
    schema: &Schema,
    visiting: &mut HashSet<String>,
) -> Option<Resolved> {
    if let Some(kind) = primitive_length_kind(ty) {
        return Some(kind);
    }

    let td = schema.types.get(ty)?;
    if !visiting.insert(ty.to_string()) {
        return None;
    }

    let result = match td {
        TypeDef::Primitive(PrimitiveDef { primitive, .. }) => primitive.length_type(),
        TypeDef::Enum(EnumDef { encoding, .. }) | TypeDef::Set(SetDef { encoding, .. }) => {
            try_length_kind_for_data_inner(encoding, schema, visiting)
        }
        TypeDef::Composite(CompositeDef { fields, .. }) => {
            fields.iter().find_map(|f| match &f.kind {
                CompositeKind::Type {
                    primitive,
                    presence,
                    ..
                } if *presence != Presence::Constant => primitive.length_type(),
                CompositeKind::Ref { ty, .. } => {
                    try_length_kind_for_data_inner(ty, schema, visiting)
                }
                _ => None,
            })
        }
    };

    visiting.remove(ty);
    result
}

/// The primitive an enum or set is ultimately encoded as, following `encodingType` through
/// any chain of enums and sets.
///
/// The chain is a graph, not a tree — an encoding names a type — so it needs a visited set to
/// terminate. Without one a schema whose encodings form a cycle overflows the stack instead of
/// being rejected.
pub(crate) fn encoding_primitive(
    encoding: &str,
    types: &std::collections::BTreeMap<Name, TypeDef>,
) -> Option<Primitive> {
    let mut seen = HashSet::new();
    let mut at = encoding;
    loop {
        if !seen.insert(at) {
            return None;
        }
        match types.get(at) {
            Some(TypeDef::Primitive(PrimitiveDef { primitive, .. })) => return Some(*primitive),
            Some(
                TypeDef::Enum(EnumDef { encoding, .. }) | TypeDef::Set(SetDef { encoding, .. }),
            ) => at = encoding,
            Some(TypeDef::Composite(CompositeDef { .. })) => return None,
            None => return Primitive::parse(at),
        }
    }
}

/// A schema type lowered to the Rust type that represents it on the wire. Everything the
/// emitters need to know about a type — how to name it, read it, write it, size it — comes
/// off here, so nothing downstream inspects a type by its spelling.
#[derive(Clone)]
pub enum Resolved {
    /// `u8`/`i8` are plain; wider primitives are zerocopy byteorder wrappers
    Scalar(Primitive),
    Array(Box<Resolved>, usize),
    Named(Path),
}

impl ToTokens for Resolved {
    fn to_tokens(&self, out: &mut TokenStream) {
        match self {
            Self::Scalar(p) => p.rust().to_tokens(out),
            Self::Array(inner, len) => {
                let len = *len;
                quote!([#inner; #len]).to_tokens(out);
            }
            Self::Named(path) => path.to_tokens(out),
        }
    }
}

impl Resolved {
    pub(crate) fn primitive(&self) -> Option<Primitive> {
        match self {
            Self::Scalar(p) => Some(*p),
            _ => None,
        }
    }

    /// wide primitives arrive as zerocopy wrappers, which read through `.get()` and are
    /// built through `::new()`; `u8`/`i8` are the raw thing already
    pub(crate) fn is_wrapped(&self) -> bool {
        self.primitive().is_some_and(|p| p.size() > 1)
    }

    pub(crate) fn size(&self) -> Option<usize> {
        match self {
            Self::Scalar(p) => Some(p.size()),
            Self::Array(inner, len) => Some(inner.size()? * len),
            Self::Named(_) => None,
        }
    }

    pub(crate) fn u8_array_len(&self) -> Option<usize> {
        match self {
            Self::Array(inner, len) if inner.primitive()?.rust() == "u8" => Some(*len),
            _ => None,
        }
    }

    /// the type a setter accepts: callers pass host integers, not byteorder wrappers
    pub(crate) fn param_ty(&self) -> Type {
        match self.primitive() {
            Some(p) => {
                let host = p.host();
                parse_quote!(#host)
            }
            None => parse_quote!(#self),
        }
    }

    pub(crate) fn encode(&self, value: TokenStream) -> TokenStream {
        match self.primitive() {
            Some(_) if self.is_wrapped() => {
                let ty = self;
                quote!(#ty::new(#value))
            }
            Some(p) => {
                let host = p.host();
                quote!((#value) as #host)
            }
            None => value,
        }
    }

    /// a schema literal rendered in this type; wrapped primitives need constructing
    pub(crate) fn const_expr(&self, raw: &str) -> Option<TokenStream> {
        let value = self.primitive()?.literal(raw);
        Some(if self.is_wrapped() {
            let ty = self;
            quote!(#ty::new(#value))
        } else {
            value
        })
    }

    /// a fixed-length array constant, padded or truncated to the declared length
    pub(crate) fn const_array(&self, raw: &str, len: usize) -> Option<(TokenStream, TokenStream)> {
        if len == 0 {
            return None;
        }
        let prim = self.primitive()?;
        let elems: Vec<TokenStream> = if prim == Primitive::Char {
            let mut bytes = raw.as_bytes().to_vec();
            bytes.resize(len, 0);
            bytes.iter().map(|b| b.to_token_stream()).collect()
        } else {
            let mut parts: Vec<&str> = raw
                .split(|c: char| c.is_ascii_whitespace() || c == ',')
                .filter(|s| !s.is_empty())
                .collect();
            if parts.is_empty() {
                parts.push(if prim.is_float() { "0.0" } else { "0" });
            }
            let mut out: Vec<TokenStream> = parts
                .iter()
                .map(|part| self.const_expr(part))
                .collect::<Option<_>>()?;
            match out.len() {
                n if n < len => out.resize(len, out.last()?.clone()),
                _ => out.truncate(len),
            }
            out
        };
        let count = len;
        let inner = self;
        Some((quote!([#inner; #count]), quote!([#(#elems),*])))
    }

    /// the single-bit value for a choice in a bit set
    pub(crate) fn bit(&self, index: u32) -> TokenStream {
        let shift = quote!((1u64 << #index));
        let Some(prim) = self.primitive() else {
            return shift;
        };
        let host = prim.host();
        let cast = if host == "u64" {
            shift
        } else {
            quote!(#shift as #host)
        };
        if self.is_wrapped() {
            let ty = self;
            quote!(#ty::new(#cast))
        } else {
            cast
        }
    }

    pub(crate) fn is_int(&self) -> bool {
        self.primitive().is_some_and(|p| !p.is_float())
    }

    pub(crate) fn read_usize(&self, expr: TokenStream) -> TokenStream {
        let read = self.read(expr.clone()).unwrap_or(expr);
        quote!(#read as usize)
    }

    /// the largest group count this dimension type can express
    pub(crate) fn max_count(&self) -> TokenStream {
        match self.primitive() {
            Some(p) => {
                let host = p.host();
                quote!(#host::MAX as usize)
            }
            None => quote!(usize::MAX),
        }
    }

    pub(crate) fn count_value(&self, count: TokenStream) -> TokenStream {
        let Some(p) = self.primitive() else {
            return quote!(#count.try_into().ok().unwrap_or_default());
        };
        let host = p.host();
        if self.is_wrapped() {
            let ty = self;
            quote!(#ty::new(#count as #host))
        } else {
            quote!(#count as #host)
        }
    }

    pub(crate) fn read(&self, expr: TokenStream) -> Option<TokenStream> {
        match self.primitive() {
            Some(_) if self.is_wrapped() => Some(quote!(#expr.get())),
            Some(_) => Some(expr),
            None => None,
        }
    }
}

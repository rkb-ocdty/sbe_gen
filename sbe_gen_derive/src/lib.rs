//! The macro behind a generated block: its constants, its assertions, its builder and its
//! encoder.
//!
//! The generator writes a struct and hangs the schema's numbers off each field. Everything that
//! follows from those is written here, once, instead of being emitted per message and per group
//! into every module. Offsets are `offset_of!`, which is a const expression, so the layout the
//! generator worked out is checked against the struct the compiler actually built rather than
//! being trusted twice.

use darling::{FromField, FromMeta, ast::NestedMeta};
use heck::ToSnakeCase;
use proc_macro::TokenStream as Tokens;
use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use sbe_gen_meta::{Block, Constant, Meta};
use syn::{Fields, Ident, ItemStruct, Type, parse_macro_input};

/// Turns a packed block declaration into the block, its constants, its builder and its encoder.
#[proc_macro_attribute]
pub fn sbe_gen(attr: Tokens, item: Tokens) -> Tokens {
    let block = NestedMeta::parse_meta_list(attr.into())
        .map_err(darling::Error::from)
        .and_then(|args| Block::from_list(&args));
    let block = match block {
        Ok(block) => block,
        Err(e) => return e.write_errors().into(),
    };
    match expand(parse_macro_input!(item as ItemStruct), block) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn expand(mut input: ItemStruct, block: Block) -> syn::Result<TokenStream> {
    let name = input.ident.clone();
    let block_name = name.clone();
    let (builder, encoder, view, body) = (
        format_ident!("{name}Builder"),
        format_ident!("{name}Encoder"),
        format_ident!("{name}View"),
        format_ident!("{name}Body"),
    );

    let Fields::Named(fields) = &mut input.fields else {
        return Err(syn::Error::new_spanned(&name, "sbe_gen needs named fields"));
    };

    let mut written: Vec<(Ident, Type)> = Vec::new();
    let mut constants = TokenStream::new();
    let mut asserts = TokenStream::new();
    let mut accessors = TokenStream::new();
    for field in &mut fields.named {
        let m = Meta::from_field(field).map_err(|e| e.with_span(field))?;
        // ours, and inert nowhere: the struct that comes out the other side must not carry it
        field.attrs.retain(|a| !a.path().is_ident("sbe_gen"));
        let ident = m.ident.expect("named");
        // a trailing underscore is the generator dodging a keyword; a suffixed name goes after
        // the field's own name, not after the dodge
        let stem = ident.to_string();
        let stem = stem.trim_end_matches('_').to_string();

        let ty = m.ty.clone();
        if !m.skip {
            written.push((ident.clone(), ty.clone()));
        }

        let Some(at) = m.offset.map(|o| o as usize).or(m.at) else {
            continue;
        };
        asserts.extend(quote! {
            const _: () = assert!(core::mem::offset_of!(#name, #ident) == #at);
        });

        let since = m.since_version.unwrap_or(0);
        let has = format_ident!("has_{ident}");
        let old = block
            .message
            .then(|| quote!(if self.acting_version < #since as u16 { return false; }));
        accessors.extend(quote! {
            #[inline]
            pub fn #has(&self) -> bool {
                #old
                #at + core::mem::size_of::<#ty>() <= self.acting_block_length
            }
            #[inline]
            pub fn #ident(&self) -> Option<&#ty> {
                if !self.#has() { return None; }
                if let Some(block) = self.body.parsed { return Some(&block.#ident); }
                let bytes = &self.body.raw[#at..#at + core::mem::size_of::<#ty>()];
                ::sbe_support::parse_prefix::<#ty>(bytes).map(|(value, _)| value)
            }
        });

        // the struct's field name is the schema's own, snake-cased, with a trailing underscore
        // where that collided with a keyword — which the shouty spelling does not carry
        let base = ident.to_string().to_uppercase();
        let named = |suffix: &str| format_ident!("{base}_{suffix}");
        if let Some(offset) = m.offset {
            let c = named("OFFSET");
            constants.extend(quote! { pub const #c: u32 = #offset; });
        }
        for (suffix, value) in [
            ("MIN", m.min),
            ("MAX", m.max),
            ("NULL", m.null),
            ("INITIAL", m.initial),
        ] {
            if let Some(value) = value {
                let c = named(suffix);
                constants.extend(quote! { pub const #c: &str = #value; });
            }
        }
        if let (Some(value), Some(optional)) = (&m.value, &m.optional) {
            let (ty, tail, null_when) = (&value.ty, value.read.tail(), optional.test());
            let bind = value.read.wrapper().then(|| quote!(let v = self.#ident;));
            let returned = match value.read.wrapper() {
                true => quote!(v),
                false => quote!(raw),
            };
            let opt = format_ident!("{stem}_opt");
            constants.extend(quote! {
                #[inline]
                pub fn #opt(&self) -> Option<#ty> {
                    #bind
                    let raw = self.#ident #tail;
                    if #null_when { None } else { Some(#returned) }
                }
            });
        }

        if let Some(value) = &m.value {
            let (ty, tail) = (&value.ty, value.read.tail());
            let value_name = format_ident!("{stem}_value");
            let (ret, read) = match &m.optional {
                Some(optional) => {
                    let null_when = optional.test();
                    (
                        quote!(Option<#ty>),
                        quote!({ let raw = (*value) #tail; if #null_when { None } else { Some(raw) } }),
                    )
                }
                None => (quote!(#ty), quote!((*value) #tail)),
            };
            accessors.extend(quote! {
                #[inline]
                pub fn #value_name(&self) -> Option<#ret> {
                    let value = self.#ident()?;
                    Some(#read)
                }
            });
            if m.required {
                let required_name = format_ident!("{stem}_required");
                let reported = ident.to_string();
                accessors.extend(match m.optional.is_some() {
                    true => quote! {
                        #[inline]
                        pub fn #required_name(&self) -> Result<#ty, ::sbe_support::DecodeFieldError> {
                            let value = self.#value_name()
                                .ok_or(::sbe_support::DecodeFieldError::MissingField(#reported))?;
                            value.ok_or(::sbe_support::DecodeFieldError::NullValue(#reported))
                        }
                    },
                    false => quote! {
                        #[inline]
                        pub fn #required_name(&self) -> Result<#ret, ::sbe_support::DecodeFieldError> {
                            self.#value_name()
                                .ok_or(::sbe_support::DecodeFieldError::MissingField(#reported))
                        }
                    },
                });
            }
        }

        if let Some(len) = m.string {
            let (bytes, as_str, trimmed) = (
                format_ident!("{stem}_bytes"),
                format_ident!("{stem}_str"),
                format_ident!("{stem}_str_trimmed"),
            );
            accessors.extend(quote! {
                #[inline(always)]
                pub fn #bytes(&self) -> Option<&[u8; #len]> {
                    self.#ident()
                }
                #[inline]
                pub fn #as_str(&self) -> Option<&str> {
                    core::str::from_utf8(self.#bytes()?).ok()
                }
                #[inline]
                pub fn #trimmed(&self) -> Option<&str> {
                    let bytes = ::sbe_support::trim_ascii_space_and_nul_right(self.#bytes()?);
                    core::str::from_utf8(bytes).ok()
                }
            });
        }

        let (since_name, sem_name) = (named("SINCE_VERSION"), named("SEMANTIC_TYPE"));
        let sem = m
            .semantic_type
            .map(|t| t.to_token_stream().to_string())
            .unwrap_or_default();
        constants.extend(quote! {
            pub const #since_name: u32 = #since;
            pub const #sem_name: &'static str = #sem;
        });
    }

    let setters = |buf: TokenStream, write: TokenStream| {
        written
            .iter()
            .map(|(field, ty)| {
                quote! {
                    pub fn #field(&mut self, value: #ty) -> &mut Self {
                        // taken before the buffer is borrowed mutably, not for readability
                        let at = self.start() + core::mem::offset_of!(#name, #field);
                        ::sbe_support::#write(#buf, at, &value);
                        self
                    }
                }
            })
            .collect::<Vec<_>>()
    };
    // a message builder owns its buffer, an entry builder holds a &mut to its parent's, and
    // reborrowing is the difference between passing the buffer and passing a reference to one
    let build_buf = match block.message {
        true => quote!(&mut self.buf),
        false => quote!(&mut *self.buf),
    };
    let build_setters = setters(build_buf, quote!(write_bytes_at));
    let encode_setters = setters(quote!(self.buf), quote!(write_bytes_into_in_bounds));

    let fixed_len = match (block.block_length, block.size) {
        (Some(declared), Some(size)) => {
            let declared = declared as usize;
            quote!({ if #declared > #size { #declared } else { #size } })
        }
        _ => quote!(core::mem::size_of::<Self>()),
    };
    let group = block.dimension.as_ref().map(|dimension| {
        let base = name.to_string();
        let base = format_ident!("{}", base.strip_suffix("Entry").unwrap_or(&base));
        let parse = match &block.name {
            Some(name) => format_ident!("parse_{name}"),
            None => format_ident!("parse_{}", base.to_string().to_snake_case()),
        };
        let (group_ty, iter, entry_view) = (
            format_ident!("{base}Group"),
            format_ident!("{base}Iter"),
            format_ident!("{base}EntryView"),
        );
        let (group_builder, group_encoder) = (
            format_ident!("{base}GroupBuilder"),
            format_ident!("{base}GroupEncoder"),
        );
        let (since, declared) = (
            block.since_version.unwrap_or(0),
            block.block_length.map_or(0, usize::from),
        );
        quote! {
            pub fn #parse<'a>(buf: &'a [u8]) -> Option<#group_ty<'a>> {
                #group_ty::parse(buf)
            }

            pub type #group_ty<'a> =
                ::sbe_support::Group<'a, #dimension, #entry_view<'a>, #since, #declared>;
            pub type #iter<'a> = ::sbe_support::GroupIter<'a, #entry_view<'a>>;
            pub type #group_builder<'a, A = ::sbe_support::Global> =
                ::sbe_support::GroupBuilder<'a, #dimension, #name, #declared, A>;
            pub type #group_encoder<'a> =
                ::sbe_support::GroupEncoder<'a, #dimension, #name, #declared>;
        }
    });

    // a group is reached through its own builder or encoder, which the group's entry declared;
    // all this side has to know is which ones this block carries and in what order
    let (group_builders, group_encoders): (Vec<_>, Vec<_>) = block
        .group
        .iter()
        .filter(|_| block.message)
        .map(|g| {
            let (ty, name) = (&g.ty, g.name.clone());
            let (builder_ty, encoder_ty) = (
                format_ident!("{ty}GroupBuilder"),
                format_ident!("{ty}GroupEncoder"),
            );
            (
                quote! {
                    pub fn #name<F>(&mut self, f: F) -> Result<&mut Self, ::sbe_support::EncodeError>
                    where
                        F: FnOnce(&mut #builder_ty<'_, A>),
                    {
                        let mut builder = #builder_ty::new(&mut self.buf);
                        f(&mut builder);
                        builder.finish()?;
                        Ok(self)
                    }
                },
                quote! {
                    pub fn #name<F>(&mut self, f: F) -> Result<&mut Self, ::sbe_support::EncodeIntoError>
                    where
                        F: FnOnce(&mut #encoder_ty<'_>) -> Result<(), ::sbe_support::EncodeIntoError>,
                    {
                        let mut encoder = #encoder_ty::new(self.buf, self.used)?;
                        f(&mut encoder)?;
                        self.used = encoder.finish()?;
                        Ok(self)
                    }
                },
            )
        })
        .unzip();

    // the view is the block as a reader sees it: what was parsed, and how much of it the
    // writer's version actually filled. A message keeps the rest of the buffer to walk its
    // groups from; an entry names the ones inside it, since those were parsed on the way past.
    let nested = block.group.iter().map(|g| {
        let (name, group_ty) = (g.name.clone(), format_ident!("{}Group", g.ty));
        quote! { pub #name: #group_ty<'a>, }
    });
    let var_data = block.data.iter().map(|d| {
        let name = &d.name;
        quote! { pub #name: ::sbe_support::VarData<'a>, }
    });
    let whole = (block.message
        && !block.group.is_empty()
        && block.group.iter().all(|g| g.stride.is_some()))
    .then(|| {
        let msg = format_ident!("{name}Message");
        let (as_ref, owned) = (format_ident!("{name}Ref"), format_ident!("{name}Owned"));
        let groups: Vec<_> = block
            .group
            .iter()
            .map(|g| (g.name.clone(), format_ident!("{}Entry", g.ty), g))
            .collect();
        let params: Vec<Ident> = (0..groups.len()).map(|i| format_ident!("G{i}")).collect();
        let members = groups
            .iter()
            .zip(&params)
            .map(|((n, _, _), p)| quote! { pub #n: #p, });
        let bounds = groups
            .iter()
            .zip(&params)
            .map(|((_, e, _), p)| quote! { #p: AsRef<[#e]>, });
        let borrowed = groups.iter().map(|(_, e, _)| quote! { &'a [#e] });
        let owned_args = groups.iter().map(|(_, e, _)| quote! { Vec<#e> });
        let walk = groups.iter().map(|(n, e, g)| {
            let (stride, dimension) = (g.stride, &g.dimension);
            quote! {
                let (header, payload) =
                    <#dimension as ::sbe_support::Dimension>::parse_header(tail)?;
                let count = ::sbe_support::Dimension::count(header);
                let #n = <[#e] as ::zerocopy::FromBytes>::ref_from_bytes(
                    payload.get(..count * #stride)?,
                )
                .ok()?;
                // the writer's own stride, which the schema can declare wider than the entry
                tail = payload.get(count * ::sbe_support::Dimension::block_length(header)..)?;
            }
        });
        let names: Vec<_> = groups.iter().map(|(n, _, _)| n.clone()).collect();

        quote! {
            /// The whole message: its block, and every group.
            #[derive(Debug, Clone, Copy, PartialEq, Eq)]
            pub struct #msg<B, #(#params),*> {
                pub block: B,
                #(#members)*
            }

            /// borrowed straight out of the packet
            pub type #as_ref<'a> = #msg<&'a #name, #(#borrowed),*>;
            /// owning what it holds, which is what a deserialiser builds
            pub type #owned = #msg<#name, #(#owned_args),*>;

            impl<B: AsRef<#name>, #(#params),*> core::ops::Deref for #msg<B, #(#params),*> {
                type Target = #name;
                fn deref(&self) -> &#name {
                    self.block.as_ref()
                }
            }

            impl<B: AsRef<#name>, #(#params),*> #msg<B, #(#params),*>
            where
                #(#bounds)*
            {
                pub fn to_owned(&self) -> #owned {
                    #owned {
                        block: *self.block.as_ref(),
                        #(#names: self.#names.as_ref().to_vec(),)*
                    }
                }
            }

            impl<'a> #as_ref<'a> {
                /// The message as written, or `None` if this build cannot read all of it.
                ///
                /// A writer on an older schema stops its block short of a field this build
                /// knows about, and there is no honest value to hand back for it. Taking the
                /// header is what makes a whole message the only thing this can hold.
                pub fn parse(
                    body: &'a [u8],
                    header: &::sbe_support::MessageHeader,
                ) -> Option<Self> {
                    let written = header.block_length.get() as usize;
                    if written < core::mem::size_of::<#name>() {
                        return None;
                    }
                    let (block, _) = ::sbe_support::parse_prefix::<#name>(body)?;
                    #[allow(unused_mut)]
                    let mut tail = body.get(written..)?;
                    #(#walk)*
                    Some(Self { block, #(#names,)* })
                }

                /// the same, reading the header off the front of `buf`
                pub fn parse_message(buf: &'a [u8]) -> Option<Self> {
                    let (header, body) = ::sbe_support::MessageHeader::parse_prefix(buf)?;
                    Self::parse(body, header)
                }
            }
        }
    });

    let view_struct = match block.message {
        true => quote! {
            #[derive(Debug, Clone)]
            pub struct #view<'a> {
                pub body: #body<'a>,
                /// everything after the fixed block: the groups in declaration order, then the
                /// var-data. Held so a group can be reached by walking from here rather than
                /// only when it happens to be the trailing member.
                pub tail: &'a [u8],
                pub acting_block_length: usize,
                pub acting_version: u16,
            }

            pub fn parse_with_header<'a>(
                body: &'a [u8],
                header: &::sbe_support::MessageHeader,
            ) -> Option<(#view<'a>, &'a [u8])> {
                let mut acting_block_length = header.block_length.get() as usize;
                if acting_block_length == 0 {
                    acting_block_length = #name::BLOCK_LENGTH as usize;
                }
                let acting_version = header.version.get();
                let parsed = #body::parse(body, acting_block_length)?;
                let (_, rest) = body.split_at(acting_block_length);
                let view = #view { body: parsed, tail: rest, acting_block_length, acting_version };
                Some((view, rest))
            }
        },
        false => quote! {
            #[derive(Debug, Clone)]
            pub struct #view<'a> {
                pub body: #body<'a>,
                pub acting_block_length: usize,
                #(#nested)*
                #(#var_data)*
            }
        },
    };

    // An entry is as long as its contents: the fixed block, then each nested group with its own
    // header and entries, then each var-data behind its length. Walking it is the only way to
    // reach the next entry, and the order is the order the schema declared.
    let entry_impl_for_view = block.dimension.as_ref().map(|_| {
        let mut steps: Vec<(usize, TokenStream, Ident)> = Vec::new();
        for g in &block.group {
            let name = g.name.clone();
            let parse = format_ident!("parse_{name}");
            steps.push((
                g.order,
                quote! {
                    let #name = #parse(tail)?;
                    tail = #name.skip()?;
                },
                name,
            ));
        }
        for d in &block.data {
            let (name, len) = (&d.name, &d.len);
            steps.push((
                d.order,
                quote! {
                    let (#name, after) = ::sbe_support::parse_var_data::<#len>(tail)?;
                    tail = after;
                },
                name.clone(),
            ));
        }
        steps.sort_by_key(|(order, _, _)| *order);
        let walk: Vec<_> = steps.iter().map(|(_, step, _)| step.clone()).collect();
        let inits: Vec<_> = steps.iter().map(|(_, _, name)| name.clone()).collect();

        quote! {
            impl<'a> ::sbe_support::GroupEntry<'a> for #view<'a> {
                fn parse_entry(cursor: &'a [u8], blen: usize) -> Option<(Self, &'a [u8])> {
                    if blen > cursor.len() {
                        return None;
                    }
                    #[allow(unused_mut)]
                    let (entry, mut tail) = cursor.split_at(blen);
                    let body = #body::parse(entry, blen)?;
                    #(#walk)*
                    Some((#view { body, acting_block_length: blen, #(#inits,)* }, tail))
                }
            }
        }
    });

    // A constant occupies no bytes, so it is not a field and cannot be read from one. On a
    // message it still answers `has_`, because the schema can add a constant in a later
    // version and an older writer will not have known about it.
    let mut constant_items = TokenStream::new();
    for Constant {
        name,
        ident,
        ty,
        value,
        since_version,
    } in &block.constant
    {
        constant_items.extend(quote! {
            pub const #ident: #ty = #value;
            #[inline]
            pub fn #name(&self) -> #ty {
                Self::#ident
            }
        });
        if block.message {
            let has = format_ident!("has_{name}");
            accessors.extend(quote! {
                #[inline]
                pub fn #has(&self) -> bool {
                    self.acting_version >= #since_version as u16
                }
                #[inline]
                pub fn #name(&self) -> Option<#ty> {
                    if !self.#has() { return None; }
                    Some(#block_name::#ident)
                }
            });
        }
    }

    let machinery = match block.message {
        true => message_impl(
            &name,
            &builder,
            &encoder,
            &fixed_len,
            build_setters,
            encode_setters,
            group_builders,
            group_encoders,
        ),
        false => entry_impl(
            &name,
            &builder,
            &encoder,
            &fixed_len,
            build_setters,
            encode_setters,
        ),
    };

    // A group whose entries are all the same width is a contiguous array on the wire, so the
    // whole thing is one `ref_from_bytes` rather than an iterator. Reaching it means stepping
    // over the groups in front of it, which is a read of each one's dimension header — and
    // that step is only right while those groups have a stride too.
    let mut slices = TokenStream::new();
    if block.message {
        for (i, g) in block.group.iter().enumerate() {
            let ahead = &block.group[..i];
            if g.stride.is_none() || ahead.iter().any(|p| p.stride.is_none()) {
                continue;
            }
            let (name, entry, stride, dimension) = (
                g.name.clone(),
                format_ident!("{}Entry", g.ty),
                g.stride,
                &g.dimension,
            );
            let skips = ahead.iter().map(|p| {
                let dimension = &p.dimension;
                quote! {{
                    let (header, payload) = <#dimension as ::sbe_support::Dimension>::parse_header(cursor)?;
                    let step = ::sbe_support::Dimension::count(header)
                        * ::sbe_support::Dimension::block_length(header);
                    cursor = payload.get(step..)?;
                }}
            });
            slices.extend(quote! {
                /// the group's entries, laid end to end after its dimension header
                pub fn #name(&self) -> Option<&'a [#entry]> {
                    #[allow(unused_mut)]
                    let mut cursor = self.tail;
                    #(#skips)*
                    let (header, payload) =
                        <#dimension as ::sbe_support::Dimension>::parse_header(cursor)?;
                    let len = ::sbe_support::Dimension::count(header) * #stride;
                    <[#entry] as ::zerocopy::FromBytes>::ref_from_bytes(payload.get(..len)?).ok()
                }
            });
        }
    }

    let fixed_layout = block.message.then(|| {
        quote! {
            #[inline]
            pub fn is_fixed_layout(&self) -> bool {
                self.acting_version >= #name::SCHEMA_VERSION
                    && self.acting_block_length >= core::mem::size_of::<#name>()
            }
        }
    });
    let view_impl =
        (!accessors.is_empty() || fixed_layout.is_some() || !slices.is_empty()).then(|| {
            quote! {
                impl<'a> #view<'a> {
                    #fixed_layout
                    #accessors
                    #slices
                }
            }
        });

    let wire = block
        .block_length
        .filter(|_| block.message)
        .map(|block_length| {
            let (template, schema, version) = (
                block.template_id.unwrap_or(0),
                block.schema_id.unwrap_or(0),
                block.schema_version.unwrap_or(0),
            );
            quote! {
                pub const BLOCK_LENGTH: u16 = #block_length;
                pub const TEMPLATE_ID: u16 = #template;
                pub const SCHEMA_ID: u16 = #schema;
                pub const SCHEMA_VERSION: u16 = #version;
            }
        });
    let since = block
        .since_version
        .map(|v| quote! { pub const SINCE_VERSION: u32 = #v; });
    let semantic = block
        .semantic_type
        .map(|s| quote! { pub const SEMANTIC_TYPE: &'static str = #s; });
    // a derivation may swap a field's type; the wire layout survives that only if every field
    // keeps its offset and the struct keeps its width, so the schema's own numbers are asserted
    // against what the struct actually compiled to
    let size = block
        .size
        .map(|size| quote! { const _: () = assert!(core::mem::size_of::<#name>() == #size); });

    // the block is bytes on the wire in declaration order, always, so the layout attributes
    // belong here rather than being written into every module alongside every struct
    Ok(quote! {
        #[repr(C)]
        #[derive(
            Debug,
            Clone,
            Copy,
            ::zerocopy::FromBytes,
            ::zerocopy::IntoBytes,
            ::zerocopy::KnownLayout,
            ::zerocopy::Immutable,
            ::zerocopy::Unaligned,
            PartialEq,
            Eq,
        )]
        #input

        #size
        #asserts

        impl AsRef<#name> for #name {
            fn as_ref(&self) -> &#name {
                self
            }
        }

        impl #name {
            // ::sbe_support::ParsePrefix provides this for anything blanket, but callers had it
            // as an inherent method and an inherent method is what they still resolve to
            #[inline]
            pub fn parse_prefix(body: &[u8]) -> Option<(&Self, &[u8])> {
                ::sbe_support::parse_prefix(body)
            }
            #wire
            #since
            #semantic
            #constants
            #constant_items
        }

        pub type #body<'a> = ::sbe_support::EntryBody<'a, #name>;

        #group
        #whole
        #view_struct
        #entry_impl_for_view
        #view_impl
        #machinery
    })
}

/// A message: the builder owns the buffer it fills, and the block starts at zero.
#[allow(clippy::too_many_arguments)]
fn message_impl(
    name: &Ident,
    builder: &Ident,
    encoder: &Ident,
    fixed_len: &TokenStream,
    build_setters: Vec<TokenStream>,
    encode_setters: Vec<TokenStream>,
    group_builders: Vec<TokenStream>,
    group_encoders: Vec<TokenStream>,
) -> TokenStream {
    quote! {
        pub struct #builder<A: ::sbe_support::Allocator = ::sbe_support::Global> {
            buf: ::sbe_support::Vec<u8, A>,
        }

        impl Default for #builder {
            fn default() -> Self {
                Self::new()
            }
        }

        impl #builder {
            pub fn new() -> Self {
                Self::new_in(::sbe_support::Global)
            }

            pub fn with_capacity(capacity: usize) -> Self {
                let mut this = Self::new();
                this.buf.reserve(capacity);
                this
            }
        }

        impl<A: ::sbe_support::Allocator> #builder<A> {
            pub const BLOCK_LENGTH: u16 = #name::BLOCK_LENGTH;
            pub const TEMPLATE_ID: u16 = #name::TEMPLATE_ID;
            pub const SCHEMA_ID: u16 = #name::SCHEMA_ID;
            pub const SCHEMA_VERSION: u16 = #name::SCHEMA_VERSION;

            #[inline]
            fn start(&self) -> usize { 0 }

            pub fn new_in(alloc: A) -> Self {
                let mut buf = ::sbe_support::Vec::new_in(alloc);
                buf.resize(Self::BLOCK_LENGTH as usize, 0);
                Self { buf }
            }

            #(#build_setters)*
            #(#group_builders)*

            pub fn finish(self) -> ::sbe_support::Vec<u8, A> {
                self.buf
            }

            pub fn finish_with_header(self) -> Vec<u8> {
                let header = ::sbe_support::MessageHeader::new(
                    Self::BLOCK_LENGTH,
                    Self::TEMPLATE_ID,
                    Self::SCHEMA_ID,
                    Self::SCHEMA_VERSION,
                );
                let mut out = Vec::with_capacity(
                    self.buf.len() + ::sbe_support::MessageHeader::SIZE,
                );
                out.extend_from_slice(&header.to_bytes());
                out.extend_from_slice(&self.buf);
                out
            }
        }

        /// Zero-allocation encoder writing directly into caller-provided memory.
        pub struct #encoder<'a> {
            buf: &'a mut [u8],
            used: usize,
        }

        impl<'a> #encoder<'a> {
            pub const BLOCK_LENGTH: u16 = #name::BLOCK_LENGTH;
            pub const FIXED_LEN: usize = #fixed_len;

            #[inline]
            fn start(&self) -> usize { 0 }

            /// Create an encoder over `buf`.
            ///
            /// The slice must have at least `FIXED_LEN` bytes. The fixed block is
            /// zero-initialized, and variable-size fields append after it.
            pub fn new(buf: &'a mut [u8]) -> Result<Self, ::sbe_support::EncodeIntoError> {
                if buf.len() < Self::FIXED_LEN {
                    return Err(::sbe_support::EncodeIntoError::BufferTooSmall {
                        required: Self::FIXED_LEN,
                        available: buf.len(),
                    });
                }
                buf[..Self::FIXED_LEN].fill(0);
                Ok(Self { buf, used: Self::FIXED_LEN })
            }

            pub fn encoded_len(&self) -> usize {
                self.used
            }

            pub fn as_slice(&self) -> &[u8] {
                &self.buf[..self.used]
            }

            #(#encode_setters)*
            #(#group_encoders)*

            pub fn finish(self) -> usize {
                self.used
            }
        }

        impl ::sbe_support::MessageEncode for #name {
            type Encoder<'a> = #encoder<'a>;
            fn encoder(dst: &mut [u8]) -> Result<Self::Encoder<'_>, ::sbe_support::EncodeIntoError> {
                #encoder::new(dst)
            }
            fn finish(encoder: Self::Encoder<'_>) -> usize {
                encoder.finish()
            }
        }
    }
}

/// A group entry: one of many in a buffer someone else owns, so both carry where it starts.
fn entry_impl(
    name: &Ident,
    builder: &Ident,
    encoder: &Ident,
    fixed_len: &TokenStream,
    build_setters: Vec<TokenStream>,
    encode_setters: Vec<TokenStream>,
) -> TokenStream {
    quote! {
        pub struct #builder<'a, A: ::sbe_support::Allocator = ::sbe_support::Global> {
            buf: &'a mut ::sbe_support::Vec<u8, A>,
            start: usize,
        }

        impl<A: ::sbe_support::Allocator> ::sbe_support::GroupEntryBuilder<A> for #name {
            type Builder<'b> = #builder<'b, A> where A: 'b;
            fn builder(buf: &mut ::sbe_support::Vec<u8, A>, start: usize) -> Self::Builder<'_> {
                #builder { buf, start }
            }
        }

        impl<'a, A: ::sbe_support::Allocator> #builder<'a, A> {
            #[inline]
            fn start(&self) -> usize { self.start }

            #(#build_setters)*
        }

        pub struct #encoder<'a> {
            buf: &'a mut [u8],
            cursor: usize,
            start: usize,
        }

        impl ::sbe_support::GroupEntryEncoder for #name {
            type Encoder<'b> = #encoder<'b>;
            const FIXED_LEN: usize = #fixed_len;
            fn encoder(
                buf: &mut [u8],
                cursor: usize,
            ) -> Result<Self::Encoder<'_>, ::sbe_support::EncodeIntoError> {
                #encoder::new(buf, cursor)
            }
            fn finish(encoder: Self::Encoder<'_>) -> usize {
                encoder.finish()
            }
        }

        impl<'a> #encoder<'a> {
            #[inline]
            fn start(&self) -> usize { self.start }

            pub fn new(
                buf: &'a mut [u8],
                cursor: usize,
            ) -> Result<Self, ::sbe_support::EncodeIntoError> {
                let start = cursor;
                let end = start + <#name as ::sbe_support::GroupEntryEncoder>::FIXED_LEN;
                if end > buf.len() {
                    return Err(::sbe_support::EncodeIntoError::BufferTooSmall {
                        required: end,
                        available: buf.len(),
                    });
                }
                buf[start..end].fill(0);
                Ok(Self { buf, cursor: end, start })
            }

            #(#encode_setters)*

            pub fn finish(self) -> usize {
                self.cursor
            }
        }
    }
}

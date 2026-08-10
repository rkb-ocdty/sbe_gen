use super::*;
use crate::{Generated, GeneratedModule};
use sbe_gen_meta as meta;

/// Stage 5. Walks the laid-out schema and renders each node into a module.
///
/// The last pass, and the only one that produces text: everything it needs was decided by the
/// passes before it, so each method here is a template with nothing left to work out.
pub(crate) struct Emit<'a> {
    pub(crate) schema: &'a LaidOutSchema,
    pub(crate) derivations: &'a [&'a dyn Derivation],
    /// every struct written out, kept for the caller to introspect
    structs: RefCell<Vec<ItemStruct>>,
}

impl<'a> Emit<'a> {
    pub(crate) fn new(schema: &'a LaidOutSchema, derivations: &'a [&'a dyn Derivation]) -> Self {
        Self {
            schema,
            derivations,
            structs: RefCell::default(),
        }
    }

    pub(crate) fn generated(&self) -> Result<Generated, CodegenError> {
        // lint levels are inherited by every module below, so `mod.rs` is the only place any
        // of this has to be said
        const MODULE_ALLOW: &str = "#![allow(dead_code, non_camel_case_types)]";
        let allows: Vec<&str> = [MODULE_ALLOW]
            .into_iter()
            .chain(self.schema.opts.allow_attr.as_deref())
            .collect();
        Ok(Generated {
            types: {
                let (body, derived) = (self.visit_types()?, self.derived_types()?);
                self.module("types.rs".to_string(), &[], quote! { #body #derived })?
            },
            messages: self
                .schema
                .messages
                .iter()
                .map(|msg| {
                    let name = format!("{}.rs", msg.msg.name.snake_ident());
                    let (body, derived) = (self.visit_message(msg)?, self.derived_message(msg)?);
                    self.module(name, &[], quote! { #body #derived })
                })
                .collect::<Result<_, _>>()?,
            // MessageHeader lives in sbe_support now, but consumers wrote
            // `use ...::message_header::*` against the module that used to hold it
            message_header: self.module(
                "message_header.rs".to_string(),
                &[],
                quote! { pub use ::sbe_support::MessageHeader; },
            )?,
            groups: self
                .visit_shared_groups()?
                .map(|body| self.module("groups.rs".to_string(), &[], body))
                .transpose()?,
            mod_rs: self.module("mod.rs".to_string(), &allows, self.visit_mod())?,
            derived: self.derived_modules()?,
            structs: self.structs.take(),
        })
    }

    fn visit_groups(&self, groups: &[PlacedGroup]) -> Result<TokenStream, CodegenError> {
        groups.iter().map(|g| self.visit_group(g)).collect()
    }

    /// A message's own groups. One that more than one message declares is written into
    /// `groups.rs` instead and named here, so the module's surface is the same either way.
    fn visit_message_groups(&self, groups: &[PlacedGroup]) -> Result<TokenStream, CodegenError> {
        groups
            .iter()
            .map(|g| match self.schema.is_shared(&g.group) {
                false => self.visit_group(g),
                true => {
                    let names = group_exports(g);
                    Ok(quote! { pub use super::groups::{#(#names),*}; })
                }
            })
            .collect()
    }

    /// `groups.rs`: every repeated group, written from the first message that declares it. The
    /// occurrences are equal as parse trees, so they lay out the same and any one of them
    /// renders the same code.
    fn visit_shared_groups(&self) -> Result<Option<TokenStream>, CodegenError> {
        let mut seen: Vec<&Group> = Vec::new();
        let mut items = TokenStream::new();
        for g in self.schema.messages.iter().flat_map(|m| &m.groups) {
            if self.schema.is_shared(&g.group) && !seen.contains(&&g.group) {
                seen.push(&g.group);
                items.extend(self.visit_group(g)?);
            }
        }
        Ok((!items.is_empty()).then_some(items))
    }

    /// The members of the packed struct: each field that occupies bytes, behind the padding the
    /// layout pass decided it needs.
    fn visit_fields(&self, fields: &[PlacedField]) -> Result<TokenStream, CodegenError> {
        fields
            .iter()
            .filter(|f| !f.is_constant())
            .map(|f| self.visit_field(f))
            .collect()
    }

    fn visit_field(&self, f: &PlacedField) -> Result<TokenStream, CodegenError> {
        let PlacedField {
            padding, pad_name, ..
        } = f;
        let pad = pad_name.as_ref().map(|name| {
            let mut pad: SynField = parse_quote!(#[sbe_gen(skip)] #name: [u8; #padding]);
            self.derive_padding(&mut pad);
            quote! { #pad, }
        });
        let (name, ty, doc) = (&f.name, &f.ty, &f.field.description);
        let meta = field_meta_attr(f);
        let mut field: SynField = parse_quote!(#doc #meta pub #name: #ty);
        self.derive_field(f, &mut field)?;
        Ok(quote! { #pad #field, })
    }

    fn module(
        &self,
        name: String,
        allows: &[&str],
        body: TokenStream,
    ) -> Result<GeneratedModule, CodegenError> {
        Ok(GeneratedModule {
            source: render_file(&name, allows, body)?,
            name,
        })
    }

    /// Generate the content of `mod.rs` re‑exporting messages and types.
    fn visit_mod(&self) -> TokenStream {
        let modules = self
            .schema
            .messages
            .iter()
            .map(|msg| msg.msg.name.snake_ident());
        let reexports = self.schema.messages.iter().map(|msg| {
            let module = msg.msg.name.snake_ident();
            let (msg_ty, builder_ty) = (&msg.names.msg, &msg.names.builder);
            quote! {
                pub use #module::#msg_ty;
                pub use #module::#builder_ty;
            }
        });
        let shared = match self.schema.shared.is_empty() {
            true => quote!(),
            false => quote!(
                pub mod groups;
            ),
        };
        quote! {
            pub mod types;
            pub mod message_header;
            #shared
            #(pub mod #modules;)*

            pub use ::sbe_support::{self, MessageHeader, framed};
            #(#reexports)*
        }
    }

    /// Generate the `types.rs` module containing definitions of enums, sets and composites.
    fn visit_types(&self) -> Result<TokenStream, CodegenError> {
        let Rendered {
            primitives,
            enums,
            sets,
            composites,
        } = Types::new(self.schema, self.schema.opts, self.derivations).collect()?;
        Ok(quote! {
            #(#primitives)*
            #(#enums)*
            #(#sets)*
            #(#composites)*
        })
    }

    fn visit_message(&self, lowered: &PlacedMessage) -> Result<TokenStream, CodegenError> {
        let PlacedMessage {
            msg,
            struct_size,
            names:
                MessageNames {
                    msg: msg_name,
                    builder: builder_name,
                    encoder: encoder_name,
                    view: view_name,
                },
            block_length: msg_block_length,
            fields,
            data: data_layout,
            groups,
        } = lowered;
        let msg_block_length = *msg_block_length;
        let (group_at, data_at) = member_order(msg.members.iter().map(|m| match m {
            MessageMember::Group(_) => Some(true),
            MessageMember::Data(_) => Some(false),
            MessageMember::Field(_) => None,
        }));
        let msg_groups = group_keys(groups, &group_at);
        let msg_data = var_data_keys(data_layout, &data_at);
        let msg_constants = constant_keys(fields);
        let (template_id, msg_since) = (msg.id, msg.since_version.unwrap_or(0));
        let (schema_id, schema_version) = (
            self.schema.schema_id.unwrap_or(0),
            self.schema.version.unwrap_or(0),
        );
        let msg_semantic = semantic(msg.semantic_type.as_deref());

        let mut msg_struct: ItemStruct = {
            let struct_fields = self.visit_fields(fields)?;
            parse_quote! {
                #[::sbe_support::sbe_gen(
                    message,
                    #msg_groups
                    #msg_data
                    #msg_constants
                    size = #struct_size,
                    block_length = #msg_block_length,
                    template_id = #template_id,
                    schema_id = #schema_id,
                    schema_version = #schema_version,
                    since_version = #msg_since,
                    #msg_semantic
                )]
                pub struct #msg_name { #struct_fields }
            }
        };
        self.derive_message_struct(lowered, &mut msg_struct)?;
        self.structs.borrow_mut().push(msg_struct.clone());

        let parse_data_methods = data_layout.iter().map(|d| {
            let (fn_name, length_ty) = (&d.parse_fn, &d.length_ty);
            quote! {
                #[inline]
                pub fn #fn_name<'a>(&self, buf: &'a [u8]) -> Option<(::sbe_support::VarData<'a>, &'a [u8])> {
                    ::sbe_support::parse_var_data::<#length_ty>(buf)
                }
            }
        });

        let plain_left = || fields.iter().filter(|f| !f.setter_is_derivable());
        let builder_setters = field_setters(plain_left(), |offset| {
            quote! { ::sbe_support::write_bytes_at(&mut self.buf, #offset, &encoded); }
        });
        let builder_data = data_layout.iter().map(|d| {
            let (name, length_ty) = (&d.name, &d.length_ty);
            quote! {
                pub fn #name(&mut self, bytes: &[u8]) -> Result<&mut Self, ::sbe_support::EncodeError> {
                    ::sbe_support::write_var_data::<#length_ty, _>(&mut self.buf, bytes)?;
                    Ok(self)
                }
            }
        });

        let encoder_setters = field_setters(plain_left(), |offset| {
            quote! { ::sbe_support::write_bytes_into_in_bounds(self.buf, #offset, &encoded); }
        });
        let encoder_data = data_layout.iter().map(|d| {
            let (name, length_ty) = (&d.name, &d.length_ty);
            quote! {
                pub fn #name(&mut self, bytes: &[u8]) -> Result<&mut Self, ::sbe_support::EncodeIntoError> {
                    let written = ::sbe_support::write_var_data_into::<#length_ty>(self.buf, self.used, bytes)?;
                    self.used += written;
                    Ok(self)
                }
            }
        });

        let group_items = self.visit_message_groups(groups)?;

        let view_fields = fields
            .iter()
            .filter(|f| !f.is_constant())
            .map(view_field_helpers);

        let view_data_methods = data_layout.iter().map(|d| {
            let (fn_name, length_ty) = (&d.parse_fn, &d.length_ty);
            quote! {
                #[inline]
                pub fn #fn_name<'b>(&self, buf: &'b [u8]) -> Option<(::sbe_support::VarData<'b>, &'b [u8])> {
                    ::sbe_support::parse_var_data::<#length_ty>(buf)
                }
            }
        });

        let msg_impl = impl_of(quote!(#msg_name), parse_data_methods.collect());
        let builder_impl = impl_of(
            quote!(<A: ::sbe_support::Allocator> #builder_name<A>),
            builder_setters.chain(builder_data).collect(),
        );
        let encoder_impl = impl_of(
            quote!(<'a> #encoder_name<'a>),
            encoder_setters.chain(encoder_data).collect(),
        );
        let view_impl = impl_of(
            quote!(<'a> #view_name<'a>),
            view_fields.chain(view_data_methods).collect(),
        );

        Ok(quote! {
            #msg_struct

            #msg_impl

            #builder_impl

            #encoder_impl

            #group_items

            #view_impl

        })
    }

    fn visit_group(&self, lowered: &PlacedGroup) -> Result<TokenStream, CodegenError> {
        let PlacedGroup {
            group: g,
            dimension,
            block_length: g_block_length,
            struct_size: entry_struct_size,
            fields: g_fields,
            data: g_data_layout,
            groups: g_nested,
            ..
        } = lowered;
        let entry_struct = g.entry_struct();
        let entry_builder = g.entry_builder();
        let entry_encoder = g.entry_encoder();

        let g_block_length = *g_block_length;

        let g_since = g.since_version.unwrap_or(0);
        let g_semantic = semantic(g.semantic_type.as_deref());

        let (group_at, data_at) = member_order(g.members.iter().map(|m| match m {
            GroupMember::Group(_) => Some(true),
            GroupMember::Data(_) => Some(false),
            GroupMember::Field(_) => None,
        }));
        let nested_list = group_keys(g_nested, &group_at);
        let data_list = var_data_keys(g_data_layout, &data_at);
        let entry_constants_keys = constant_keys(g_fields);
        let entry_block_size = (*entry_struct_size).max(g_block_length);
        let g_accessor = g.name.snake_ident();

        let entry_doc = g
            .description
            .as_ref()
            .map(|d| Docs::new(format!("Group: {}", d.text)));
        let mut entry_struct_item: ItemStruct = {
            let entry_fields = self.visit_fields(g_fields)?;
            // SBE lays entries `blockLength` apart, so the struct has to be that wide. A
            // narrower one still decodes entry zero and then walks off the wire's grid, which
            // is how a slice of them silently returned the wrong bytes.
            let tail_pad = g_block_length.saturating_sub(*entry_struct_size);
            let end_pad = (tail_pad > 0).then(|| {
                let mut pad: SynField =
                    parse_quote!(#[sbe_gen(skip)] __padding_end: [u8; #tail_pad]);
                self.derive_padding(&mut pad);
                quote! { #pad, }
            });
            parse_quote! {
                #[::sbe_support::sbe_gen(
                    size = #entry_block_size,
                    block_length = #g_block_length,
                    dimension = #dimension,
                    name = #g_accessor,
                    #nested_list
                    #data_list
                    #entry_constants_keys
                    since_version = #g_since,
                    #g_semantic
                )]
                pub struct #entry_struct { #entry_fields #end_pad }
            }
        };
        self.derive_group_struct(lowered, &mut entry_struct_item)?;
        self.structs.borrow_mut().push(entry_struct_item.clone());

        let entry_view_impl = quote!();

        let derived_here = || g_fields.iter().filter(|f| !f.setter_is_derivable());
        let entry_builder_setters = field_setters(derived_here(), |offset| {
            quote! { ::sbe_support::write_bytes_at(&mut *self.buf, self.start + #offset, &encoded); }
        });
        let entry_builder_nested = g_nested.iter().map(|nested| {
            let nested = &nested.group;
            let name = nested.name.snake_ident();
            let builder = format_ident!("{}GroupBuilder", nested.name.type_ident());
            quote! {
                pub fn #name<F>(&mut self, f: F) -> Result<&mut Self, ::sbe_support::EncodeError>
                where
                    F: FnOnce(&mut #builder<'_, A>),
                {
                    let mut builder = #builder::new(self.buf);
                    f(&mut builder);
                    builder.finish()?;
                    Ok(self)
                }
            }
        });
        let entry_builder_data = g_data_layout.iter().map(|d| {
            let (name, length_ty) = (&d.name, &d.length_ty);
            quote! {
                pub fn #name(&mut self, bytes: &[u8]) -> Result<&mut Self, ::sbe_support::EncodeError> {
                    ::sbe_support::write_var_data::<#length_ty, _>(&mut *self.buf, bytes)?;
                    Ok(self)
                }
            }
        });

        let entry_encoder_setters = field_setters(derived_here(), |offset| {
            quote! { ::sbe_support::write_bytes_into_in_bounds(self.buf, self.start + #offset, &encoded); }
        });
        let entry_encoder_nested = g_nested.iter().map(|nested| {
            let nested = &nested.group;
            let name = nested.name.snake_ident();
            let encoder = format_ident!("{}GroupEncoder", nested.name.type_ident());
            quote! {
                pub fn #name<F>(&mut self, f: F) -> Result<&mut Self, ::sbe_support::EncodeIntoError>
                where
                    F: FnOnce(&mut #encoder<'_>) -> Result<(), ::sbe_support::EncodeIntoError>,
                {
                    let mut encoder = #encoder::new(self.buf, self.cursor)?;
                    f(&mut encoder)?;
                    self.cursor = encoder.finish()?;
                    Ok(self)
                }
            }
        });
        let entry_encoder_data = g_data_layout.iter().map(|d| {
            let (name, length_ty) = (&d.name, &d.length_ty);
            quote! {
                pub fn #name(&mut self, bytes: &[u8]) -> Result<&mut Self, ::sbe_support::EncodeIntoError> {
                    let written = ::sbe_support::write_var_data_into::<#length_ty>(self.buf, self.cursor, bytes)?;
                    self.cursor += written;
                    Ok(self)
                }
            }
        });

        let extra = |head: TokenStream, methods: Vec<TokenStream>| match methods.is_empty() {
            true => quote!(),
            false => quote! { impl #head { #(#methods)* } },
        };
        let entry_builder_extra = extra(
            quote!(<'a, A: ::sbe_support::Allocator> #entry_builder<'a, A>),
            entry_builder_setters
                .chain(entry_builder_nested)
                .chain(entry_builder_data)
                .collect(),
        );
        let entry_encoder_extra = extra(
            quote!(<'a> #entry_encoder<'a>),
            entry_encoder_setters
                .chain(entry_encoder_nested)
                .chain(entry_encoder_data)
                .collect(),
        );

        let nested_items = self.visit_groups(g_nested)?;

        Ok(quote! {


            #entry_doc
            #entry_struct_item


            #entry_view_impl



            #entry_builder_extra
            #entry_encoder_extra



            #nested_items
        })
    }
}

/// every name a group writes, so a message that no longer holds the definition can name them
/// all back into its own module. Nested groups come with it: they are rendered inside their
/// parent, so they move when it does.
fn group_exports(g: &PlacedGroup) -> Vec<Ident> {
    let gr = &g.group;
    [
        gr.view_struct(),
        gr.iter_struct(),
        gr.entry_body(),
        gr.entry_view(),
        gr.entry_struct(),
        gr.builder(),
        gr.entry_builder(),
        gr.encoder(),
        gr.entry_encoder(),
        gr.parse_fn(),
    ]
    .into_iter()
    .chain(g.groups.iter().flat_map(group_exports))
    .collect()
}

/// A block's FIX message-type code, when the schema gave it one. An empty string is what the
/// XML says when it did not, and saying that is worse than saying nothing.
fn semantic(value: Option<&str>) -> TokenStream {
    match value.filter(|s| !s.is_empty()) {
        Some(value) => quote!(semantic_type = #value,),
        None => quote!(),
    }
}

/// An impl block, or nothing when the schema left it with no members to put in it.
fn impl_of(target: TokenStream, members: Vec<TokenStream>) -> TokenStream {
    // a field that contributes nothing still yields an empty stream, so emptiness is per token
    let members: Vec<_> = members.into_iter().filter(|m| !m.is_empty()).collect();
    match members.is_empty() {
        true => quote!(),
        false => quote! { impl #target { #(#members)* } },
    }
}

/// One `group(..)` per repeating group, in wire order. A stride is only given where every
/// entry is the same width; without one the group cannot be handed back as a slice, and nor
/// can any group behind it, because stepping over it reads a width that is not there.
fn group_keys(groups: &[PlacedGroup], order: &[usize]) -> TokenStream {
    groups
        .iter()
        .zip(order)
        .map(|(g, order)| {
            let (ty, dimension) = (g.group.name.type_ident(), g.dimension.clone());
            let name = g.group.name.snake_ident();
            let stride = (g.data.is_empty() && g.groups.is_empty())
                .then(|| g.struct_size.max(g.block_length));
            let stride = stride.map(|s| quote!(stride = #s,));
            quote!(group(order = #order, name = #name, ty = #ty, dimension = #dimension, #stride),)
        })
        .collect()
}

/// One `constant(..)` per field the schema fixed the value of.
fn constant_keys(fields: &[PlacedField]) -> TokenStream {
    fields
        .iter()
        .filter(|f| f.is_constant())
        .filter_map(|f| {
            let (name, ident) = (&f.name, &f.view.const_name);
            let (ty, value) = f.view.constant.as_ref()?;
            let (ty, value) = (ty.to_string(), value.to_string());
            let since = f.field.since_version.unwrap_or(0);
            Some(quote!(constant(name = #name, ident = #ident, ty = #ty, value = #value, since_version = #since),))
        })
        .collect()
}

/// One `data(name = .., len = ..)` per variable-length member, in wire order.
fn var_data_keys(data: &[DataLayout], order: &[usize]) -> TokenStream {
    data.iter()
        .zip(order)
        .map(|(d, order)| {
            let (name, len) = (&d.name, &d.length_ty);
            quote!(data(order = #order, name = #name, len = #len),)
        })
        .collect()
}

/// Where each group and each var-data sits among a block's members. Groups and var-data are
/// separate lists by the time they reach here, and the wire interleaves them in the order the
/// schema declared.
fn member_order(kinds: impl Iterator<Item = Option<bool>>) -> (Vec<usize>, Vec<usize>) {
    let (mut groups, mut data) = (Vec::new(), Vec::new());
    for (at, kind) in kinds.enumerate() {
        match kind {
            Some(true) => groups.push(at),
            Some(false) => data.push(at),
            None => {}
        }
    }
    (groups, data)
}

/// The schema's numbers for one field, hung on it so the macro can write its constants and
/// check its place against what the struct compiled to.
fn field_meta_attr(f: &PlacedField) -> TokenStream {
    let (offset, at) = match (f.field.offset, f.at()) {
        (Some(offset), _) => (Some(offset), None),
        (None, Some((at, _))) => (None, Some(at)),
        (None, None) => (None, None),
    };
    let value = f.view.opt_accessor.as_ref().map_or_else(
        || {
            f.view.value_read.map(|read| meta::Value {
                ty: syn::parse_str(&f.view.value_ty.to_string()).expect("a rendered type"),
                read,
            })
        },
        |opt| {
            Some(meta::Value {
                ty: syn::parse_str(&opt.ret_ty.to_string()).expect("a rendered type"),
                read: match (opt.wrapper, opt.get) {
                    (false, false) => meta::Read::Plain,
                    (false, true) => meta::Read::Get,
                    (true, false) => meta::Read::Inner,
                    (true, true) => meta::Read::InnerGet,
                },
            })
        },
    );
    let meta = meta::Meta {
        ident: None,
        ty: f.ty.clone(),
        skip: !f.setter_is_derivable(),
        padding: false,
        offset,
        at,
        since_version: f.field.since_version,
        semantic_type: f
            .field
            .semantic_type
            .as_deref()
            .and_then(|s| syn::parse_str(s).ok()),
        optional: f.view.opt_accessor.as_ref().map(|opt| meta::Optional {
            null: opt
                .null
                .as_ref()
                .map(|n| syn::parse_str(&n.to_string()).expect("a sentinel")),
            nan: opt.null.is_none(),
        }),
        required: f.view.required && value.is_some(),
        string: f.view.fixed_string,
        value,
        min: f.field.min_value.clone(),
        max: f.field.max_value.clone(),
        null: f.field.null_value.clone(),
        initial: f.field.initial_value.clone(),
    };
    quote!(#[sbe_gen(#meta)])
}

pub(crate) fn field_setters<'a>(
    layouts: impl Iterator<Item = &'a PlacedField> + 'a,
    store: impl Fn(usize) -> TokenStream + 'a,
) -> impl Iterator<Item = TokenStream> + 'a {
    layouts.filter_map(move |layout| {
        // only the fields that occupy bytes get a setter; `at` excludes constants too
        let (offset, _) = layout.at()?;
        let (name, resolved_type) = (&layout.name, &layout.resolved);
        let param_ty = resolved_type.param_ty();
        let encoded_expr = resolved_type.encode(quote!(value));
        let write = store(offset);
        let checks = &layout.setter_checks;
        Some(quote! {
            pub fn #name(&mut self, value: #param_ty) -> &mut Self {
                #checks
                let encoded = #encoded_expr;
                #write
                self
            }
        })
    })
}

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    parse::Parser,
    parse_macro_input,
    spanned::Spanned,
    Attribute, Expr, FnArg, Ident, ImplItem, ImplItemFn, ItemImpl, Meta, MetaNameValue,
    PatType, Receiver, ReturnType, Type,
};

#[proc_macro_attribute]
pub fn actor(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = match parse_actor_args(attr) {
        Ok(v) => v,
        Err(err) => return err.to_compile_error().into(),
    };

    let mut impl_block = parse_macro_input!(item as ItemImpl);

    let self_ty = impl_block.self_ty.clone();
    let actor_ident = match extract_type_ident(&self_ty) {
        Ok(id) => id,
        Err(err) => return err.to_compile_error().into(),
    };

    let enum_name = format_ident!("{}Message", actor_ident);

    let mut consumers = Vec::new();
    let mut init_method = None;
    let mut init_param_type = None;

    for impl_item in &mut impl_block.items {
        let ImplItem::Fn(method) = impl_item else {
            continue;
        };

        if let Some(idx) = method.attrs.iter().position(|a| a.path().is_ident("initialize")) {
            method.attrs.remove(idx);
            if init_method.is_some() {
                return syn::Error::new(
                    method.span(),
                    "only one #[initialize] method is allowed",
                )
                .to_compile_error()
                .into();
            }
            init_method = Some(method.sig.ident.clone());
            
            // Extract parameter type for initialize
            let mut params = method.sig.inputs.iter().filter_map(|arg| match arg {
                FnArg::Typed(p) => Some(p),
                FnArg::Receiver(_) => None,
            });
            
            if let Some(param) = params.next() {
                init_param_type = Some((*param.ty).clone());
            } else {
                init_param_type = Some(syn::parse_quote! { () });
            }
        }

        let Some(variant_ident) = take_message_consumer_attr(method) else {
            continue;
        };

        match parse_consumer_method(method, variant_ident.clone()) {
            Ok(info) => consumers.push(info),
            Err(err) => return err.to_compile_error().into(),
        }
    }

    if consumers.is_empty() {
        return syn::Error::new(
            impl_block.span(),
            "no #[message_consumer(...)] methods found inside #[actor] impl block",
        )
        .to_compile_error()
        .into();
    }

    let impl_generics = &impl_block.generics;
    let (_, ty_generics, where_clause) = impl_block.generics.split_for_impl();
    let state_type = &args.state_type;
    let handle_field = &args.handle_field;

    let variants = consumers.iter().map(|c| {
        let variant = &c.variant;
        if let Some(msg_ty) = &c.msg_type {
            quote! { #variant(#msg_ty) }
        } else {
            quote! { #variant }
        }
    });

    let match_arms = consumers.iter().map(|c| {
        let variant = &c.variant;
        let method = &c.method_name;
        if c.msg_type.is_some() {
            quote! {
                #enum_name::#variant(inner) => self.#method(inner, ctx, state)
            }
        } else {
            quote! {
                #enum_name::#variant => self.#method(ctx, state)
            }
        }
    });

    let from_impls = consumers.iter().filter_map(|c| {
        let variant = &c.variant;
        let msg_ty = c.msg_type.as_ref()?;
        Some(quote! {
            impl #impl_generics ::core::convert::From<#msg_ty> for #enum_name #ty_generics #where_clause {
                fn from(value: #msg_ty) -> Self {
                    #enum_name::#variant(value)
                }
            }
        })
    });

    let (init_impl, init_param_type) = match (init_method, init_param_type) {
        (Some(method), Some(param_ty)) => {
            if same_type(&param_ty, &syn::parse_quote! { () }) {
                (quote! { self.#method() }, param_ty)
            } else {
                (quote! { self.#method(param) }, param_ty)
            }
        },
        _ => (quote! { Default::default() }, syn::parse_quote! { () }),
    };

    let expanded = quote! {
        #impl_block

        pub enum #enum_name #impl_generics #where_clause {
            #(#variants),*
        }

        impl #impl_generics ::actrs::Actor for #self_ty #where_clause {
            type S = #state_type;
            type M = #enum_name #ty_generics;
            type I = #init_param_type;

            fn consume_message(
                &self,
                msg: Box<Self::M>,
                ctx: ::actrs::MsgCtx,
                state: Self::S,
            ) -> Self::S {
                match *msg {
                    #(#match_arms),*
                }
            }

            fn handle_init(&self, param: Self::I) -> Self::S {
                #init_impl
            }
        }

        impl #impl_generics ::actrs::StageAware for #self_ty #where_clause {
            fn set_handle(&mut self, handle: ::actrs::ActorHandle) {
                self.#handle_field = Some(handle);
            }
        }

        impl #impl_generics #self_ty #where_clause {
            pub fn send<M, TargetActorMessage>(
                &self,
                to: ::actrs::ActorRef,
                msg: M,
            ) -> ::core::result::Result<(), &'static str>
            where
                M: ::core::any::Any + Send + 'static,
                TargetActorMessage: ::actrs::Actor + ::core::any::Any + Send + 'static,
                <TargetActorMessage as ::actrs::Actor>::M: ::core::convert::From<M>,
            {
                match &self.#handle_field {
                    Some(handle) => handle.send(to, Box::new(<<TargetActorMessage as ::actrs::Actor>::M>::from(msg))),
                    None => Err("actor handle not initialized"),
                }
            }
        }

        #(#from_impls)*
    };

    expanded.into()
}

#[proc_macro_attribute]
pub fn initialize(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

struct ActorArgs {
    state_type: Type,
    handle_field: Ident,
}

fn parse_actor_args(attr: TokenStream) -> syn::Result<ActorArgs> {
    let parser = syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated;
    let args = parser.parse(attr)?;

    let mut state_type = None;
    let mut handle_field = None;

    for meta in args {
        if let Meta::NameValue(MetaNameValue { path, value, .. }) = meta {
            if path.is_ident("state") {
                if let Expr::Path(ref expr_path) = value {
                    state_type = Some(Type::Path(syn::TypePath {
                        qself: None,
                        path: expr_path.path.clone(),
                    }));
                } else {
                    return Err(syn::Error::new(
                        value.span(),
                        "expected a type path after state = ...",
                    ));
                }
            } else if path.is_ident("handle") {
                if let Expr::Path(ref expr_path) = value {
                    if let Some(seg) = expr_path.path.segments.last() {
                        handle_field = Some(seg.ident.clone());
                    } else {
                        return Err(syn::Error::new(
                            value.span(),
                            "expected a field name after handle = ...",
                        ));
                    }
                } else {
                    return Err(syn::Error::new(
                        value.span(),
                        "expected a field name after handle = ...",
                    ));
                }
            }
        }
    }

    Ok(ActorArgs {
        state_type: state_type.ok_or_else(|| {
            syn::Error::new(proc_macro2::Span::call_site(), "expected #[actor(state = T)]")
        })?,
        handle_field: handle_field.unwrap_or_else(|| format_ident!("_handle")),
    })
}

fn extract_type_ident(self_ty: &Type) -> syn::Result<Ident> {
    match self_ty {
        Type::Path(tp) => tp
            .path
            .segments
            .last()
            .map(|seg| seg.ident.clone())
            .ok_or_else(|| syn::Error::new(tp.span(), "could not determine actor type name")),
        _ => Err(syn::Error::new(
            self_ty.span(),
            "#[actor] currently supports only concrete path types like MyActor<T>",
        )),
    }
}

fn take_message_consumer_attr(method: &mut ImplItemFn) -> Option<Ident> {
    let idx = method
        .attrs
        .iter()
        .position(|a| a.path().is_ident("message_consumer"))?;

    let attr = method.attrs.remove(idx);
    parse_message_consumer_attr(&attr).ok()
}

fn parse_message_consumer_attr(attr: &Attribute) -> syn::Result<Ident> {
    attr.parse_args::<Ident>()
}

struct ConsumerMethod {
    variant: Ident,
    method_name: Ident,
    msg_type: Option<Type>,
}

fn parse_consumer_method(method: &ImplItemFn, variant: Ident) -> syn::Result<ConsumerMethod> {
    ensure_receiver_is_shared(&method.sig.inputs, method)?;
    ensure_return_type_present(method)?;

    let non_receiver: Vec<&PatType> = method
        .sig
        .inputs
        .iter()
        .filter_map(|arg| match arg {
            FnArg::Typed(p) => Some(p),
            FnArg::Receiver(_) => None,
        })
        .collect();

    let (msg_type, ctx_type, state_pat, state_type) = match non_receiver.as_slice() {
        [msg, ctx, state] => (
            Some((*msg.ty).clone()),
            (*ctx.ty).clone(),
            &state.pat,
            (*state.ty).clone(),
        ),
        [ctx, state] => (None, (*ctx.ty).clone(), &state.pat, (*state.ty).clone()),
        _ => {
            return Err(syn::Error::new(
                method.sig.span(),
                "message consumer must have one of these signatures:\n  fn handler(&self, msg: Msg, ctx: ::actrs::MsgCtx, state: T) -> T\n  fn handler(&self, ctx: ::actrs::MsgCtx, state: T) -> T",
            ))
        }
    };

    if let syn::Pat::Ident(syn::PatIdent { mutability: Some(m), .. }) = &**state_pat {
        return Err(syn::Error::new(
            m.span(),
            "the state parameter must not be mutable (The state should never be mutable)",
        ));
    }

    if let syn::Type::Reference(r) = &state_type {
        return Err(syn::Error::new(
            r.span(),
            "the state parameter must be passed by value, not by reference",
        ));
    }

    let ret_ty = match &method.sig.output {
        ReturnType::Type(_, ty) => ty.as_ref(),
        ReturnType::Default => {
            return Err(syn::Error::new(
                method.sig.output.span(),
                "message consumer must return state type T",
            ))
        }
    };

    if !same_type(ret_ty, &state_type) {
        return Err(syn::Error::new(
            ret_ty.span(),
            "return type must be the same as the state parameter type",
        ));
    }

    if !is_msg_ctx_type(&ctx_type) {
        return Err(syn::Error::new(
            ctx_type.span(),
            "second-to-last parameter must have type MsgCtx",
        ));
    }

    Ok(ConsumerMethod {
        variant,
        method_name: method.sig.ident.clone(),
        msg_type,
    })
}

fn ensure_receiver_is_shared(
    inputs: &syn::punctuated::Punctuated<FnArg, syn::Token![,]>,
    method: &ImplItemFn,
) -> syn::Result<()> {
    let first = inputs.first().ok_or_else(|| {
        syn::Error::new(
            method.sig.span(),
            "message consumer must have receiver &self as first parameter",
        )
    })?;

    match first {
        FnArg::Receiver(Receiver {
                            reference: Some(_),
                            mutability: None,
                            ..
                        }) => Ok(()),
        FnArg::Receiver(Receiver {
                            reference: Some(_),
                            mutability: Some(_),
                            ..
                        }) => Err(syn::Error::new(
            first.span(),
            "message consumer must use &self, not &mut self",
        )),
        FnArg::Receiver(_) => Err(syn::Error::new(
            first.span(),
            "message consumer must use &self",
        )),
        FnArg::Typed(_) => Err(syn::Error::new(
            first.span(),
            "message consumer must have &self as first parameter",
        )),
    }
}

fn ensure_return_type_present(method: &ImplItemFn) -> syn::Result<()> {
    match method.sig.output {
        ReturnType::Default => Err(syn::Error::new(
            method.sig.span(),
            "message consumer must return state type T",
        )),
        ReturnType::Type(_, _) => Ok(()),
    }
}

fn same_type(a: &Type, b: &Type) -> bool {
    quote!(#a).to_string() == quote!(#b).to_string()
}

fn is_msg_ctx_type(ty: &Type) -> bool {
    let Type::Path(tp) = ty else {
        return false;
    };

    // Accept both "MsgCtx" and "::actrs::MsgCtx" and "actrs::MsgCtx"
    if let Some(last_seg) = tp.path.segments.last() {
        if last_seg.ident == "MsgCtx" {
            return true;
        }
    }

    false
}

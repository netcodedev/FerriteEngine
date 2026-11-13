//! `#[derive(Script)]`: makes a struct a game script. Its public fields are
//! the script's parameters: saved in scenes and shown in the editor's
//! inspector. Other fields are the script's own state, set by `Default`.
//! A public field marked `#[script(skip)]` is not a parameter.
//!
//! The struct needs `Default` (parameters start at their defaults) and
//! `Behaviour` (what it does); parameters need `Clone` and serde.

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{parse_macro_input, Data, DeriveInput, Fields, Visibility};

#[proc_macro_derive(Script, attributes(script))]
pub fn derive_script(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand(&input) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn expand(input: &DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let name = &input.ident;
    let vis = &input.vis;
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "a script cannot have generic parameters",
        ));
    }
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(name, "only a struct can be a script"));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(
            name,
            "a script needs named fields (its parameters are its public fields)",
        ));
    };

    let mut params = Vec::new();
    for field in &fields.named {
        if matches!(field.vis, Visibility::Inherited) || is_skipped(field)? {
            continue;
        }
        params.push(field);
    }
    let param_names: Vec<_> = params.iter().map(|f| f.ident.clone().unwrap()).collect();
    let param_types: Vec<_> = params.iter().map(|f| f.ty.clone()).collect();
    let param_docs: Vec<Vec<_>> = params
        .iter()
        .map(|f| f.attrs.iter().filter(|a| a.path().is_ident("doc")).collect())
        .collect();
    let param_strings: Vec<String> = param_names.iter().map(|n| n.to_string()).collect();
    let data_name = format_ident!("{}ScriptData", name);
    let script_name = name.to_string();

    Ok(quote! {
        #[doc = concat!("The parameters of the script `", #script_name, "`.")]
        #[derive(::ferrite::serde::Serialize, ::ferrite::serde::Deserialize)]
        #[serde(crate = "::ferrite::serde", default)]
        #vis struct #data_name {
            #( #(#param_docs)* pub #param_names: #param_types, )*
        }

        impl ::core::default::Default for #data_name {
            fn default() -> Self {
                let defaults = <#name as ::core::default::Default>::default();
                Self { #( #param_names: defaults.#param_names, )* }
            }
        }

        impl ::ferrite::core::script::Script for #name {
            const NAME: &'static str = #script_name;
            const PARAMETERS: &'static [&'static str] = &[ #( #param_strings ),* ];
            type Data = #data_name;

            fn data(&self) -> #data_name {
                #data_name { #( #param_names: ::core::clone::Clone::clone(&self.#param_names), )* }
            }

            #[allow(unused_variables)]
            fn apply(&mut self, data: #data_name) {
                #( self.#param_names = data.#param_names; )*
            }
        }
    })
}

fn is_skipped(field: &syn::Field) -> syn::Result<bool> {
    let mut skipped = false;
    for attr in field.attrs.iter().filter(|a| a.path().is_ident("script")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("skip") {
                skipped = true;
                Ok(())
            } else {
                Err(meta.error("unknown script option; expected `skip`"))
            }
        })?;
    }
    Ok(skipped)
}

use proc_macro2::Span;
use proc_macro2::{self, TokenStream};
use quote::ToTokens;
use quote::quote;
use syn::Field;
use syn::FieldsNamed;
use syn::FieldsUnnamed;
use syn::Ident;
use syn::Token;
use syn::Variant;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Data, DataStruct, DeriveInput, Fields, Type, parse_macro_input};
use syn::{DataEnum, Index};

fn is_bool(ty: &Type) -> bool {
    matches!(ty, Type::Path(type_path) if type_path.clone().into_token_stream().to_string() == "bool")
}

fn handle_named_struct(fields_named: &FieldsNamed) -> TokenStream {
    fn unwrap_entry(f: &Field) -> (Ident, String) {
        let name = f.ident.as_ref().unwrap().to_token_stream();
        let name_str = name.to_string();
        (
            Ident::new(&format!("_f_{}", name_str), name.span()),
            name_str,
        )
    }

    let bool_fields: Vec<_> = fields_named
        .named
        .iter()
        .filter(|f| is_bool(&f.ty))
        .collect();

    let bool_handler = if !bool_fields.is_empty() {
        let entries = bool_fields.iter().map(|f| {
            let (name, name_str) = unwrap_entry(f);
            (quote! { #name_str => *#name = false }, name_str)
        });

        let matches = entries.clone().map(|f| f.0);
        let names = entries.map(|f| f.1);

        quote! {
            if field_tok.0 == crate::cmdline::CmdlineTokenData::Not {
                match lexer.next_tok()?.unwrap_ident()? {
                    #(#matches,)*
                    _ => {
                        return Err(field_tok.make_error(
                            crate::cmdline::CmdlineErrorCode::UnknownFlagField(&[#(#names,)*])
                        ));
                    }
                }
            }
        }
    } else {
        quote! {}
    };

    let main_handler = fields_named.named.iter().map(|f| {
        let (name, name_str) = unwrap_entry(f);

        (
            if is_bool(&f.ty) {
                quote! {
                    #name_str => {
                        if lexer.peek().0 != crate::cmdline::CmdlineTokenData::Colon {
                            *#name = true;
                        } else {
                            lexer.next_tok()?;
                            #name.parse(lexer)?;
                        }
                    }
                }
            } else {
                quote! {
                    #name_str => {
                        lexer.expect(crate::cmdline::CmdlineTokenData::Colon)?;
                        #name.parse(lexer)?;
                    }
                }
            },
            name_str,
        )
    });

    let matches = main_handler.clone().map(|f| f.0);
    let fields = main_handler.map(|f| f.1);

    quote! {
        lexer.expect(crate::cmdline::CmdlineTokenData::OpenBrace)?;
        lexer.parse_block(
            crate::cmdline::CmdlineTokenData::ClosedBrace,
            crate::cmdline::CmdlineTokenData::Comma,
            |lexer| {
                let field_tok = lexer.next_tok()?;

                #bool_handler

                match field_tok.unwrap_ident()? {
                    #(#matches,)*
                    _ => {
                        return Err(field_tok.make_error(
                            crate::cmdline::CmdlineErrorCode::UnknownField(&[#(#fields,)*])
                        ));
                    }
                }

                Ok(())
            },
        )?;

        Ok(())
    }
}

fn handle_unnamed_struct(fields: &FieldsUnnamed) -> TokenStream {
    let parse_entries = fields.unnamed.iter().enumerate().map(|(index, _)| {
        let tok = Ident::new(&format!("_f_{}", index), Span::mixed_site());

        quote! {
            if lexer.peek().0 == crate::cmdline::CmdlineTokenData::Identifier("_") {
                lexer.next_tok()?;
            } else {
                #tok.parse(lexer)?;
            }

            {
                let tok = lexer.peek();
                if tok.0 == crate::cmdline::CmdlineTokenData::Comma {
                    lexer.next_tok()?;
                } else {
                    lexer.expect(crate::cmdline::CmdlineTokenData::ClosedParen)?;
                    return Ok(());
                }
            }
        }
    });

    quote! {
        lexer.expect(crate::cmdline::CmdlineTokenData::OpenParen)?;

        if lexer.peek().0 == crate::cmdline::CmdlineTokenData::ClosedParen {
            lexer.next_tok()?;
            return Ok(());
        }

        #(#parse_entries)*

        return Ok(());
    }
}

fn handle_fields(fields: &Fields, allow_unit: bool) -> TokenStream {
    match fields {
        Fields::Named(fields) => handle_named_struct(fields),
        Fields::Unnamed(fields) => handle_unnamed_struct(fields),
        Fields::Unit => {
            if allow_unit {
                quote! {}
            } else {
                todo!()
            }
        }
    }
}

fn handle_enum(variants: &Punctuated<Variant, Token![,]>) -> TokenStream {
    // figure out the variant

    let entries: Vec<_> = variants
        .iter()
        .map(|f| {
            let enum_name_ident = &f.ident;
            let match_name = f.ident.to_token_stream().to_string().to_lowercase();

            let init = match &f.fields {
                Fields::Named(fields_named) => {
                    let initializers = fields_named.named.iter().map(|f| {
                        let name = f.ident.as_ref().unwrap();
                        let init_ident = Ident::new(&format!("_i_{}", name), name.span());
                        let mangled = Ident::new(&format!("_f_{}", name), name.span());
                        let ty = &f.ty;
                        let init = f
                            .attrs
                            .iter().find(|f| f.path.to_token_stream().to_string() == "default_value")
                            .map(|f| f.tokens.clone())
                            .unwrap_or(quote! { std::default::Default::default() });

                        quote! {
                            let mut #init_ident: #ty = #init;
                            let #mangled = &mut #init_ident;
                        }
                    });

                    quote! { #(#initializers;)* }
                }
                Fields::Unnamed(_fields_unnamed) => todo!(),
                Fields::Unit => quote! {},
            };

            let parse_body = handle_fields(&f.fields, true);

            let build = match &f.fields {
                Fields::Named(fields_named) => {
                    let initializers = fields_named.named.iter().map(|f| {
                        let name = f.ident.as_ref().unwrap();
                        let mangled = Ident::new(&format!("_f_{}", name), name.span());
                        quote! { #name: #mangled }
                    });

                    quote! { Self::#enum_name_ident { #(#initializers,)* } }
                }
                Fields::Unnamed(_fields_unnamed) => todo!(),
                Fields::Unit => quote! { Self::#enum_name_ident },
            };

            (
                quote! {
                    #match_name => {
                        #init

                        #parse_body

                        #build
                    }
                },
                match_name,
            )
        })
        .collect();

    let handlers = entries.iter().map(|f| &f.0);
    let names = entries.iter().map(|f| &f.1);

    quote! {
        let id_tok = lexer.next_tok()?;
        *self = match id_tok.unwrap_ident()? {
            #(#handlers)*
            _ => return Err(id_tok.make_error(
                crate::cmdline::CmdlineErrorCode::UnknownEnumerator(&[#(#names,)*])
            ))
        };

        Ok(())
    }
}

fn handle_struct(fields: &Fields) -> TokenStream {
    let unwrapper = match fields {
        Fields::Named(fields) => {
            let entries = fields.named.iter().map(|f| {
                let name = f.ident.as_ref().unwrap();
                let mangled = Ident::new(&format!("_f_{}", name), name.span());

                quote! { let #mangled = &mut self.#name }
            });

            quote! { #(#entries;)* }
        }
        Fields::Unnamed(fields) => {
            let entries = fields.unnamed.iter().enumerate().map(|(index, _)| {
                let tok = Ident::new(&format!("_f_{}", index), Span::mixed_site());
                let index = Index::from(index);
                quote! {
                    let #tok = &mut self.#index
                }
            });

            quote! { #(#entries;)* }
        }
        Fields::Unit => todo!(),
    };

    let inner = handle_fields(fields, false);

    quote! {
        #unwrapper
        #inner
    }
}

#[proc_macro_derive(CmdlineParsable, attributes(default_value))]
pub fn derive(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input);

    let DeriveInput { ident, data, .. } = input;

    let body = match data {
        Data::Struct(DataStruct { fields, .. }) => handle_struct(&fields),
        Data::Enum(DataEnum { variants, .. }) => handle_enum(&variants),
        _ => return quote! { compile_error!("unsupported data type") }.into(),
    };

    quote! {
        impl CmdlineParsable for #ident {
            fn parse<'a>(&mut self, lexer: &mut crate::cmdline::CmdlineLexer<'a>) -> Result<(), crate::cmdline::CmdlineParseError<'a>> {
                #body
            }
        }
    }.into()
}

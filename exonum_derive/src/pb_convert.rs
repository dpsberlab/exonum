// Copyright 2018 The Exonum Team
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use proc_macro::TokenStream;
use proc_macro2::{Ident, Span};
use syn::{Attribute, Data, DeriveInput, Lit, Path};

use super::PB_CONVERT_ATTRIBUTE;

fn get_protobuf_struct_path(attrs: &[Attribute]) -> Path {
    let map_attrs = super::get_exonum_attributes(attrs);
    let struct_path = map_attrs.into_iter().find_map(|nv| {
        if nv.ident == PB_CONVERT_ATTRIBUTE {
            match nv.lit {
                Lit::Str(path) => Some(path.parse::<Path>().unwrap()),
                _ => None,
            }
        } else {
            None
        }
    });

    struct_path.unwrap_or_else(|| panic!("{} attribute is not set properly.", PB_CONVERT_ATTRIBUTE))
}

fn get_field_names(input: &DeriveInput) -> Vec<Ident> {
    let data = match &input.data {
        Data::Struct(x) => x,
        _ => panic!("Protobuf convert can be derived for structs only."),
    };
    data.fields
        .iter()
        .map(|f| f.ident.clone().unwrap())
        .collect()
}

fn implement_protobuf_convert_from_pb(field_names: &[Ident]) -> impl quote::ToTokens {
    let getters = field_names
        .iter()
        .map(|i| Ident::new(&format!("get_{}", i), Span::call_site()));
    let our_struct_names = field_names.to_vec();

    quote! {
        fn from_pb(pb: Self::ProtoStruct) -> std::result::Result<Self, ()> {
          Ok(Self {
           #( #our_struct_names: ProtobufConvert::from_pb(pb.#getters().to_owned())?, )*
          })
        }
    }
}

fn implement_protobuf_convert_to_pb(field_names: &[Ident]) -> impl quote::ToTokens {
    let setters = field_names
        .iter()
        .map(|i| Ident::new(&format!("set_{}", i), Span::call_site()));
    let our_struct_names = field_names.to_vec();

    quote! {
        fn to_pb(&self) -> Self::ProtoStruct {
            let mut msg = Self::ProtoStruct::new();
            #( msg.#setters(ProtobufConvert::to_pb(&self.#our_struct_names).into()); )*
            msg
        }
    }
}

fn implement_protobuf_convert_trait(
    name: &Ident,
    pb_name: &Path,
    field_names: &[Ident],
) -> impl quote::ToTokens {
    let to_pb_fn = implement_protobuf_convert_to_pb(field_names);
    let from_pb_fn = implement_protobuf_convert_from_pb(field_names);

    quote! {
        impl ProtobufConvert for #name {
            type ProtoStruct = #pb_name;

            #to_pb_fn
            #from_pb_fn
        }
    }
}

fn implement_binary_form(name: &Ident, cr: &quote::ToTokens) -> impl quote::ToTokens {
    quote! {
        impl #cr::messages::BinaryForm for #name {

            fn encode(&self) -> std::result::Result<Vec<u8>, _EncodingError> {
                self.to_pb().write_to_bytes().map_err(|e| _EncodingError::Other(Box::new(e)))
            }

            fn decode(buffer: &[u8]) -> std::result::Result<Self, _EncodingError> {
                let mut pb = <Self as ProtobufConvert>::ProtoStruct::new();
                pb.merge_from_bytes(buffer).map_err(|e| _EncodingError::Other(Box::new(e)))?;
                Self::from_pb(pb).map_err(|_| "Conversion from protobuf error.".into())
            }
        }
    }
}

fn implement_storage_traits(name: &Ident, cr: &quote::ToTokens) -> impl quote::ToTokens {
    quote! {
        impl #cr::crypto::CryptoHash for #name {
            fn hash(&self) -> #cr::crypto::Hash {
                let v = self.to_pb().write_to_bytes().unwrap();
                #cr::crypto::hash(&v)
            }
        }

        // This trait assumes that we work with trusted data so we can unwrap here.
        impl #cr::storage::StorageValue for #name {
            fn into_bytes(self) -> Vec<u8> {
                self.to_pb().write_to_bytes().expect(&format!(
                    "Failed to serialize in StorageValue for {}",
                    stringify!(#name)
                ))
            }

            fn from_bytes(value: std::borrow::Cow<[u8]>) -> Self {
                let mut block = <Self as ProtobufConvert>::ProtoStruct::new();
                block.merge_from_bytes(value.as_ref()).unwrap();
                ProtobufConvert::from_pb(block).expect(&format!(
                    "Failed to deserialize in StorageValue for {}",
                    stringify!(#name)
                ))
            }
        }
    }
}

pub fn implement_protobuf_convert(input: TokenStream) -> TokenStream {
    let input: DeriveInput = syn::parse(input).unwrap();

    let name = input.ident.clone();
    let proto_struct_name = get_protobuf_struct_path(&input.attrs);
    let cr = super::get_exonum_types_prefix(&input.attrs);

    let mod_name = Ident::new(&format!("pb_convert_impl_{}", name), Span::call_site());

    let field_names = get_field_names(&input);
    let protobuf_convert =
        implement_protobuf_convert_trait(&name, &proto_struct_name, &field_names);
    let binary_form = implement_binary_form(&name, &cr);
    let storage_traits = implement_storage_traits(&name, &cr);

    let expanded = quote! {
        mod #mod_name {
            extern crate protobuf as _protobuf_crate;

            use super::*;

            use #cr::encoding::protobuf::ProtobufConvert;
            use #cr::encoding::Error as _EncodingError;
            use self::_protobuf_crate::Message as _ProtobufMessage;

            #protobuf_convert
            #binary_form
            #storage_traits
        }
    };

    expanded.into()
}

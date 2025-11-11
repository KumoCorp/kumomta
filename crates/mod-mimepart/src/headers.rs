use crate::PartRef;
use config::{SerdeWrappedValue, any_err};
use mailparsing::{
    AddressList, Header, HeaderMap, MailParsingError, Mailbox, MailboxList, MessageID,
    MimeParameters,
};
use mlua::{
    IntoLua, Lua, MetaMethod, MultiValue, UserData, UserDataFields, UserDataMethods, Value,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone)]
pub struct HeaderMapRef(PartRef);

impl HeaderMapRef {
    pub fn new(part: PartRef) -> Self {
        Self(part)
    }
}

struct HeaderIterInner {
    part: PartRef,
    idx: usize,
    name: Option<String>,
}

impl UserData for HeaderMapRef {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::ToString, move |lua, this, ()| {
            let part = this.0.resolve().map_err(any_err)?;
            let headers = part.headers();
            let mut out = vec![];
            for hdr in headers.iter() {
                hdr.write_header(&mut out).map_err(any_err)?;
            }
            lua.create_string(&out)
        });

        methods.add_method("append", |_lua, this, (name, value): (String, String)| {
            this.0
                .mutate(|part| {
                    part.headers_mut().append_header(&name, value);
                    Ok(())
                })
                .map_err(any_err)
        });

        methods.add_method("prepend", |_lua, this, (name, value): (String, String)| {
            this.0
                .mutate(|part| {
                    part.headers_mut().prepend(&name, value);
                    Ok(())
                })
                .map_err(any_err)
        });

        methods.add_method("remove_all_named", |_lua, this, name: String| {
            this.0
                .mutate(|part| {
                    part.headers_mut().remove_all_named(&name);
                    Ok(())
                })
                .map_err(any_err)
        });

        methods.add_method("get_first_named", |_lua, this, name: String| {
            let part = this.0.resolve().map_err(any_err)?;
            Ok(part
                .headers()
                .get_first(&name)
                .map(|hdr| HeaderWrap(hdr.to_owned())))
        });

        methods.add_method("iter", |lua, this, name: Option<String>| {
            let mut iter = HeaderIterInner {
                part: this.0.clone(),
                idx: 0,
                name,
            };

            let iter_func =
                lua.create_function_mut(move |lua: &Lua, (_state, _idx): (Value, Value)| {
                    let part = iter.part.resolve().map_err(any_err)?;
                    let headers = part.headers();
                    let mut result = vec![];

                    while let Some(hdr) = headers.get(iter.idx) {
                        iter.idx += 1;

                        let matched = iter
                            .name
                            .as_ref()
                            .map(|name| hdr.get_name().eq_ignore_ascii_case(name))
                            .unwrap_or(true);

                        if matched {
                            result.push(HeaderWrap(hdr.to_owned()).into_lua(lua)?);
                            break;
                        }
                    }

                    Ok(MultiValue::from_vec(result))
                })?;

            Ok(iter_func)
        });

        fn getter<T, GET>(
            get: GET,
        ) -> impl Fn(&Lua, &HeaderMapRef, ()) -> mlua::Result<Option<SerdeWrappedValue<T>>>
        where
            T: serde::de::DeserializeOwned,
            GET: Fn(&HeaderMap) -> Result<Option<T>, MailParsingError>,
        {
            move |_lua, this, ()| {
                let part = this.0.resolve().map_err(any_err)?;
                let value = (get)(part.headers())
                    .map_err(any_err)?
                    .map(SerdeWrappedValue);
                Ok(value)
            }
        }

        fn setter_unstructured<APPLY>(
            apply: APPLY,
        ) -> impl Fn(&Lua, &HeaderMapRef, String) -> mlua::Result<()>
        where
            APPLY: Fn(&mut HeaderMap, &str) -> Result<(), MailParsingError>,
        {
            move |_lua, this, value: String| {
                this.0
                    .mutate(|part| Ok(apply(part.headers_mut(), &value)?))
                    .map_err(any_err)
            }
        }

        fn getter_unstructured<GET>(
            get: GET,
        ) -> impl Fn(&Lua, &HeaderMapRef, ()) -> mlua::Result<Option<String>>
        where
            GET: Fn(&HeaderMap) -> Result<Option<String>, MailParsingError>,
        {
            move |_lua, this, ()| {
                let part = this.0.resolve().map_err(any_err)?;
                let value = (get)(part.headers()).map_err(any_err)?;
                Ok(value)
            }
        }

        /// Helper to ensure that we picked a reasonably typed accessor
        fn assert_type<T>(
            value: Result<Option<T>, MailParsingError>,
        ) -> Result<Option<T>, MailParsingError> {
            value
        }

        macro_rules! accessor {
            ($get_name:literal, $getter:ident, $set_name:literal, $setter:ident, $ty:path) => {
                methods.add_method(
                    $get_name,
                    getter(|headers| assert_type::<$ty>(headers.$getter())),
                );
                methods.add_method($set_name, |lua, this, value: mlua::Value| match value {
                    Value::String(s) => {
                        let s = s.to_str()?;
                        this.0
                            .mutate(|part| {
                                part.headers_mut().$setter(&*s)?;
                                Ok(())
                            })
                            .map_err(any_err)
                    }
                    value => {
                        let typed = config::from_lua_value::<$ty>(lua, value)?;
                        this.0
                            .mutate(|part| {
                                part.headers_mut().$setter(typed)?;
                                Ok(())
                            })
                            .map_err(any_err)
                    }
                });
            };

            ($get_name:literal, $getter:ident, $set_name:literal, $setter:ident, $ty:path, $via_ty:path) => {
                methods.add_method(
                    $get_name,
                    getter(|headers| headers.$getter().map(|opt| opt.map(<$ty>::from))),
                );
                methods.add_method($set_name, |lua, this, value: mlua::Value| match value {
                    Value::String(s) => {
                        let s = s.to_str()?;
                        this.0
                            .mutate(|part| {
                                part.headers_mut().$setter(&*s)?;
                                Ok(())
                            })
                            .map_err(any_err)
                    }
                    value => {
                        let typed: $via_ty = config::from_lua_value::<$ty>(lua, value)?.into();
                        this.0
                            .mutate(|part| {
                                part.headers_mut().$setter(typed)?;
                                Ok(())
                            })
                            .map_err(any_err)
                    }
                });
            };
        }

        macro_rules! unstructured {
            ($get_name:literal, $getter:ident, $set_name:literal, $setter:ident) => {
                methods.add_method($get_name, getter_unstructured(|headers| headers.$getter()));
                methods.add_method(
                    $set_name,
                    setter_unstructured(|headers, value| headers.$setter(value)),
                );
            };
        }

        accessor!("from", from, "set_from", set_from, MailboxList);
        accessor!(
            "resent_from",
            resent_from,
            "set_resent_from",
            set_from,
            MailboxList
        );
        accessor!("to", to, "set_to", set_to, AddressList);
        accessor!(
            "reply_to",
            reply_to,
            "set_reply_to",
            set_reply_to,
            AddressList
        );
        accessor!("cc", cc, "set_cc", set_cc, AddressList);
        accessor!("bcc", bcc, "set_bcc", set_bcc, AddressList);
        accessor!(
            "resent_to",
            resent_to,
            "set_resent_to",
            set_resent_to,
            AddressList
        );
        accessor!(
            "resent_cc",
            resent_cc,
            "set_resent_cc",
            set_resent_cc,
            AddressList
        );
        accessor!(
            "resent_bcc",
            resent_bcc,
            "set_resent_bcc",
            set_resent_bcc,
            AddressList
        );

        // accessor!("date", date, "set_date", set_date, DateTime<FixedOffset>); FIXME: establish lua time type

        accessor!("sender", sender, "set_sender", set_sender, Mailbox);
        accessor!(
            "resent_sender",
            resent_sender,
            "set_resent_sender",
            set_resent_sender,
            Mailbox
        );

        accessor!(
            "message_id",
            message_id,
            "set_message_id",
            set_message_id,
            MessageID
        );
        accessor!(
            "content_id",
            content_id,
            "set_content_id",
            set_content_id,
            MessageID
        );
        accessor!(
            "references",
            references,
            "set_references",
            set_references,
            Vec<MessageID>
        );

        unstructured!("subject", subject, "set_subject", set_subject);
        unstructured!("comments", comments, "set_comments", set_comments);
        unstructured!(
            "mime_version",
            mime_version,
            "set_mime_version",
            set_mime_version
        );

        accessor!(
            "content_transfer_encoding",
            content_transfer_encoding,
            "set_content_transfer_encoding",
            set_content_transfer_encoding,
            MimeParams,
            MimeParameters
        );
        accessor!(
            "content_disposition",
            content_disposition,
            "set_content_disposition",
            set_content_disposition,
            MimeParams,
            MimeParameters
        );
        accessor!(
            "content_type",
            content_type,
            "set_content_type",
            set_content_type,
            MimeParams,
            MimeParameters
        );
    }
}

/// A fully-decoded representation of the underlying MimeParameters value,
/// to make it more convenient to inspect from lua
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MimeParams {
    pub value: String,
    pub parameters: BTreeMap<String, String>,
}

impl From<MimeParameters> for MimeParams {
    fn from(params: MimeParameters) -> MimeParams {
        let map = params.parameter_map();

        MimeParams {
            value: params.value,
            parameters: map,
        }
    }
}

impl From<MimeParams> for MimeParameters {
    fn from(params: MimeParams) -> MimeParameters {
        let mut result = MimeParameters::new(&params.value);
        for (name, value) in params.parameters {
            result.set(&name, &value);
        }
        result
    }
}

#[derive(Clone)]
pub struct HeaderWrap(Header<'static>);

fn get_mailbox_list(lua: &Lua, header: &Header) -> mlua::Result<mlua::Value> {
    SerdeWrappedValue(header.as_mailbox_list().map_err(any_err)?).to_lua_value(lua)
}
fn get_address_list(lua: &Lua, header: &Header) -> mlua::Result<mlua::Value> {
    SerdeWrappedValue(header.as_address_list().map_err(any_err)?).to_lua_value(lua)
}
fn get_mailbox(lua: &Lua, header: &Header) -> mlua::Result<mlua::Value> {
    SerdeWrappedValue(header.as_mailbox().map_err(any_err)?).to_lua_value(lua)
}
fn get_message_id(lua: &Lua, header: &Header) -> mlua::Result<mlua::Value> {
    SerdeWrappedValue(header.as_message_id().map_err(any_err)?).to_lua_value(lua)
}
fn get_content_id(lua: &Lua, header: &Header) -> mlua::Result<mlua::Value> {
    SerdeWrappedValue(header.as_content_id().map_err(any_err)?).to_lua_value(lua)
}
fn get_message_id_list(lua: &Lua, header: &Header) -> mlua::Result<mlua::Value> {
    SerdeWrappedValue(header.as_message_id_list().map_err(any_err)?).to_lua_value(lua)
}
fn get_unstructured(lua: &Lua, header: &Header) -> mlua::Result<mlua::Value> {
    SerdeWrappedValue(header.as_unstructured().map_err(any_err)?).to_lua_value(lua)
}
fn get_content_transfer_encoding(lua: &Lua, header: &Header) -> mlua::Result<mlua::Value> {
    let params: MimeParams = header
        .as_content_transfer_encoding()
        .map_err(any_err)?
        .into();
    SerdeWrappedValue(params).to_lua_value(lua)
}
fn get_content_disposition(lua: &Lua, header: &Header) -> mlua::Result<mlua::Value> {
    let params: MimeParams = header.as_content_disposition().map_err(any_err)?.into();
    SerdeWrappedValue(params).to_lua_value(lua)
}
fn get_content_type(lua: &Lua, header: &Header) -> mlua::Result<mlua::Value> {
    let params: MimeParams = header.as_content_type().map_err(any_err)?.into();
    SerdeWrappedValue(params).to_lua_value(lua)
}
fn get_authentication_results(lua: &Lua, header: &Header) -> mlua::Result<mlua::Value> {
    SerdeWrappedValue(header.as_authentication_results().map_err(any_err)?).to_lua_value(lua)
}

const NAME_GETTER: &[(&str, fn(&Lua, &Header) -> mlua::Result<mlua::Value>)] = &[
    ("From", get_mailbox_list),
    ("Reply-To", get_address_list),
    ("To", get_address_list),
    ("Cc", get_address_list),
    ("Bcc", get_address_list),
    ("Message-ID", get_message_id),
    ("Subject", get_unstructured),
    ("Mime-Version", get_unstructured),
    ("Content-Transfer-Encoding", get_content_transfer_encoding),
    ("Content-Type", get_content_type),
    ("Content-Disposition", get_content_disposition),
    ("Authentication-Results", get_authentication_results),
    ("Resent-From", get_mailbox_list),
    ("Resent-To", get_address_list),
    ("Resent-Cc", get_address_list),
    ("Resent-Bcc", get_address_list),
    ("Resent-Sender", get_mailbox),
    ("Sender", get_mailbox),
    ("Content-ID", get_content_id),
    ("References", get_message_id_list),
    ("Comments", get_unstructured),
];

impl UserData for HeaderWrap {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::ToString, |_lua, this, ()| {
            Ok(this.0.to_header_string())
        });
    }

    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("name", |lua, this| lua.create_string(this.0.get_name()));

        fields.add_field_method_get("value", |lua, this| {
            let name = this.0.get_name();
            for (candidate, getter) in NAME_GETTER {
                if candidate.eq_ignore_ascii_case(name) {
                    return (getter)(lua, &this.0);
                }
            }
            get_unstructured(lua, &this.0)
        });

        fields.add_field_method_get("raw_value", |lua, this| {
            lua.create_string(this.0.get_raw_value())
        });
        fields.add_field_method_get("unstructured", |_lua, this| {
            this.0.as_unstructured().map_err(any_err)
        });
        fields.add_field_method_get("mailbox_list", |_lua, this| {
            Ok(SerdeWrappedValue(
                this.0.as_mailbox_list().map_err(any_err)?,
            ))
        });
        fields.add_field_method_get("address_list", |_lua, this| {
            Ok(SerdeWrappedValue(
                this.0.as_address_list().map_err(any_err)?,
            ))
        });
        fields.add_field_method_get("message_id", |_lua, this| {
            Ok(SerdeWrappedValue(this.0.as_message_id().map_err(any_err)?))
        });
        fields.add_field_method_get("message_id_list", |_lua, this| {
            Ok(SerdeWrappedValue(
                this.0.as_message_id_list().map_err(any_err)?,
            ))
        });
        fields.add_field_method_get("mime_params", |_lua, this| {
            Ok(SerdeWrappedValue(MimeParams::from(
                this.0.as_content_transfer_encoding().map_err(any_err)?,
            )))
        });
        fields.add_field_method_get("authentication_results", |_lua, this| {
            Ok(SerdeWrappedValue(
                this.0.as_authentication_results().map_err(any_err)?,
            ))
        });
    }
}

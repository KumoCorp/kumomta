use config::any_err;
use mailparsing::{DecodedBody, MimePart, PartPointer};
use mlua::{MetaMethod, UserData, UserDataFields, UserDataMethods, UserDataRef};
use parking_lot::Mutex;
use std::sync::Arc;

#[derive(Clone)]
pub struct PartRef {
    root_part: Arc<Mutex<MimePart<'static>>>,
    ptr: PartPointer,
}

impl PartRef {
    pub fn new(part: MimePart<'static>) -> Self {
        Self {
            root_part: Arc::new(Mutex::new(part)),
            ptr: PartPointer::root(),
        }
    }

    pub fn resolve(&self) -> anyhow::Result<MimePart<'_>> {
        let root = self.root_part.lock();
        root.resolve_ptr(self.ptr.clone())
            .ok_or_else(|| anyhow::anyhow!("failed to resolve PartRef to MimePart"))
            .cloned()
    }

    pub fn mutate<F: FnOnce(&mut MimePart) -> anyhow::Result<R>, R>(
        &self,
        f: F,
    ) -> anyhow::Result<R> {
        let mut root = self.root_part.lock();
        let part = root.resolve_ptr_mut(self.ptr.clone());
        match part {
            Some(p) => (f)(p),
            None => anyhow::bail!("failed to resolve PartRef to MimePart"),
        }
    }

    pub fn make_ref(&self, ptr: PartPointer) -> Self {
        Self {
            root_part: self.root_part.clone(),
            ptr,
        }
    }

    pub fn get_simple_structure(&self) -> anyhow::Result<SimpleStructure> {
        let part = self.resolve()?;

        let parts = part.simplified_structure_pointers().map_err(any_err)?;

        let mut attachments = vec![];
        for ptr in parts.attachments {
            attachments.push(self.make_ref(ptr));
        }

        Ok(SimpleStructure {
            text_part: parts.text_part.map(|ptr| self.make_ref(ptr)),
            html_part: parts.html_part.map(|ptr| self.make_ref(ptr)),
            header_part: self.make_ref(parts.header_part),
            attachments,
        })
    }

    pub fn replace_body(
        &self,
        body: mlua::String,
        mut content_type: Option<String>,
    ) -> anyhow::Result<()> {
        self.mutate(|part| {
            if content_type.is_none() {
                if let Ok(Some(params)) = part.headers().content_type() {
                    content_type.replace(params.value);
                }
            }

            match body.to_str() {
                Ok(s) => {
                    part.replace_text_body(content_type.as_deref().unwrap_or("text/plain"), &s)?;
                }
                _ => {
                    part.replace_binary_body(
                        content_type
                            .as_deref()
                            .unwrap_or("application/octet-stream"),
                        &body.as_bytes(),
                    )?;
                }
            }
            Ok(())
        })
    }
}

impl UserData for PartRef {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method(
            "append_part",
            move |_lua, this, part: UserDataRef<PartRef>| {
                let part_to_append = part.resolve().map_err(any_err)?.to_owned();

                this.mutate(|this_part| {
                    this_part.child_parts_mut().push(part_to_append);
                    Ok(())
                })
                .map_err(any_err)?;

                Ok(())
            },
        );

        methods.add_method("get_simple_structure", move |lua, this, ()| {
            let s = this.get_simple_structure().map_err(any_err)?;

            let result = lua.create_table()?;
            result.set("text_part", s.text_part)?;
            result.set("html_part", s.html_part)?;
            result.set("header_part", s.header_part)?;

            let attachments = lua.create_table()?;
            for a in s.attachments {
                let a_part = a.resolve().map_err(any_err)?;
                let attach = lua.create_table()?;

                let mut file_name = format!("attachment{}", a.ptr.id_string());
                let mut inline = true;
                let mut content_id = None;
                let mut content_type = None;

                if let Ok(info) = a_part.rfc2045_info() {
                    if let Some(mut opts) = info.attachment_options {
                        if let Some(name) = opts.file_name.take() {
                            file_name = name;
                        }
                        inline = opts.inline;
                        content_id = opts.content_id;
                    }

                    if let Some(ct) = info.content_type {
                        content_type.replace(ct.value);
                    }
                }

                attach.set("file_name", file_name)?;
                attach.set("inline", inline)?;
                attach.set("content_id", content_id)?;
                match content_type {
                    Some(ct) => {
                        attach.set("content_type", ct)?;
                    }
                    None => {
                        attach.set("content_type", "application/octet-stream")?;
                    }
                }

                attach.set("part", a)?;
                attachments.push(attach)?;
            }

            result.set("attachments", attachments)?;

            Ok(result)
        });

        methods.add_meta_method(MetaMethod::ToString, move |_lua, this, ()| {
            let root = this.root_part.lock();
            Ok(root.to_message_string())
        });

        methods.add_method("rebuild", move |_lua, this, ()| {
            let root = this.root_part.lock();
            let part = root.rebuild().map_err(any_err)?;
            Ok(PartRef::new(part))
        });

        methods.add_method(
            "replace_body",
            move |_lua, this, (body, content_type): (mlua::String, Option<String>)| {
                this.replace_body(body, content_type).map_err(any_err)
            },
        );
    }

    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("body", |lua, this| {
            let part = this.resolve().map_err(any_err)?;
            let body = part.body().map_err(any_err)?;
            match body {
                DecodedBody::Text(s) => lua.create_string(s.as_str()),
                DecodedBody::Binary(b) => lua.create_string(b),
            }
        });
        fields.add_field_method_set("body", |_lua, this, body: mlua::String| {
            this.replace_body(body, None).map_err(any_err)
        });
        fields.add_field_method_get("headers", |_lua, this| {
            let _part = this.resolve().map_err(any_err)?;
            Ok(crate::headers::HeaderMapRef::new(this.clone()))
        });
        fields.add_field_method_get("id", |_lua, this| Ok(this.ptr.id_string()));
    }
}

pub struct SimpleStructure {
    pub text_part: Option<PartRef>,
    pub html_part: Option<PartRef>,
    pub header_part: PartRef,
    pub attachments: Vec<PartRef>,
}

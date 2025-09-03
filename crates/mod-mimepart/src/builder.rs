use crate::PartRef;
use config::{SerdeWrappedValue, any_err};
use mailparsing::{AttachmentOptions, MessageBuilder};
use mlua::{UserData, UserDataMethods, UserDataRef};
use parking_lot::Mutex;
use std::sync::Arc;

#[derive(Clone)]
pub struct Builder {
    builder: Arc<Mutex<Option<MessageBuilder<'static>>>>,
}

impl Builder {
    pub fn new() -> Self {
        Self {
            builder: Arc::new(Mutex::new(Some(MessageBuilder::new()))),
        }
    }

    pub fn mutate<F: FnOnce(&mut MessageBuilder) -> anyhow::Result<R>, R>(
        &self,
        f: F,
    ) -> anyhow::Result<R> {
        let mut builder = self.builder.lock();
        match builder.as_mut() {
            Some(builder) => (f)(builder),
            None => anyhow::bail!("builder already built!"),
        }
    }

    pub fn take(&self) -> anyhow::Result<MessageBuilder<'static>> {
        match self.builder.lock().take() {
            Some(builder) => Ok(builder),
            None => anyhow::bail!("builder already built!"),
        }
    }
}

impl UserData for Builder {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("set_stable_content", |_lua, this, stable: bool| {
            this.mutate(|builder| {
                builder.set_stable_content(stable);
                Ok(())
            })
            .map_err(any_err)
        });
        methods.add_method("text_plain", |_lua, this, text: String| {
            this.mutate(|builder| {
                builder.text_plain(&text);
                Ok(())
            })
            .map_err(any_err)
        });
        methods.add_method("text_html", |_lua, this, text: String| {
            this.mutate(|builder| {
                builder.text_html(&text);
                Ok(())
            })
            .map_err(any_err)
        });
        methods.add_method(
            "attach",
            |_lua,
             this,
             (content_type, data, opts): (
                String,
                mlua::String,
                Option<SerdeWrappedValue<AttachmentOptions>>,
            )| {
                this.mutate(|builder| {
                    builder.attach(&content_type, &data.as_bytes(), opts.as_deref())?;
                    Ok(())
                })
                .map_err(any_err)
            },
        );
        methods.add_method("attach_part", |_lua, this, part: UserDataRef<PartRef>| {
            let part = part.resolve().map_err(any_err)?.to_owned();
            this.mutate(|builder| {
                builder.attach_part(part);
                Ok(())
            })
            .map_err(any_err)
        });
        methods.add_method("build", |_lua, this, ()| {
            let part = this.take().map_err(any_err)?.build().map_err(any_err)?;
            Ok(PartRef::new(part))
        });
    }
}

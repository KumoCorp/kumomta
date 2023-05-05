use config::{any_err, get_or_create_sub_module};
use mlua::prelude::LuaUserData;
use mlua::{Lua, LuaSerdeExt, MetaMethod, UserDataMethods, Value};
use reqwest::header::HeaderMap;
use reqwest::{Client, ClientBuilder, RequestBuilder, Response, Url};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// Client ----

#[derive(Deserialize, Debug, Clone)]
struct ClientOptions {
    user_agent: Option<String>,
}

#[derive(Clone)]
struct ClientWrapper {
    client: Arc<Client>,
}

impl LuaUserData for ClientWrapper {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_method("get", |_, this, url: String| {
            let builder = this.client.get(url);
            Ok(RequestWrapper::new(builder))
        });
        methods.add_method("post", |_, this, url: String| {
            let builder = this.client.post(url);
            Ok(RequestWrapper::new(builder))
        });
        methods.add_method("put", |_, this, url: String| {
            let builder = this.client.put(url);
            Ok(RequestWrapper::new(builder))
        });
    }
}

// Request ----

#[derive(Clone)]
struct RequestWrapper {
    builder: Arc<Mutex<Option<RequestBuilder>>>,
}

impl RequestWrapper {
    fn new(builder: RequestBuilder) -> Self {
        Self {
            builder: Arc::new(Mutex::new(Some(builder))),
        }
    }

    fn apply<F>(&self, func: F) -> mlua::Result<()>
    where
        F: FnOnce(RequestBuilder) -> anyhow::Result<RequestBuilder>,
    {
        let b = self
            .builder
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| mlua::Error::external("broken request builder"))?;

        let b = (func)(b).map_err(any_err)?;

        self.builder.lock().unwrap().replace(b);
        Ok(())
    }

    async fn send(&self) -> mlua::Result<Response> {
        let b = self
            .builder
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| mlua::Error::external("broken request builder"))?;

        b.send().await.map_err(any_err)
    }
}

impl LuaUserData for RequestWrapper {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_method("header", |_, this, (key, value): (String, String)| {
            this.apply(|b| Ok(b.header(key, value)))?;
            Ok(this.clone())
        });

        methods.add_method("headers", |_, this, headers: HashMap<String, String>| {
            for (key, value) in headers {
                this.apply(|b| Ok(b.header(key, value)))?;
            }
            Ok(this.clone())
        });

        methods.add_method(
            "basic_auth",
            |_, this, (username, password): (String, Option<String>)| {
                this.apply(|b| Ok(b.basic_auth(username, password)))?;
                Ok(this.clone())
            },
        );

        methods.add_method("bearer_auth", |_, this, token: String| {
            this.apply(|b| Ok(b.bearer_auth(token)))?;
            Ok(this.clone())
        });

        methods.add_method("body", |_, this, body: String| {
            this.apply(|b| Ok(b.body(body)))?;
            Ok(this.clone())
        });

        methods.add_async_method("send", |_, this, _: ()| async move {
            let response = this.send().await?;
            Ok(ResponseWrapper {
                response: Arc::new(Mutex::new(Some(response))),
            })
        });
    }
}

// Response ----

#[derive(Clone)]
struct ResponseWrapper {
    response: Arc<Mutex<Option<Response>>>,
}

impl ResponseWrapper {
    fn with<F, T>(&self, func: F) -> mlua::Result<T>
    where
        F: FnOnce(&Response) -> anyhow::Result<T>,
    {
        let locked = self.response.lock().unwrap();
        let response = locked
            .as_ref()
            .ok_or_else(|| mlua::Error::external("broken response wrapper"))?;

        (func)(response).map_err(any_err)
    }

    async fn text(&self) -> mlua::Result<String> {
        let r = self
            .response
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| mlua::Error::external("broken response wrapper"))?;

        r.text().await.map_err(any_err)
    }

    async fn bytes<'lua>(&self, lua: &'lua Lua) -> mlua::Result<mlua::String<'lua>> {
        let r = self
            .response
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| mlua::Error::external("broken response wrapper"))?;

        let bytes = r.bytes().await.map_err(any_err)?;

        lua.create_string(bytes.as_ref())
    }
}

impl LuaUserData for ResponseWrapper {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_method("status_code", |_, this, _: ()| {
            this.with(|response| Ok(response.status().as_u16()))
        });
        methods.add_method("status_reason", |_, this, _: ()| {
            this.with(|response| Ok(response.status().canonical_reason()))
        });
        methods.add_method("status_is_informational", |_, this, _: ()| {
            this.with(|response| Ok(response.status().is_informational()))
        });
        methods.add_method("status_is_success", |_, this, _: ()| {
            this.with(|response| Ok(response.status().is_success()))
        });
        methods.add_method("status_is_redirection", |_, this, _: ()| {
            this.with(|response| Ok(response.status().is_redirection()))
        });
        methods.add_method("status_is_client_error", |_, this, _: ()| {
            this.with(|response| Ok(response.status().is_client_error()))
        });
        methods.add_method("status_is_server_error", |_, this, _: ()| {
            this.with(|response| Ok(response.status().is_server_error()))
        });
        methods.add_method("headers", |_, this, _: ()| {
            this.with(|response| Ok(HeaderMapWrapper(response.headers().clone())))
        });
        methods.add_method("content_length", |_, this, _: ()| {
            this.with(|response| Ok(response.content_length()))
        });

        methods.add_async_method("text", |_, this, _: ()| async move { this.text().await });

        methods.add_async_method(
            "bytes",
            |lua, this, _: ()| async move { this.bytes(lua).await },
        );
    }
}

// Headermap ---

#[derive(Clone)]
struct HeaderMapWrapper(HeaderMap);

impl LuaUserData for HeaderMapWrapper {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::Index, |lua, this, key: String| {
            if let Some(value) = this.0.get(&key) {
                let s = lua.create_string(value.as_bytes())?;
                return Ok(Value::String(s));
            }
            Ok(Value::Nil)
        });

        methods.add_meta_method(MetaMethod::Pairs, |lua, this, ()| {
            let stateless_iter =
                lua.create_function(|lua, (this, key): (HeaderMapWrapper, Option<String>)| {
                    let mut iter = this.0.iter();

                    let mut this_is_key = false;

                    if key.is_none() {
                        this_is_key = true;
                    }

                    while let Some((this_key, value)) = iter.next() {
                        if this_is_key {
                            let key = lua.create_string(this_key.as_str().as_bytes())?;
                            let value = lua.create_string(value.as_bytes())?;

                            return Ok(mlua::MultiValue::from_vec(vec![
                                Value::String(key),
                                Value::String(value),
                            ]));
                        }
                        if Some(this_key.as_str()) == key.as_deref() {
                            this_is_key = true;
                        }
                    }
                    return Ok(mlua::MultiValue::new());
                })?;
            Ok((stateless_iter, this.clone(), Value::Nil))
        });
    }
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let http_mod = get_or_create_sub_module(lua, "http")?;

    http_mod.set(
        "build_url",
        lua.create_function(|_lua, (url, params): (String, HashMap<String, String>)| {
            let url = Url::parse_with_params(&url, params.into_iter()).map_err(any_err)?;
            let url: String = url.into();
            Ok(url)
        })?,
    )?;

    http_mod.set(
        "build_client",
        lua.create_function(|lua, options: Value| {
            let options: ClientOptions = lua.from_value(options)?;
            let mut builder = ClientBuilder::new();

            if let Some(user_agent) = options.user_agent {
                builder = builder.user_agent(user_agent);
            }

            let client = builder.build().map_err(any_err)?;
            Ok(ClientWrapper {
                client: Arc::new(client),
            })
        })?,
    )?;

    Ok(())
}

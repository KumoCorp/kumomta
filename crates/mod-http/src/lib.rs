use config::{any_err, from_lua_value, get_or_create_sub_module};
use futures_util::StreamExt;
use mlua::prelude::LuaUserData;
use mlua::{Lua, LuaSerdeExt, MetaMethod, UserDataMethods, Value};
use reqwest::header::HeaderMap;
use reqwest::{Body, Client, ClientBuilder, RequestBuilder, Response, StatusCode, Url};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Mutex as TokioMutex;
use tokio_tungstenite::tungstenite::Message;

// Client ----

#[derive(Deserialize, Debug, Clone)]
struct ClientOptions {
    #[serde(default)]
    user_agent: Option<String>,
    #[serde(default)]
    connection_verbose: Option<bool>,
    #[serde(default, with = "duration_serde")]
    pool_idle_timeout: Option<Duration>,
    #[serde(default, with = "duration_serde")]
    timeout: Option<Duration>,
}

#[derive(Clone)]
struct ClientWrapper {
    client: Arc<Mutex<Option<Arc<Client>>>>,
}

impl ClientWrapper {
    fn get_client(&self) -> mlua::Result<Arc<Client>> {
        let inner = self.client.lock().unwrap();
        inner
            .as_ref()
            .map(Arc::clone)
            .ok_or_else(|| mlua::Error::external("client was closed"))
    }
}

impl LuaUserData for ClientWrapper {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("get", |_, this, url: String| {
            let builder = this.get_client()?.get(url);
            Ok(RequestWrapper::new(builder))
        });
        methods.add_method("post", |_, this, url: String| {
            let builder = this.get_client()?.post(url);
            Ok(RequestWrapper::new(builder))
        });
        methods.add_method("put", |_, this, url: String| {
            let builder = this.get_client()?.put(url);
            Ok(RequestWrapper::new(builder))
        });
        methods.add_method("close", |_, this, _: ()| {
            this.client.lock().unwrap().take();
            Ok(())
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

#[derive(Deserialize, Clone, Hash, PartialEq, Eq, Debug)]
pub struct FilePart {
    data: String,
    file_name: String,
}

impl LuaUserData for RequestWrapper {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
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

        methods.add_method("timeout", |_, this, duration: Value| {
            let duration = match duration {
                Value::Number(n) => std::time::Duration::from_secs_f64(n),
                Value::String(s) => {
                    let s = s.to_str()?;
                    humantime::parse_duration(&s).map_err(any_err)?
                }
                _ => {
                    return Err(mlua::Error::external("invalid timeout duration"));
                }
            };
            this.apply(|b| Ok(b.timeout(duration)))?;
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

        methods.add_method(
            "form_url_encoded",
            |_, this, params: HashMap<String, String>| {
                this.apply(|b| Ok(b.form(&params)))?;
                Ok(this.clone())
            },
        );

        methods.add_method(
            "form_multipart_data",
            |lua, this, params: HashMap<String, mlua::Value>| {
                // Generate a MIME body from the provided parameters
                use mail_builder::headers::text::Text;
                use mail_builder::headers::HeaderType;
                use mail_builder::mime::MimePart;
                use mailparse::MailHeaderMap;
                use std::borrow::Cow;

                let mut data = MimePart::new_multipart("multipart/form-data", vec![]);

                for (k, v) in params {
                    match v {
                        mlua::Value::String(s) => {
                            let part = if let Ok(s) = s.to_str() {
                                MimePart::new_text(Cow::Owned(s.to_string()))
                            } else {
                                MimePart::new_binary(
                                    "application/octet-stream",
                                    Cow::Owned(s.as_bytes().to_vec()),
                                )
                            };
                            data.add_part(part.header(
                                "Content-Disposition",
                                HeaderType::Text(Text::new(format!("form-data; name=\"{k}\""))),
                            ));
                        }
                        _ => {
                            let file: FilePart = lua.from_value(v.clone())?;

                            let part = MimePart::new_binary(
                                "application/octet-stream",
                                file.data.into_bytes(),
                            );
                            data.add_part(part.header(
                                "Content-Disposition",
                                HeaderType::Text(Text::new(format!(
                                    "form-data; name=\"{k}\"; filename=\"{}\"",
                                    file.file_name
                                ))),
                            ));
                        }
                    }
                }
                let builder = mail_builder::MessageBuilder::new();
                let builder = builder.body(data);
                let body = builder.write_to_vec().map_err(any_err)?;

                // Now, parse out the Content-Type header so that we can set that in
                // the request, and get the generated body with its generated boundary
                // string into a separate variable so that we can assign it as the body
                // of the HTTP request.

                let (headers, body_offset) = mailparse::parse_headers(&body).map_err(any_err)?;

                let content_type = headers
                    .get_first_value("Content-Type")
                    .ok_or_else(|| mlua::Error::external("missing Content-Type!?".to_string()))?;

                let body = &body[body_offset..];

                this.apply(|b| Ok(b.header("Content-Type", content_type).body(body.to_vec())))?;

                Ok(this.clone())
            },
        );

        methods.add_async_method("send", |_, this, _: ()| async move {
            let response = this.send().await?;
            let status = response.status();
            Ok(ResponseWrapper {
                status,
                response: Arc::new(Mutex::new(Some(response))),
            })
        });
    }
}

// Response ----

#[derive(Clone)]
struct ResponseWrapper {
    status: StatusCode,
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

    async fn bytes(&self, lua: &Lua) -> mlua::Result<mlua::String> {
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
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("status_code", |_, this, _: ()| Ok(this.status.as_u16()));
        methods.add_method("status_reason", |_, this, _: ()| {
            Ok(this.status.canonical_reason())
        });
        methods.add_method("status_is_informational", |_, this, _: ()| {
            Ok(this.status.is_informational())
        });
        methods.add_method("status_is_success", |_, this, _: ()| {
            Ok(this.status.is_success())
        });
        methods.add_method("status_is_redirection", |_, this, _: ()| {
            Ok(this.status.is_redirection())
        });
        methods.add_method("status_is_client_error", |_, this, _: ()| {
            Ok(this.status.is_client_error())
        });
        methods.add_method("status_is_server_error", |_, this, _: ()| {
            Ok(this.status.is_server_error())
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
            |lua, this, _: ()| async move { this.bytes(&lua).await },
        );
    }
}

// Headermap ---

#[derive(Clone, mlua::FromLua)]
struct HeaderMapWrapper(HeaderMap);

impl LuaUserData for HeaderMapWrapper {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
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

#[derive(Clone)]
struct WebSocketStream {
    stream: Arc<
        TokioMutex<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
        >,
    >,
}

impl LuaUserData for WebSocketStream {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_async_method("recv", |lua, this, ()| async move {
            let maybe_msg = {
                let mut stream = this.stream.lock().await;
                stream.next().await
            };
            let msg = match maybe_msg {
                Some(msg) => msg.map_err(any_err)?,
                None => return Ok(None),
            };
            Ok(match msg {
                Message::Text(s) => Some(lua.create_string(&s)?),
                Message::Close(_close_frame) => {
                    return Ok(None);
                }
                Message::Pong(s) | Message::Binary(s) => Some(lua.create_string(&s)?),
                Message::Ping(_) | Message::Frame(_) => {
                    unreachable!()
                }
            })
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
            let options: ClientOptions = from_lua_value(lua, options)?;
            let mut builder = ClientBuilder::new().timeout(
                options
                    .timeout
                    .unwrap_or_else(|| std::time::Duration::from_secs(60)),
            );

            if let Some(verbose) = options.connection_verbose {
                builder = builder.connection_verbose(verbose);
            }

            if let Some(idle) = options.pool_idle_timeout {
                builder = builder.pool_idle_timeout(idle);
            }

            if let Some(user_agent) = options.user_agent {
                builder = builder.user_agent(user_agent);
            }

            let client = builder.build().map_err(any_err)?;
            Ok(ClientWrapper {
                client: Arc::new(Mutex::new(Some(Arc::new(client)))),
            })
        })?,
    )?;

    http_mod.set(
        "connect_websocket",
        lua.create_async_function(|_, url: String| async move {
            let (stream, response) = tokio_tungstenite::connect_async(url)
                .await
                .map_err(any_err)?;
            let stream = WebSocketStream {
                stream: Arc::new(TokioMutex::new(stream)),
            };

            // Adapt the retured http::response into a reqwest::Response
            // so that we can use our existing ResponseWrapper type with it
            let status = response.status();
            let (parts, body) = response.into_parts();
            let body = Body::from(body.unwrap_or_else(|| vec![]));
            let response = tokio_tungstenite::tungstenite::http::Response::from_parts(parts, body);

            let response = ResponseWrapper {
                status,
                response: Arc::new(Mutex::new(Some(Response::from(response)))),
            };

            Ok((stream, response))
        })?,
    )?;

    Ok(())
}

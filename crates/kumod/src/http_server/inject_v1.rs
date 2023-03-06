use crate::http_server::auth::AuthKind;
use crate::http_server::AppError;
use crate::logging::{log_disposition, LogDisposition, RecordType};
use crate::mx::ResolvedAddress;
use crate::queue::QueueManager;
use anyhow::Context;
use axum::extract::Json;
use axum_client_ip::InsecureClientIp;
use config::{load_config, LuaConfig};
use mail_builder::headers::text::Text;
use mail_builder::headers::HeaderType;
use mail_builder::mime::MimePart;
use message::EnvelopeAddress;
use minijinja::{Environment, Template};
use ouroboros::self_referencing;
use rfc5321::Response;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use spool::SpoolId;
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::net::IpAddr;
use std::sync::Arc;

#[derive(Serialize, Deserialize, Debug)]
pub struct FromHeader {
    pub email: String,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Recipient {
    pub email: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub substitutions: HashMap<String, Value>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct InjectV1Request {
    pub envelope_sender: String,
    pub recipients: Vec<Recipient>,
    pub content: Content,
    #[serde(default)]
    pub substitutions: HashMap<String, Value>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct InjectV1Response {
    pub success_count: usize,
    pub fail_count: usize,
    pub failed_recipients: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
pub enum Content {
    Rfc822(String),
    Builder {
        #[serde(default)]
        text_body: Option<String>,
        #[serde(default)]
        html_body: Option<String>,
        #[serde(default)]
        attachments: Vec<Attachment>,
        #[serde(default)]
        headers: BTreeMap<String, String>,
        #[serde(default)]
        from: Option<FromHeader>,
        #[serde(default)]
        subject: Option<String>,
        #[serde(default)]
        reply_to: Option<FromHeader>,
    },
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
pub enum Header {
    Full(String),
    NameValue(String, String),
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Attachment {
    data: String,
    content_type: String,
    #[serde(default)]
    content_id: Option<String>,
    #[serde(default)]
    file_name: Option<String>,
    #[serde(default)]
    base64: bool,
}

#[self_referencing]
struct Compiled<'a> {
    env: Environment<'a>,
    #[borrows(env)]
    #[covariant]
    templates: Vec<Template<'this>>,
    inline: Vec<MimePart<'a>>,
    attached: Vec<MimePart<'a>>,
}

impl<'a> Compiled<'a> {
    pub fn expand_for_recip(
        &self,
        recip: &Recipient,
        global_subst: &HashMap<String, Value>,
        content: &Content,
    ) -> anyhow::Result<String> {
        let mut subst = serde_json::json!({});
        for (k, v) in global_subst {
            subst.as_object_mut().unwrap().insert(k.clone(), v.clone());
        }
        subst
            .as_object_mut()
            .unwrap()
            .insert("email".to_string(), recip.email.to_string().into());
        if let Some(name) = &recip.name {
            subst
                .as_object_mut()
                .unwrap()
                .insert("name".to_string(), name.to_string().into());
        }

        for (k, v) in &recip.substitutions {
            subst.as_object_mut().unwrap().insert(k.clone(), v.clone());
        }

        let mut id = 0;
        match content {
            Content::Rfc822(_) => Ok(self.borrow_templates()[id].render(&subst)?),
            Content::Builder {
                text_body,
                html_body,
                headers,
                from,
                reply_to,
                ..
            } => {
                let mut builder = mail_builder::MessageBuilder::new();

                let mut text = None;
                let mut html = None;

                if text_body.is_some() {
                    text.replace(MimePart::new_text(
                        self.borrow_templates()[id].render(&subst)?,
                    ));
                    id += 1;
                }

                if html_body.is_some() {
                    html.replace(MimePart::new_html(
                        self.borrow_templates()[id].render(&subst)?,
                    ));
                    id += 1;
                }

                builder = builder.to(mail_builder::headers::address::Address::new_address(
                    recip.name.as_ref(),
                    &recip.email,
                ));

                if let Some(from) = from {
                    builder = builder.to(mail_builder::headers::address::Address::new_address(
                        from.name.as_ref(),
                        &from.email,
                    ));
                }

                if let Some(reply_to) = reply_to {
                    builder =
                        builder.reply_to(mail_builder::headers::address::Address::new_address(
                            reply_to.name.as_ref(),
                            &reply_to.email,
                        ));
                }

                for (name, _value) in headers {
                    let expanded = self.borrow_templates()[id].render(&subst)?;
                    id += 1;
                    builder = builder.header(
                        name.to_string(),
                        HeaderType::Text(Text::new(expanded.to_string())),
                    );
                }

                let attached = self.borrow_attached();
                let inline = self.borrow_inline();

                let content_node = match (text, html) {
                    (Some(t), Some(h)) => {
                        MimePart::new_multipart("multipart/alternative", vec![t, h])
                    }
                    (Some(t), None) => t,
                    (None, Some(h)) => h,
                    (None, None) => anyhow::bail!("refusing to send an empty message"),
                };

                let content_node = if !inline.is_empty() {
                    let mut parts = Vec::with_capacity(inline.len() + 1);
                    parts.push(content_node);
                    parts.extend(inline.iter().cloned());
                    MimePart::new_multipart("multipart/related", parts)
                } else {
                    content_node
                };

                let root = if !attached.is_empty() {
                    let mut parts = Vec::with_capacity(attached.len() + 1);
                    parts.push(content_node);
                    parts.extend(attached.iter().cloned());

                    MimePart::new_multipart("multipart/mixed", parts)
                } else {
                    content_node
                };

                builder = builder.body(root);

                Ok(builder.write_to_string()?)
            }
        }
    }
}

impl InjectV1Request {
    /// Apply the from/subject/reply_to header shortcuts to the more
    /// general headers map to make the compile/expand phases
    fn normalize(&mut self) {
        match &mut self.content {
            Content::Builder {
                text_body: _,
                html_body: _,
                attachments: _,
                headers,
                from: _,
                subject,
                reply_to: _,
            } => {
                if let Some(v) = subject {
                    headers.insert("Subject".to_string(), v.to_string());
                }
            }
            _ => {}
        }
    }

    fn compile(&self) -> anyhow::Result<Compiled> {
        let mut env = Environment::new();
        let mut source = minijinja::Source::new();
        let mut id = 0;

        // Pass 1: create the templates
        match &self.content {
            Content::Rfc822(text) => {
                let name = id.to_string();
                source.add_template(&name, text)?;
            }
            Content::Builder {
                text_body: None,
                html_body: None,
                ..
            } => anyhow::bail!("at least one of text_body and/or html_body must be given"),
            Content::Builder {
                text_body,
                html_body,
                headers,
                ..
            } => {
                if let Some(tb) = text_body {
                    let name = id.to_string();
                    id += 1;
                    source.add_template(&name, tb)?;
                }
                if let Some(hb) = html_body {
                    // The filename extension is needed to enable auto-escaping
                    let name = format!("{id}.html");
                    id += 1;
                    source.add_template(&name, hb)?;
                }
                for value in headers.values() {
                    let name = id.to_string();
                    id += 1;
                    source.add_template(&name, value)?;
                }
            }
        }

        env.set_source(source);

        // Pass 2: retrieve the references

        fn get_templates<'b>(
            env: &'b Environment,
            content: &Content,
        ) -> anyhow::Result<Vec<Template<'b>>> {
            let mut id = 0;
            let mut templates = vec![];
            match content {
                Content::Rfc822(_) => {
                    let name = id.to_string();
                    templates.push(env.get_template(&name)?);
                }
                Content::Builder {
                    text_body,
                    html_body,
                    headers,
                    ..
                } => {
                    if text_body.is_some() {
                        let name = id.to_string();
                        id += 1;
                        templates.push(env.get_template(&name)?);
                    }
                    if html_body.is_some() {
                        // The filename extension is needed to enable auto-escaping
                        let name = format!("{id}.html");
                        id += 1;
                        templates.push(env.get_template(&name)?);
                    }
                    for _ in headers {
                        let name = id.to_string();
                        id += 1;
                        templates.push(env.get_template(&name)?);
                    }
                }
            };
            Ok(templates)
        }

        let (inline, attached) = self.attachment_data()?;

        Ok(CompiledTryBuilder {
            env,
            inline,
            attached,
            templates_builder: |env: &Environment| get_templates(env, &self.content),
        }
        .try_build()?)
    }

    fn attachment_data(&self) -> anyhow::Result<(Vec<MimePart>, Vec<MimePart>)> {
        match &self.content {
            Content::Rfc822(_) => Ok((vec![], vec![])),
            Content::Builder { attachments, .. } => {
                let mut inline = vec![];
                let mut attached = vec![];
                for a in attachments {
                    let mut part = if a.base64 {
                        MimePart::new_binary(&a.content_type, Cow::Owned(base64::decode(&a.data)?))
                    } else {
                        MimePart::new_binary(&a.content_type, Cow::Borrowed(a.data.as_bytes()))
                    };

                    if let Some(file_name) = &a.file_name {
                        part = part.attachment(file_name);
                    }

                    if let Some(cid) = &a.content_id {
                        part = part.cid(cid).inline();
                        inline.push(part);
                    } else {
                        attached.push(part);
                    }
                }
                Ok((inline, attached))
            }
        }
    }
}

async fn process_recipient<'a>(
    config: &mut LuaConfig,
    sender: &EnvelopeAddress,
    peer_address: IpAddr,
    recip: &Recipient,
    request: &'a InjectV1Request,
    compiled: &Compiled<'a>,
    auth: &AuthKind,
) -> anyhow::Result<()> {
    let recip_addr = EnvelopeAddress::parse(&recip.email)
        .with_context(|| format!("recipient email {}", recip.email))?;

    let generated = compiled.expand_for_recip(recip, &request.substitutions, &request.content)?;
    // build into a Message
    let id = SpoolId::new();
    let message = message::Message::new_dirty(
        id,
        sender.clone(),
        recip_addr,
        serde_json::json!({}),
        Arc::new(generated.into_bytes().into_boxed_slice()),
    )?;

    message.set_meta("http_auth", auth.summarize())?;

    // call callback to assign to queue
    config
        .async_call_callback("http_message_generated", message.clone())
        .await?;

    // spool and insert to queue
    let queue_name = message.get_queue_name()?;

    if queue_name != "null" {
        let deferred_spool = false; // TODO: configurable somehow
        if !deferred_spool {
            message.save().await?;
        }
        log_disposition(LogDisposition {
            kind: RecordType::Reception,
            msg: message.clone(),
            site: "",
            peer_address: Some(&ResolvedAddress {
                name: "".to_string(),
                addr: peer_address,
            }),
            response: Response {
                code: 250,
                enhanced_code: None,
                command: None,
                content: "".to_string(),
            },
            egress_source: None,
            egress_pool: None,
            relay_disposition: None,
        })
        .await;
        tokio::task::Builder::new()
            .name(&format!("http inject for {peer_address:?}"))
            .spawn_local(async move { QueueManager::insert(&queue_name, message).await })?;
    }

    Ok(())
}

async fn inject_v1_impl(
    auth: AuthKind,
    sender: EnvelopeAddress,
    peer_address: IpAddr,
    request: InjectV1Request,
) -> Result<Json<InjectV1Response>, AppError> {
    let compiled = request.compile()?;
    let mut success_count = 0;
    let mut fail_count = 0;
    let mut errors = vec![];
    let mut failed_recipients = vec![];
    let mut config = load_config().await?;
    for recip in &request.recipients {
        match process_recipient(
            &mut config,
            &sender,
            peer_address,
            recip,
            &request,
            &compiled,
            &auth,
        )
        .await
        {
            Ok(()) => {
                success_count += 1;
            }
            Err(err) => {
                fail_count += 1;
                failed_recipients.push(recip.email.to_string());
                errors.push(format!("{}: {err:#}", recip.email));
            }
        }
    }
    Ok(Json(InjectV1Response {
        success_count,
        fail_count,
        failed_recipients,
        errors,
    }))
}

pub async fn inject_v1(
    auth: AuthKind,
    InsecureClientIp(peer_address): InsecureClientIp,
    // Note: Json<> must be last in the param list
    Json(mut request): Json<InjectV1Request>,
) -> Result<Json<InjectV1Response>, AppError> {
    let sender = EnvelopeAddress::parse(&request.envelope_sender).context("envelope_sender")?;
    request.normalize();
    let (tx, rx) = tokio::sync::oneshot::channel();

    // Bounce to the thread pool where we can run async lua
    crate::runtime::Runtime::run(move || {
        tokio::task::Builder::new()
            .name(&format!("http inject_v1 for {peer_address:?}"))
            .spawn_local(async move {
                tx.send(inject_v1_impl(auth, sender, peer_address, request).await)
            })
            .expect("spawned injection task");
    })?;
    rx.await?
}

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn test_generate_basic() {
        let input = r#"From: Me
Subject: A test
To: "{{ name }}" <{{ email }}>

This is a test message to {{ name }}
"#;
        let request = InjectV1Request {
            envelope_sender: "noreply@example.com".to_string(),
            recipients: vec![Recipient {
                email: "user@example.com".to_string(),
                name: Some("James Smythe".to_string()),
                substitutions: HashMap::new(),
            }],
            substitutions: HashMap::new(),
            content: Content::Rfc822(input.to_string()),
        };

        let compiled = request.compile().unwrap();
        let generated = compiled
            .expand_for_recip(
                &request.recipients[0],
                &request.substitutions,
                &request.content,
            )
            .unwrap();
        k9::snapshot!(
            generated,
            r#"
From: Me
Subject: A test
To: "James Smythe" <user@example.com>

This is a test message to James Smythe
"#
        );
    }

    #[tokio::test]
    async fn test_generate_builder() {
        let request = InjectV1Request {
            envelope_sender: "noreply@example.com".to_string(),
            recipients: vec![Recipient {
                email: "user@example.com".to_string(),
                name: Some("James Smythe".to_string()),
                substitutions: HashMap::new(),
            }],
            substitutions: HashMap::new(),
            content: Content::Builder {
                text_body: Some("I am the plain text, {{ name }}. 😀".to_string()),
                html_body: Some(
                    "I am the <b>html</b> text, {{ name }}. 👾 <img src=\"cid:my-image\"/>"
                        .to_string(),
                ),
                attachments: vec![Attachment {
                    data: "R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7".to_string(),
                    base64: true,
                    content_type: "image/gif".to_string(),
                    content_id: Some("my-image".to_string()),
                    file_name: None,
                }],
                subject: Some("hello {{ name }}".to_string()),
                from: None,
                reply_to: None,
                headers: Default::default(),
            },
        };

        let compiled = request.compile().unwrap();
        let generated = compiled
            .expand_for_recip(
                &request.recipients[0],
                &request.substitutions,
                &request.content,
            )
            .unwrap();

        println!("{generated}");
        let parsed = mail_parser::Message::parse(&generated.as_bytes()).unwrap();
        println!("{parsed:?}");
        k9::snapshot!(
            parsed.body_html(0),
            r#"
Some(
    "I am the <b>html</b> text, James Smythe. 👾 <img src="cid:my-image"/>",
)
"#
        );
        k9::snapshot!(
            parsed.body_text(0),
            r#"
Some(
    "I am the plain text, James Smythe. 😀",
)
"#
        );

        k9::assert_equal!(parsed.html_body_count(), 1);
        k9::assert_equal!(parsed.text_body_count(), 1);
        k9::assert_equal!(parsed.attachment_count(), 1);
    }
}

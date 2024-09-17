use crate::delivery_metrics::MetricsWrappedConnection;
use crate::logging::disposition::{log_disposition, LogDisposition, RecordType};
use crate::queue::{DeliveryProto, QueueConfig, QueueManager};
use crate::ready_queue::{Dispatcher, QueueDispatcher};
use crate::spool::SpoolManager;
use anyhow::Context;
use arc_swap::ArcSwap;
use async_trait::async_trait;
use axum::extract::Json;
use axum_client_ip::InsecureClientIp;
use config::{any_err, get_or_create_sub_module, load_config, CallbackSignature, LuaConfig};
use kumo_log_types::ResolvedAddress;
use kumo_prometheus::AtomicCounter;
use kumo_server_common::http_server::auth::AuthKind;
use kumo_server_common::http_server::AppError;
use kumo_server_runtime::{Runtime, RUNTIME};
use mailparsing::{AddrSpec, Address, EncodeHeaderValue, Mailbox, MessageBuilder, MimePart};
use message::{EnvelopeAddress, Message};
use minijinja::{Environment, Template};
use minijinja_contrib::add_to_environment;
use mlua::{Lua, LuaSerdeExt};
use once_cell::sync::Lazy;
use rfc5321::Response;
use self_cell::self_cell;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use spool::SpoolId;
use std::collections::{BTreeMap, HashMap};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use throttle::ThrottleSpec;
use utoipa::{ToResponse, ToSchema};

pub const GENERATOR_QUEUE_NAME: &str = "generator.kumomta.internal";

static MSGS_RECVD: Lazy<AtomicCounter> =
    Lazy::new(|| crate::metrics_helper::total_msgs_received_for_service("http_listener"));

static HTTPINJECT: Lazy<Runtime> =
    Lazy::new(|| Runtime::new("httpinject", |cpus| cpus * 3 / 8, &HTTPINJECT_THREADS).unwrap());

static HTTPINJECT_THREADS: AtomicUsize = AtomicUsize::new(0);
static LIMIT: Lazy<ArcSwap<Option<ThrottleSpec>>> = Lazy::new(|| ArcSwap::new(Arc::new(None)));

pub fn set_httpinject_recipient_rate_limit(spec: Option<ThrottleSpec>) {
    let spec = Arc::new(spec);
    LIMIT.store(spec);
}

pub fn set_httpinject_threads(n: usize) {
    HTTPINJECT_THREADS.store(n, Ordering::SeqCst);
}

#[derive(Serialize, Deserialize, Debug, ToSchema)]
pub struct FromHeader {
    /// The email address of the sender
    #[schema(example = "sales@sender-example.com")]
    pub email: String,
    /// The displayable name of the sender
    #[serde(default)]
    #[schema(example = "Sales")]
    pub name: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, ToSchema)]
pub struct Recipient {
    /// The email address of the recipient
    #[schema(example = "john.smith@mailbox-example.com")]
    pub email: String,

    /// The displayable name of the recipient
    #[serde(default)]
    #[schema(example = "John Smith")]
    pub name: Option<String>,

    /// When using templating, this is the map of placeholder
    /// name to replacement value that should be used by the
    /// templating engine when processing just this recipient.
    /// Note that `name` is implicitly set from the `name`
    /// field, so you do not need to duplicate it here.
    #[serde(default)]
    #[schema(example=json!({
        "age": 42,
        "gender": "male",
    }))]
    pub substitutions: HashMap<String, Value>,
}

#[derive(Serialize, Deserialize, Debug, ToSchema)]
pub struct InjectV1Request {
    /// Specify the envelope sender that will be sent in the
    /// MAIL FROM portion of SMTP.
    #[schema(example = "some.id@bounces.sender-example.com")]
    pub envelope_sender: String,

    /// The list of recipients
    pub recipients: Vec<Recipient>,

    /// The content of the message
    pub content: Content,

    /// When using templating, this is the map of placeholder
    /// name to replacement value that should be used by
    /// the templating engine.  This map applies to all
    /// recipients, with the per-recipient substitutions
    /// taking precedence.
    #[serde(default)]
    #[schema(example=json!({
        "campaign_title": "Fall Campaign",
    }))]
    pub substitutions: HashMap<String, Value>,

    /// When set to true, the message will not be written to
    /// the spool until it encounters its first transient failure.
    /// This can improve injection rate but introduces the risk
    /// of loss of accountability for the message if the system
    /// were to crash before the message is delivered or written
    /// to spool, so use with caution!
    #[serde(default)]
    pub deferred_spool: bool,

    /// When set to true, the injection request will be queued
    /// and the actual generation and substitution will happen
    /// asynchronously with respect to the injection request.
    #[serde(default)]
    pub deferred_generation: bool,
}

#[derive(Serialize, Deserialize, Debug, ToResponse, ToSchema)]
pub struct InjectV1Response {
    /// The number of messages that were injected successfully
    pub success_count: usize,
    /// The number of messages that failed to inject
    pub fail_count: usize,

    /// The list of failed recipients
    pub failed_recipients: Vec<String>,

    /// The list of error messages
    pub errors: Vec<String>,
}

/// The message content.
/// Can either be a fully formed MIME message, or a json
/// object describing the MIME structure that should be created.
#[derive(Serialize, Deserialize, Debug, ToSchema)]
#[serde(untagged)]
pub enum Content {
    /// A complete MIME message string
    Rfc822(String),
    /// Describe the MIME structure to be created
    Builder {
        /// If set, will be used to create a text/plain part
        #[serde(default)]
        text_body: Option<String>,

        /// If set, will be used to create a text/html part
        #[serde(default)]
        html_body: Option<String>,

        /// Optional list of attachments
        #[serde(default)]
        attachments: Vec<Attachment>,

        /// Optional map of headers to include in the message.
        /// This is a map of header name to header value
        #[serde(default)]
        #[schema(example=json!({
            "X-Tenant": "MyTenant"
        }))]
        headers: BTreeMap<String, String>,

        /// Set the From: header
        #[serde(default)]
        from: Option<FromHeader>,

        /// Set the Subject: header
        #[serde(default)]
        subject: Option<String>,

        /// Set the Reply-To: header
        #[serde(default)]
        reply_to: Option<FromHeader>,
    },
}

/// An email header.
#[derive(Serialize, Deserialize, Debug, ToSchema)]
#[serde(untagged)]
pub enum Header {
    Full(String),
    NameValue(String, String),
}

#[derive(Serialize, Deserialize, Debug, ToSchema)]
pub struct Attachment {
    /// The content of the payload.
    /// This is interpreted as UTF-8 text unless the
    /// `base64` field is set to `true`.
    data: String,
    /// The MIME `Content-Type` header that should be
    /// set for this attachment.
    content_type: String,
    /// Set the `Content-ID` header for this attachment.
    /// This is used in multipart/related messages to
    /// embed inline images in text/html parts.
    #[serde(default)]
    content_id: Option<String>,
    /// The the preferred filename for the attachment
    #[serde(default)]
    file_name: Option<String>,
    /// If true, the `data` field must be encoded as base64
    #[serde(default)]
    base64: bool,
}

type TemplateList<'a> = Vec<Template<'a, 'a>>;

self_cell!(
    struct CompiledTemplates<'a> {
        owner: Environment<'a>,
        #[covariant]
        dependent: TemplateList,
    }
);

struct Compiled<'a> {
    env_and_templates: CompiledTemplates<'a>,
    attached: Vec<MimePart<'a>>,
}

impl<'a> Compiled<'a> {
    pub fn expand_for_recip(
        &self,
        recip: &Recipient,
        global_subst: &HashMap<String, Value>,
        content: &Content,
    ) -> anyhow::Result<String> {
        let mut subst = serde_json::Map::new();
        for (k, v) in global_subst {
            subst.insert(k.clone(), v.clone());
        }
        subst.insert("email".to_string(), recip.email.to_string().into());
        if let Some(name) = &recip.name {
            subst.insert("name".to_string(), name.to_string().into());
        }

        for (k, v) in &recip.substitutions {
            subst.insert(k.clone(), v.clone());
        }

        let subst = serde_json::Value::Object(subst);

        let mut id = 0;
        match content {
            Content::Rfc822(_) => {
                let content = self.env_and_templates.borrow_dependent()[id].render(&subst)?;
                let mut msg = MimePart::parse(&*content)
                    .with_context(|| format!("failed to parse content: {content}"))?
                    .rebuild()
                    .with_context(|| format!("failed to rebuild content {content}"))?;

                if msg.headers().mime_version()?.is_none() {
                    msg.headers_mut().set_mime_version("1.0");
                }

                Ok(msg.to_message_string())
            }
            Content::Builder {
                text_body,
                html_body,
                headers,
                ..
            } => {
                let mut builder = MessageBuilder::new();

                if text_body.is_some() {
                    builder
                        .text_plain(&self.env_and_templates.borrow_dependent()[id].render(&subst)?);
                    id += 1;
                }

                if html_body.is_some() {
                    builder
                        .text_html(&self.env_and_templates.borrow_dependent()[id].render(&subst)?);
                    id += 1;
                }

                builder.set_to(Address::Mailbox(Mailbox {
                    name: recip.name.clone(),
                    address: AddrSpec::parse(&recip.email)?,
                }));

                for (name, _value) in headers {
                    let expanded = self.env_and_templates.borrow_dependent()[id].render(&subst)?;
                    id += 1;
                    builder.push(mailparsing::Header::new_unstructured(
                        name.to_string(),
                        expanded.to_string(),
                    ));
                }

                for part in &self.attached {
                    builder.attach_part(part.clone());
                }

                Ok(builder.build()?.to_message_string())
            }
        }
    }
}

impl InjectV1Request {
    /// Apply the from/subject/reply_to header shortcuts to the more
    /// general headers map to make the compile/expand phases
    fn normalize(&mut self) -> anyhow::Result<()> {
        match &mut self.content {
            Content::Builder {
                text_body: _,
                html_body: _,
                attachments: _,
                headers,
                from,
                subject,
                reply_to,
            } => {
                if let Some(from) = from {
                    let mailbox = Address::Mailbox(Mailbox {
                        name: from.name.clone(),
                        address: AddrSpec::parse(&from.email)?,
                    });

                    headers.insert("From".to_string(), mailbox.encode_value().to_string());
                }
                if let Some(reply_to) = reply_to {
                    let mailbox = Address::Mailbox(Mailbox {
                        name: reply_to.name.clone(),
                        address: AddrSpec::parse(&reply_to.email)?,
                    });
                    headers.insert("Reply-To".to_string(), mailbox.encode_value().to_string());
                }
                if let Some(v) = subject {
                    headers.insert("Subject".to_string(), v.to_string());
                }

                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn compile(&self) -> anyhow::Result<Compiled> {
        let mut env = Environment::new();
        add_to_environment(&mut env);
        let mut id = 0;

        // Pass 1: create the templates
        match &self.content {
            Content::Rfc822(text) => {
                let name = id.to_string();
                env.add_template_owned(name, text)?;
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
                    env.add_template_owned(name, tb)?;
                }
                if let Some(hb) = html_body {
                    // The filename extension is needed to enable auto-escaping
                    let name = format!("{id}.html");
                    id += 1;
                    env.add_template_owned(name, hb)?;
                }
                for value in headers.values() {
                    let name = id.to_string();
                    id += 1;
                    env.add_template_owned(name, value)?;
                }
            }
        }

        // Pass 2: retrieve the references

        fn get_templates<'b>(
            env: &'b Environment,
            content: &Content,
        ) -> anyhow::Result<TemplateList<'b>> {
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

        let attached = self.attachment_data()?;

        let env_and_templates =
            CompiledTemplates::try_new(env, |env: &Environment| get_templates(env, &self.content))?;

        Ok(Compiled {
            env_and_templates,
            attached,
        })
    }

    fn attachment_data(&self) -> anyhow::Result<Vec<MimePart>> {
        match &self.content {
            Content::Rfc822(_) => Ok(vec![]),
            Content::Builder { attachments, .. } => {
                let mut attached = vec![];
                for a in attachments {
                    let opts = mailparsing::AttachmentOptions {
                        file_name: a.file_name.clone(),
                        inline: a.content_id.is_some(),
                        content_id: a.content_id.clone(),
                    };

                    let decoded_data;

                    let part = MimePart::new_binary(
                        &a.content_type,
                        if a.base64 {
                            decoded_data = data_encoding::BASE64.decode(a.data.as_bytes())?;
                            &decoded_data
                        } else {
                            a.data.as_bytes()
                        },
                        Some(&opts),
                    );

                    attached.push(part);
                }
                Ok(attached)
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
    MSGS_RECVD.inc();
    let recip_addr = EnvelopeAddress::parse(&recip.email)
        .with_context(|| format!("recipient email {}", recip.email))?;

    let generated = compiled.expand_for_recip(recip, &request.substitutions, &request.content)?;

    // Ensure that there are no bare LF in the message, as that will
    // confuse SMTP delivery!
    let normalized = mailparsing::normalize_crlf(generated.as_bytes());

    // build into a Message
    let id = SpoolId::new();
    let message = Message::new_dirty(
        id,
        sender.clone(),
        recip_addr,
        serde_json::json!({}),
        Arc::new(normalized.into_boxed_slice()),
    )?;

    message.set_meta("http_auth", auth.summarize())?;
    message.set_meta("reception_protocol", "HTTP")?;
    message.set_meta("received_from", peer_address.to_string())?;

    // call callback to assign to queue
    let sig = CallbackSignature::<message::Message, ()>::new("http_message_generated");
    config.async_call_callback(&sig, message.clone()).await?;

    // spool and insert to queue
    let queue_name = message.get_queue_name()?;

    if queue_name != "null" {
        if !request.deferred_spool {
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
            delivery_protocol: None,
            tls_info: None,
            source_address: None,
            provider: None,
        })
        .await;
        QueueManager::insert(&queue_name, message).await?;
    }

    Ok(())
}

async fn queue_deferred(
    auth: AuthKind,
    sender: EnvelopeAddress,
    peer_address: IpAddr,
    mut request: InjectV1Request,
) -> anyhow::Result<InjectV1Response> {
    request.deferred_generation = false;
    // build into a Message
    let id = SpoolId::new();
    let payload: Vec<u8> = serde_json::to_string(&request)?.into();
    let message = message::Message::new_dirty(
        id,
        sender.clone(),
        EnvelopeAddress::null_sender(),
        serde_json::json!({}),
        Arc::new(payload.into_boxed_slice()),
    )?;

    message.set_meta("http_auth", auth.summarize())?;
    message.set_meta("reception_protocol", "HTTP")?;
    message.set_meta("received_from", peer_address.to_string())?;
    message.set_meta("queue", GENERATOR_QUEUE_NAME)?;
    if !request.deferred_spool {
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
        delivery_protocol: None,
        tls_info: None,
        source_address: None,
        provider: None,
    })
    .await;
    QueueManager::insert(GENERATOR_QUEUE_NAME, message).await?;
    Ok(InjectV1Response {
        success_count: 0,
        fail_count: 0,
        failed_recipients: vec![],
        errors: vec![],
    })
}

async fn inject_v1_impl(
    auth: AuthKind,
    sender: EnvelopeAddress,
    peer_address: IpAddr,
    mut request: InjectV1Request,
) -> Result<Json<InjectV1Response>, AppError> {
    request.normalize()?;

    if request.deferred_generation {
        return Ok(Json(
            queue_deferred(auth, sender, peer_address, request).await?,
        ));
    }

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

pub fn make_generate_queue_config() -> anyhow::Result<QueueConfig> {
    Ok(QueueConfig {
        protocol: DeliveryProto::HttpInjectionGenerator,
        retry_interval: Duration::from_secs(10),
        ..QueueConfig::default()
    })
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let module = get_or_create_sub_module(lua, "api.inject")?;

    module.set(
        "inject_v1",
        lua.create_async_function(|lua, request: mlua::Value| async move {
            let request: InjectV1Request = lua.from_value(request)?;
            let sender = EnvelopeAddress::parse(&request.envelope_sender)
                .context("envelope_sender")
                .map_err(any_err)?;
            let my_ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
            let result = inject_v1_impl(AuthKind::TrustedIp(my_ip), sender, my_ip, request)
                .await
                .map_err(|err| any_err(err.0))?;

            lua.to_value(&result.0)
        })?,
    )?;

    Ok(())
}

/// Inject a message using a given message body, with template expansion,
/// to a list of recipients.
#[utoipa::path(
    post,
    tag="inject",
    path="/api/inject/v1",
    responses(
        (status = 200, description = "Message(s) injected successfully", body=InjectV1Response)
    ),
)]
pub async fn inject_v1(
    auth: AuthKind,
    InsecureClientIp(peer_address): InsecureClientIp,
    // Note: Json<> must be last in the param list
    Json(request): Json<InjectV1Request>,
) -> Result<Json<InjectV1Response>, AppError> {
    if kumo_server_memory::get_headroom() == 0 {
        // Using too much memory
        return Err(anyhow::anyhow!("load shedding").into());
    }
    if kumo_server_common::disk_space::is_over_limit() {
        return Err(anyhow::anyhow!("disk is too full").into());
    }

    let limit = LIMIT.load();
    if let Some(limit) = limit.as_ref() {
        loop {
            let result = limit
                .throttle_quantity(
                    "kumomta.httpinject.ratelimit",
                    request.recipients.len() as u64,
                )
                .await?;
            if let Some(delay) = result.retry_after {
                tokio::time::sleep(delay).await;
                continue;
            } else {
                break;
            }
        }
    }

    let sender = EnvelopeAddress::parse(&request.envelope_sender).context("envelope_sender")?;

    let (tx, rx) = tokio::sync::oneshot::channel();

    // Bounce to the thread pool where we can run async lua
    let pool = if request.deferred_generation {
        &*RUNTIME
    } else {
        &*HTTPINJECT
    };

    pool.spawn(format!("http inject_v1 for {peer_address:?}"), move || {
        Ok(async move { tx.send(inject_v1_impl(auth, sender, peer_address, request).await) })
    })
    .await?;
    rx.await?
}

#[derive(Debug)]
pub struct HttpInjectionGeneratorDispatcher {
    connection: Option<MetricsWrappedConnection<()>>,
}

impl HttpInjectionGeneratorDispatcher {
    pub fn new() -> Self {
        Self { connection: None }
    }

    async fn try_send(&self, msg: Message) -> anyhow::Result<()> {
        HTTPINJECT
            .spawn("http inject_v1".to_string(), move || {
                Ok(async move {
                    msg.load_data_if_needed().await?;
                    msg.load_meta_if_needed().await?;
                    let data = msg.get_data();
                    let request: InjectV1Request = serde_json::from_slice(&data)?;
                    let peer_address = msg
                        .get_meta_string("received_from")?
                        .ok_or_else(|| anyhow::anyhow!("received_from metadata is missing!?"))?
                        .parse()?;

                    let sender = msg.sender()?;

                    let _ = inject_v1_impl(
                        AuthKind::TrustedIp(peer_address),
                        sender,
                        peer_address,
                        request,
                    )
                    .await
                    .map_err(|err| err.0)?;

                    Ok(())
                })
            })
            .await?
            .await?
    }
}

#[async_trait(?Send)]
impl QueueDispatcher for HttpInjectionGeneratorDispatcher {
    async fn close_connection(&mut self, _dispatcher: &mut Dispatcher) -> anyhow::Result<bool> {
        match self.connection.take() {
            Some(_) => Ok(true),
            None => Ok(false),
        }
    }
    async fn attempt_connection(&mut self, dispatcher: &mut Dispatcher) -> anyhow::Result<()> {
        if self.connection.is_none() {
            self.connection
                .replace(dispatcher.metrics.wrap_connection(()));
        }
        Ok(())
    }
    async fn have_more_connection_candidates(&mut self, _dispatcher: &mut Dispatcher) -> bool {
        false
    }
    async fn deliver_message(
        &mut self,
        mut msgs: Vec<Message>,
        dispatcher: &mut Dispatcher,
    ) -> anyhow::Result<()> {
        // parse out the inject payload and run it
        anyhow::ensure!(
            msgs.len() == 1,
            "smtp_dispatcher only supports a batch size of 1"
        );
        let msg = msgs.pop().expect("just verified that there is one");

        let response = match self.try_send(msg).await {
            Ok(()) => Response {
                code: 250,
                enhanced_code: None,
                content: "ok".to_string(),
                command: None,
            },
            Err(err) => Response {
                code: 500,
                enhanced_code: None,
                content: format!("{err:#}"),
                command: None,
            },
        };

        tracing::debug!("Delivered OK! {response:?}");
        let was_ok = response.code == 250;

        if let Some(msg) = dispatcher.msgs.pop() {
            log_disposition(LogDisposition {
                kind: if was_ok {
                    RecordType::Delivery
                } else {
                    RecordType::Bounce
                },
                msg: msg.clone(),
                site: &dispatcher.name,
                peer_address: None,
                response,
                egress_pool: None,
                egress_source: None,
                relay_disposition: None,
                delivery_protocol: Some("HttpInjectionGenerator"),
                tls_info: None,
                source_address: None,
                provider: dispatcher.path_config.borrow().provider_name.as_deref(),
            })
            .await;
            SpoolManager::remove_from_spool(*msg.id()).await?;
        }
        if was_ok {
            dispatcher.metrics.inc_delivered();
        } else {
            dispatcher.metrics.inc_fail();
        }

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn test_generate_basic() {
        let input = r#"From: Me <me@example.com>
Subject: A test 🛳️
To: "{{ name }}" <{{ email }}>

This is a test message to {{ name }}, with some 👻🍉💩 emoji!
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
            deferred_spool: true,
            deferred_generation: false,
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
Content-Type: text/plain;\r
\tcharset="utf-8"\r
Content-Transfer-Encoding: quoted-printable\r
From: Me <me@example.com>\r
Subject: A test =?UTF-8?q?=F0=9F=9B=B3=EF=B8=8F?=\r
To: James Smythe <user@example.com>\r
Mime-Version: 1.0\r
\r
This is a test message to James Smythe, with some =F0=9F=91=BB=F0=9F=8D=89=\r
=F0=9F=92=A9 emoji!\r

"#
        );
    }

    #[tokio::test]
    async fn test_generate_builder() {
        let mut request = InjectV1Request {
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
                subject: Some("hello {{ name }} 👫".to_string()),
                from: None,
                reply_to: None,
                headers: Default::default(),
            },
            deferred_spool: true,
            deferred_generation: false,
        };

        request.normalize().unwrap();
        let compiled = request.compile().unwrap();
        let generated = compiled
            .expand_for_recip(
                &request.recipients[0],
                &request.substitutions,
                &request.content,
            )
            .unwrap();

        println!("{generated}");
        let parsed = MimePart::parse(generated.as_str()).unwrap();
        println!("{parsed:#?}");

        assert!(parsed.headers().mime_version().unwrap().is_some());

        let structure = parsed.simplified_structure().unwrap();
        eprintln!("{structure:?}");

        k9::snapshot!(
            structure.html,
            r#"
Some(
    "I am the <b>html</b> text, James Smythe. 👾 <img src="cid:my-image"/>\r
",
)
"#
        );
        k9::snapshot!(
            structure.text,
            r#"
Some(
    "I am the plain text, James Smythe. 😀\r
",
)
"#
        );

        k9::snapshot!(
            structure.headers.subject().unwrap(),
            r#"
Some(
    "hello James Smythe 👫",
)
"#
        );

        k9::assert_equal!(structure.attachments.len(), 1);
    }

    #[tokio::test]
    async fn test_to_from_builder() {
        let mut request = InjectV1Request {
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
                subject: Some("hello {{ name }}".to_string()),
                from: Some(FromHeader {
                    email: "from@example.com".to_string(),
                    name: Some("Sender Name".to_string()),
                }),
                reply_to: None,
                headers: Default::default(),
                attachments: vec![],
            },
            deferred_spool: true,
            deferred_generation: false,
        };

        request.normalize().unwrap();
        let compiled = request.compile().unwrap();
        let generated = compiled
            .expand_for_recip(
                &request.recipients[0],
                &request.substitutions,
                &request.content,
            )
            .unwrap();

        println!("{generated}");
        let parsed = MimePart::parse(generated.as_str()).unwrap();
        println!("{parsed:?}");

        k9::snapshot!(
            parsed.headers().to(),
            r#"
Ok(
    Some(
        AddressList(
            [
                Mailbox(
                    Mailbox {
                        name: Some(
                            "James Smythe",
                        ),
                        address: AddrSpec {
                            local_part: "user",
                            domain: "example.com",
                        },
                    },
                ),
            ],
        ),
    ),
)
"#
        );

        k9::snapshot!(
            parsed.headers().from(),
            r#"
Ok(
    Some(
        MailboxList(
            [
                Mailbox {
                    name: Some(
                        "Sender Name",
                    ),
                    address: AddrSpec {
                        local_part: "from",
                        domain: "example.com",
                    },
                },
            ],
        ),
    ),
)
"#
        );
    }
}

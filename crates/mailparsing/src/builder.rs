use crate::mimepart::AttachmentOptions;
use crate::{HeaderMap, MailParsingError, MimePart};

#[derive(Default)]
pub struct MessageBuilder<'a> {
    text: Option<String>,
    html: Option<String>,
    headers: HeaderMap<'a>,
    inline: Vec<MimePart<'a>>,
    attached: Vec<MimePart<'a>>,
    stable_content_ids: bool,
}

impl<'a> MessageBuilder<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_stable_content_ids(&mut self, v: bool) {
        self.stable_content_ids = v;
    }

    pub fn text_plain(&mut self, text: &str) {
        self.text.replace(text.to_string());
    }

    pub fn text_html(&mut self, html: &str) {
        self.html.replace(html.to_string());
    }

    pub fn attach(&mut self, content_type: &str, data: &[u8], opts: Option<&AttachmentOptions>) {
        let is_inline = opts.map(|opt| opt.inline).unwrap_or(false);

        let part = MimePart::new_binary(content_type, data, opts);

        if is_inline {
            self.inline.push(part);
        } else {
            self.attached.push(part);
        }
    }

    pub fn build(self) -> Result<MimePart<'a>, MailParsingError> {
        let text = self.text.as_deref().map(MimePart::new_text_plain);
        let html = self.html.as_deref().map(MimePart::new_html);

        let content_node = match (text, html) {
            (Some(t), Some(h)) => MimePart::new_multipart(
                "multipart/alternative",
                vec![t, h],
                if self.stable_content_ids {
                    Some("ma-boundary")
                } else {
                    None
                },
            ),
            (Some(t), None) => t,
            (None, Some(h)) => h,
            (None, None) => {
                return Err(MailParsingError::BuildError(
                    "no text or html part was specified",
                ))
            }
        };

        let content_node = if !self.inline.is_empty() {
            let mut parts = Vec::with_capacity(self.inline.len() + 1);
            parts.push(content_node);
            parts.extend(self.inline.into_iter());
            MimePart::new_multipart(
                "multipart/related",
                parts,
                if self.stable_content_ids {
                    Some("mr-boundary")
                } else {
                    None
                },
            )
        } else {
            content_node
        };

        let mut root = if !self.attached.is_empty() {
            let mut parts = Vec::with_capacity(self.attached.len() + 1);
            parts.push(content_node);
            parts.extend(self.attached.into_iter());
            MimePart::new_multipart(
                "multipart/mixed",
                parts,
                if self.stable_content_ids {
                    Some("mm-boundary")
                } else {
                    None
                },
            )
        } else {
            content_node
        };

        root.headers_mut()
            .headers
            .extend(self.headers.headers.into_iter());

        if root.headers().mime_version()?.is_none() {
            root.headers_mut().set_mime_version("1.0");
        }

        // TODO: Date, and Content-Id

        Ok(root)
    }
}

impl<'a> std::ops::Deref for MessageBuilder<'a> {
    type Target = HeaderMap<'a>;
    fn deref(&self) -> &HeaderMap<'a> {
        &self.headers
    }
}

impl<'a> std::ops::DerefMut for MessageBuilder<'a> {
    fn deref_mut(&mut self) -> &mut HeaderMap<'a> {
        &mut self.headers
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn basic() {
        let mut b = MessageBuilder::new();
        b.set_stable_content_ids(true);
        b.set_subject("Hello there! 🍉");
        b.text_plain("This is the body! 👻");
        b.text_html("<b>this is html 🚀</b>");
        let msg = b.build().unwrap();
        k9::snapshot!(
            msg.to_message_string(),
            r#"
Content-Type: multipart/alternative;\r
\tboundary="ma-boundary"\r
Content-Transfer-Encoding: quoted-printable\r
Subject: Hello there! =?UTF-8?q?=F0=9F=8D=89?=\r
Mime-Version: 1.0\r
\r
--ma-boundary\r
Content-Type: text/plain;\r
\tcharset="utf-8"\r
Content-Transfer-Encoding: quoted-printable\r
\r
This is the body! =F0=9F=91=BB\r
--ma-boundary\r
Content-Type: text/html;\r
\tcharset="utf-8"\r
Content-Transfer-Encoding: quoted-printable\r
\r
<b>this is html =F0=9F=9A=80</b>\r
--ma-boundary--\r

"#
        );
    }
}

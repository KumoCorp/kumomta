use crate::mimepart::AttachmentOptions;
use crate::{HeaderMap, MailParsingError, MimePart};

#[derive(Default)]
pub struct MessageBuilder<'a> {
    text: Option<String>,
    html: Option<String>,
    // <https://amp.dev/documentation/guides-and-tutorials/email/learn/email-spec/amp-email-structure>
    amp_html: Option<String>,
    headers: HeaderMap<'a>,
    inline: Vec<MimePart<'a>>,
    attached: Vec<MimePart<'a>>,
    stable_content: bool,
}

impl<'a> MessageBuilder<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_stable_content(&mut self, v: bool) {
        self.stable_content = v;
    }

    pub fn text_plain(&mut self, text: &str) {
        self.text.replace(text.to_string());
    }

    pub fn text_html(&mut self, html: &str) {
        self.html.replace(html.to_string());
    }

    pub fn text_amp_html(&mut self, html: &str) {
        self.amp_html.replace(html.to_string());
    }

    pub fn attach(
        &mut self,
        content_type: &str,
        data: &[u8],
        opts: Option<&AttachmentOptions>,
    ) -> Result<(), MailParsingError> {
        let is_inline = opts.map(|opt| opt.inline).unwrap_or(false);

        let part = MimePart::new_binary(content_type, data, opts)?;

        if is_inline {
            self.inline.push(part);
        } else {
            self.attached.push(part);
        }

        Ok(())
    }

    pub fn attach_part(&mut self, part: MimePart<'a>) {
        let is_inline = part
            .headers()
            .content_disposition()
            .ok()
            .and_then(|opt_cd| opt_cd.map(|cd| cd.value == "inline"))
            .unwrap_or(false);
        if is_inline {
            self.inline.push(part);
        } else {
            self.attached.push(part);
        }
    }

    pub fn build(self) -> Result<MimePart<'a>, MailParsingError> {
        let text = self.text.as_deref().map(MimePart::new_text_plain);
        let html = self.html.as_deref().map(MimePart::new_html);
        let amp_html = self
            .amp_html
            .as_deref()
            .map(|html| MimePart::new_text("text/x-amp-html", html));

        // Phrase the alternative parts.
        // Note that, when there are both HTML and AMP HTML parts,
        // we are careful to NOT place the amp part as the last part
        // as the AMP docs recommend that we keep the regular HTML
        // part as the last part as some clients can only render
        // the last alternative part(!)
        let content_node = match (text, html, amp_html) {
            (Some(t), Some(h), Some(amp)) => MimePart::new_multipart(
                "multipart/alternative",
                vec![t?, amp?, h?],
                if self.stable_content {
                    Some("ma-boundary")
                } else {
                    None
                },
            )?,
            (Some(first), Some(second), None)
            | (None, Some(second), Some(first))
            | (Some(first), None, Some(second)) => MimePart::new_multipart(
                "multipart/alternative",
                vec![first?, second?],
                if self.stable_content {
                    Some("ma-boundary")
                } else {
                    None
                },
            )?,
            (Some(only), None, None) | (None, Some(only), None) => only?,
            (None, None, Some(_amp)) => {
                return Err(MailParsingError::BuildError(
                    "the AMP email spec requires at least one non-amp part \
                        to be present in the message",
                ))
            }
            (None, None, None) => {
                return Err(MailParsingError::BuildError(
                    "no text or html part was specified",
                ))
            }
        };

        let content_node = if !self.inline.is_empty() {
            let mut parts = Vec::with_capacity(self.inline.len() + 1);
            parts.push(content_node);
            parts.extend(self.inline);
            MimePart::new_multipart(
                "multipart/related",
                parts,
                if self.stable_content {
                    Some("mr-boundary")
                } else {
                    None
                },
            )?
        } else {
            content_node
        };

        let mut root = if !self.attached.is_empty() {
            let mut parts = Vec::with_capacity(self.attached.len() + 1);
            parts.push(content_node);
            parts.extend(self.attached);
            MimePart::new_multipart(
                "multipart/mixed",
                parts,
                if self.stable_content {
                    Some("mm-boundary")
                } else {
                    None
                },
            )?
        } else {
            content_node
        };

        root.headers_mut().headers.extend(self.headers.headers);

        if root.headers().mime_version()?.is_none() {
            root.headers_mut().set_mime_version("1.0")?;
        }

        if root.headers().date()?.is_none() {
            if self.stable_content {
                root.headers_mut().set_date(
                    chrono::DateTime::parse_from_rfc2822("Tue, 1 Jul 2003 10:52:37 +0200")
                        .expect("test date to be valid"),
                )?;
            } else {
                root.headers_mut().set_date(chrono::Utc::now())?;
            };
        }

        // TODO: Content-Id? Hard to do without context on the machine
        // name and other external data, so perhaps punt this indefinitely
        // from this module?

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
        b.set_stable_content(true);
        b.set_subject("Hello there! 🍉").unwrap();
        b.text_plain("This is the body! 👻");
        b.text_html("<b>this is html 🚀</b>");
        let msg = b.build().unwrap();
        k9::snapshot!(
            msg.to_message_string(),
            r#"
Content-Type: multipart/alternative;\r
\tboundary="ma-boundary"\r
Subject: =?UTF-8?q?Hello_there!_=F0=9F=8D=89?=\r
Mime-Version: 1.0\r
Date: Tue, 1 Jul 2003 10:52:37 +0200\r
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

    #[test]
    fn amp() {
        let mut b = MessageBuilder::new();
        b.set_stable_content(true);
        b.set_subject("Hello there! 🍉").unwrap();
        b.text_plain("This is the body! 👻");
        b.text_html("<b>this is html 🚀</b>");
        b.text_amp_html(
            &r#"<!doctype html>
<html ⚡4email>
<head>
  <meta charset="utf-8">
  <style amp4email-boilerplate>body{visibility:hidden}</style>
  <script async src="https://cdn.ampproject.org/v0.js"></script>
</head>
<body>
Hello World in AMP!
</body>
</html>
"#
            .replace("\n", "\r\n"),
        );
        let msg = b.build().unwrap();
        k9::snapshot!(
            msg.to_message_string(),
            r#"
Content-Type: multipart/alternative;\r
\tboundary="ma-boundary"\r
Subject: =?UTF-8?q?Hello_there!_=F0=9F=8D=89?=\r
Mime-Version: 1.0\r
Date: Tue, 1 Jul 2003 10:52:37 +0200\r
\r
--ma-boundary\r
Content-Type: text/plain;\r
\tcharset="utf-8"\r
Content-Transfer-Encoding: quoted-printable\r
\r
This is the body! =F0=9F=91=BB\r
--ma-boundary\r
Content-Type: text/x-amp-html;\r
\tcharset="utf-8"\r
Content-Transfer-Encoding: quoted-printable\r
\r
<!doctype html>\r
<html =E2=9A=A14email>\r
<head>\r
  <meta charset=3D"utf-8">\r
  <style amp4email-boilerplate>body{visibility:hidden}</style>\r
  <script async src=3D"https://cdn.ampproject.org/v0.js"></script>\r
</head>\r
<body>\r
Hello World in AMP!\r
</body>\r
</html>\r
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

    #[test]
    fn utf8_attachment_name() {
        let mut b = MessageBuilder::new();
        b.set_stable_content(true);
        b.set_subject("Hello there! 🍉").unwrap();
        b.text_plain("This is the body! 👻");
        b.attach(
            "text/plain",
            b"hello",
            Some(&AttachmentOptions {
                content_id: None,
                file_name: Some("日本語の添付.txt".to_string()),
                inline: false,
            }),
        )
        .unwrap();
        let msg = b.build().unwrap();
        k9::snapshot!(
            msg.to_message_string(),
            r#"
Content-Type: multipart/mixed;\r
\tboundary="mm-boundary"\r
Subject: =?UTF-8?q?Hello_there!_=F0=9F=8D=89?=\r
Mime-Version: 1.0\r
Date: Tue, 1 Jul 2003 10:52:37 +0200\r
\r
--mm-boundary\r
Content-Type: text/plain;\r
\tcharset="utf-8"\r
Content-Transfer-Encoding: quoted-printable\r
\r
This is the body! =F0=9F=91=BB\r
--mm-boundary\r
Content-Disposition: attachment;\r
\tfilename*0*=UTF-8''%E6%97%A5%E6%9C%AC%E8%AA%9E%E3%81%AE%E6%B7%BB%E4%BB%98.;\r
\tfilename*1*=txt\r
Content-Type: text/plain;\r
\tname="=?UTF-8?q?=E6=97=A5=E6=9C=AC=E8=AA=9E=E3=81=AE=E6=B7=BB=E4=BB=98.txt?="\r
Content-Transfer-Encoding: base64\r
\r
aGVsbG8=\r
--mm-boundary--\r

"#
        );
    }
}

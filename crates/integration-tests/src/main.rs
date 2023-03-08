#[cfg(test)]
mod kumod;

fn main() {
    println!("Run me via `cargo nextest run` or `cargo test`");
}

#[cfg(test)]
mod test {
    use super::kumod::*;
    use mailparse::MailHeaderMap;
    use rfc5321::*;
    use std::time::Duration;

    #[tokio::test]
    async fn end_to_end() -> anyhow::Result<()> {
        eprintln!("start sink");
        let mut sink = KumoDaemon::spawn_maildir().await?;
        eprintln!("start source");
        let smtp = sink.listener("smtp");
        let mut source = KumoDaemon::spawn(KumoArgs {
            policy_file: "source.lua".to_string(),
            env: vec![("KUMOD_SMTP_SINK_PORT".to_string(), smtp.port().to_string())],
        })
        .await?;

        eprintln!("sending message");
        tokio::time::timeout(Duration::from_secs(10), async {
            let mut client = source.smtp_client().await?;
            client.ehlo("localhost").await?;
            const BODY: &str = "From: <me@localhost>\r\n\
                                To: <you@localhost>\r\n\
                                Subject: a test message\r\n\
                                \r\n\
                                All done";
            let response = client
                .send_mail(
                    ReversePath::try_from("sender@example.com").unwrap(),
                    ForwardPath::try_from("recipient@example.com").unwrap(),
                    BODY,
                )
                .await?;
            eprintln!("{response:?}");
            Ok::<(), anyhow::Error>(())
        })
        .await??;

        eprintln!("waiting for maildir to populate");

        sink.wait_for_maildir_count(1, Duration::from_secs(10))
            .await;

        let (res_1, res_2) = tokio::join!(source.stop(), sink.stop());
        res_1?;
        res_2?;
        println!("Stopped!");

        eprintln!("source logs:");
        source.dump_logs()?;
        eprintln!("sink logs:");
        sink.dump_logs()?;

        let mut messages = vec![];
        let md = sink.maildir();
        for entry in md.list_new() {
            messages.push(entry?);
        }

        assert_eq!(messages.len(), 1);
        let parsed = messages[0].parsed()?;
        println!("headers: {:?}", parsed.headers);

        assert!(parsed.headers.get_first_header("Received").is_some());
        assert!(parsed.headers.get_first_header("X-KumoRef").is_some());
        assert_eq!(
            parsed.headers.get_first_value("From").unwrap(),
            "<me@localhost>"
        );
        assert_eq!(
            parsed.headers.get_first_value("To").unwrap(),
            "<you@localhost>"
        );
        assert_eq!(
            parsed.headers.get_first_value("Subject").unwrap(),
            "a test message"
        );
        assert_eq!(parsed.get_body()?, "All done\r\n");

        Ok(())
    }
}

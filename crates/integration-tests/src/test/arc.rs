use crate::kumod::{generate_message_text, DaemonWithMaildirOptions, MailGenParams};
use anyhow::Context;
use dns_resolver::TestResolver;
use k9::assert_equal;
use kumo_dkim::arc::{ChainValidationStatus, ARC};
use kumo_dkim::ParsedEmail;
use rfc5321::ClientError;
use std::time::Duration;

/// Verify that arc_verify and arc_seal operate, end to end
#[tokio::test]
async fn arc() -> anyhow::Result<()> {
    let mut daemon = DaemonWithMaildirOptions::new()
        .policy_file("arc.lua")
        .start()
        .await
        .context("DaemonWithMaildir::start")?;

    let mut client = daemon.smtp_client().await.context("make smtp_client")?;
    let body = generate_message_text(1024, 78);
    let response = MailGenParams {
        body: Some(&body),
        ..Default::default()
    }
    .send(&mut client)
    .await
    .context("send message")?;
    eprintln!("{response:?}");
    anyhow::ensure!(response.code == 250);

    daemon
        .wait_for_maildir_count(1, Duration::from_secs(10))
        .await;

    // Let's validated the sending in bogus ARC headers is caught by arc_verify
    let failed_send = MailGenParams {
        full_content: Some(
            "Subject: woot\r\n\
            ARC-Seal: i=1; a=rsa-sha256; cv=none; d=messagingengine.com; s=fm3; t=\r\n\
            \t1761818287; b=ycwa8bnZ1tzWb6H7JnfvMbAcXQarSXzeH4FF70n897IuqJjpK5\r\n\
            \tm0OEH28pRIMClrC7MyimD9LcccWGTipSLMnEpv0KmI9vR2U6XhP4H4GRjTPllIkA\r\n\
            \tiUmAlS3sBmZTgfYSA/G1EbNaT4Nze7gnls5nRJkzxr1fj8uISFwMxkbNLam4KX/7\r\n\
            \t+LzoKHUjI73wSb163y3j6iPb4haU7pxtuBoStGBlh0iZmnM+uj1akTbJnKr7VHKS\r\n\
            \tEi84Pwc6wiRDjGwR7L8WUNJ1jOUaxHs+9fBDf03DL/yRmLIuVo14l/vjN/GV/9Ho\r\n\
            \tZYblFYalGQ/0vYo06nbN1AprpO02h75DX8TA==\r\n\
            \r\nthis is not ok\r\n",
        ),
        ..Default::default()
    }
    .send(&mut client)
    .await
    .unwrap_err();
    eprintln!("{failed_send:#?}");

    let ClientError::Rejected(response) = failed_send.downcast_ref::<ClientError>().unwrap() else {
        panic!("expected ClientError::Rejected");
    };
    k9::assert_equal!(response.code, 550);
    k9::assert_equal!(response.content,
        "ARC Validation failure: The ARC Set with instance 1 is missing some of its constituent headers");

    daemon.stop_both().await.context("stop_both")?;
    println!("Stopped!");

    let mut messages = daemon.extract_maildir_messages()?;

    assert_equal!(messages.len(), 1);
    let payload = messages[0].read_data()?;

    let resolver = TestResolver::default().with_txt(
        "default._domainkey.example.com",
        "v=DKIM1; h=sha256; k=rsa; p=\
        MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAhqi4/9qc0sAty6lCkOad\
        vpFmCOLKPcMVRr4N6brrHoJvvnWxse6Umy5QbOWsT8rB5M8vwMJUtPh+Aioe5W+p\
        VHobfQVbHSd5Rd0Z6vwbU3kYg5Eds6Yt5F/hcQ9ck2QFQSc0M5ULgDbciHuvCgwj\
        zbRMCGZnFal8HO9MZte62KPaG6gqQk4V8CKW1ecbyf7o4ohuq1Tk1tj7YfBimLJJ\
        2SSGj6gOrMGTiS1bPA77np7wOKUPo7lqHO+40x6NTVvyK591t508lKqDZ2ZICz9K\
        qJ7eu+pUmppZfGKr8TP+/kPbY6WoAe02xQTb0SqDsfo0eVsXAE9PIw10Bp/QXR9R\
        yQIDAQAB",
    );

    let email = ParsedEmail::parse(String::from_utf8(payload.to_vec()).unwrap()).unwrap();
    let arc = ARC::verify(&email, &resolver).await;
    assert_eq!(arc.chain_validation_status(), ChainValidationStatus::Pass);

    Ok(())
}

use crate::kumod::KumoDaemon;

#[tokio::test]
async fn scrape_policy_counter() -> anyhow::Result<()> {
    let mut daemon = KumoDaemon::spawn_with_policy("policy-counter.lua").await?;
    let samples = daemon
        .api_client()
        .get_metrics(|m| {
            (m.name().as_str() == "policy_rule_evaluations_total"
                && m.label_is("rule", "quoted \"rule\"\npath\\"))
            .then(|| m.value())
        })
        .await?;
    daemon.stop().await?;
    assert_eq!(samples, [1.0]);
    Ok(())
}

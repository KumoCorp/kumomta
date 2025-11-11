# Integrations

This chapter is focused on partner and adjacent systems integrations.
Some of the third-party integrations could involve a separate external fee, while others are entirely FOSS (Free and Open Source Software).

We are available to provide paid professional services to assist you with any of these integrations.


```mermaid
---
config:
  theme: 'base'
  themeVariables:
    primaryColor: '#BB8'
    primaryTextColor: '#000'
    primaryBorderColor: '#C0000'
    lineColor: '#F8B229'
    secondaryColor: '#121212'
    tertiaryColor: '#F8B22f'
  kanban:
    ticketBaseUrl: 'https://docs.kumomta.com/userguide/integrations/#TICKET#'
---
kanban
  Campaign Management
    [Ongage]@{ticket: ongage}
    [Mautic Marketing Automation]@{ticket: mautic}
    [EmailElement]@{ticket: emailelement}
  [Reporting UIs]
    [Postmastery Console]@{ticket: postmastery}
    [Tatami Monitor]@{ticket: tatamimonitor}
    [Grafana Dashboard]@{ticket: grafana}
  [Data Feeds]
    [Prometheus Metrics Monitor]@{ticket: prometheus }
  [AV/AS]
    [Hornetsecurity Spam and Malware protection]@{ticket: hornetsecurity }
    [Rspamd]@{ticket: rspamd }

```


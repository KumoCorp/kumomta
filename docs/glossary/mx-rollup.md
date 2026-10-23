# MX rollup

MX rollup is grouping destination domains that share the same MX infrastructure so that queues and traffic shaping apply to the receiving system rather than to each domain name. Thousands of Google Workspace-hosted domains resolve to the same Google MXs; without rollup, a sender would open per-domain connections that collectively exceed what the provider tolerates from one IP. Rolling up by shared MX target is how serious outbound MTAs keep per-provider limits accurate.

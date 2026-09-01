# Retry schedule / exponential backoff

A retry schedule defines when deferred messages are attempted again, typically with exponentially increasing intervals until a maximum message age is reached. Retry tuning is provider-aware in practice: retrying too aggressively into a throttling provider makes blocks worse, while retrying too slowly sacrifices timely delivery.

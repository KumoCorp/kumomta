---
tags:
 - templates
---

# kumo.set_httpinject_template_cache_enabled

```lua
kumo.set_httpinject_template_cache_enabled(ENABLED)
```

{{since('dev')}}

Controls whether the [HTTP injection API](../http/kumod/api_inject_v1_post.md)
caches compiled templates.  The cache is enabled by default.

Compiling a template is relatively expensive, and it is common for many
injection requests to share the same template text.  When the cache is
enabled, the compiled form of the following fields is cached and reused by
subsequent requests that use the same template text and `template_dialect`:

* `content`, when it is a complete RFC 5322 message string
* `content.text_body`
* `content.html_body`
* `content.amp_html_body`

Header templates, including those generated from `content.subject`,
`content.from` and `content.reply_to`, are not cached, nor is content that
contains no template syntax (such as a message that was fully rendered before
it was injected, or any content using the `Static` dialect), as KumoMTA uses
that text as-is without compiling it.  Enabling or disabling the cache does
not change the content of the generated messages.

The cache is named `http_inject_template`.  It holds up to 1,000 compiled
templates by default; entries expire 10 minutes after they were compiled.
Each entry retains a copy of its template text, so the memory used by the
cache grows with both the number and the size of the distinct templates
that are injected.
Use [kumo.set_lruttl_cache_capacity](set_lruttl_cache_capacity.md) to adjust
its capacity, and the `lruttl_*` metrics, such as
[lruttl_hit_count](../metrics/kumod/lruttl_hit_count.md) and
[lruttl_cache_size](../metrics/kumod/lruttl_cache_size.md), to observe it.

```lua
kumo.on('pre_init', function()
  -- The cache is enabled by default; this disables it
  kumo.set_httpinject_template_cache_enabled(false)

  -- Alternatively, keep it enabled and increase its capacity
  -- kumo.set_lruttl_cache_capacity('http_inject_template', 5000)
end)
```

# normalize_smtp_response

```rust
pub fn normalize_smtp_response(text: &str) -> String
```

This filter normalizes an SMTP response as described in
[kumo.string.normalize_smtp_response](../string/normalize_smtp_response.md).

{% raw %}
```jinja
{{ response.content | normalize_smtp_response }}
```
{% endraw %}

# pluralize

```rust
pub fn pluralize(
    v: Value,
    singular: Option<Value>,
    plural: Option<Value>,
) -> Result<Value, Error>
```

Returns a plural suffix if the value is not 1, '1', or an object of
length 1.

By default, the plural suffix is 's' and the singular suffix is
empty (''). You can specify a singular suffix as the first argument (or
`None`, for the default). You can specify a plural suffix as the second
argument (or `None`, for the default).

{% raw %}
```jinja
{{ users|length }} user{{ users|pluralize }}.
```

```jinja
{{ entities|length }} entit{{ entities|pluralize("y", "ies") }}.
```

```jinja
{{ platypuses|length }} platypus{{ platypuses|pluralize(None, "es") }}.
```
{% endraw %}

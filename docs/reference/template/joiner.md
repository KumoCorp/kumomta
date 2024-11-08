```rust
pub fn joiner(sep: Option<Value>) -> Value
```

A tiny helper that can be used to “join” multiple sections.  A
joiner is passed a string and will return that string every time
it’s called, except the first time (in which case it returns an
empty string). You can use this to join things:

{% raw %}
```jinja
{% set pipe = joiner("|") %}
{% if categories %} {{ pipe() }}
Categories: {{ categories|join(", ") }}
{% endif %}
{% if author %} {{ pipe() }}
Author: {{ author() }}
{% endif %}
{% if can_edit %} {{ pipe() }}
<a href="?action=edit">Edit</a>
{% endif %}
```
{% endraw %}

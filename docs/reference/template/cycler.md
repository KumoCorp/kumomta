```rust
pub fn cycler(items: Vec<Value>) -> Result<Value, Error>
```

Returns a cycler.

Similar to `loop.cycle`, but can be used outside loops or across
multiple loops. For example, render a list of folders and files in a
list, alternating giving them "odd" and "even" classes.

{% raw %}
```jinja
{% set row_class = cycler("odd", "even") %}
<ul class="browser">
{% for folder in folders %}
  <li class="folder {{ row_class.next() }}">{{ folder }}
{% endfor %}
{% for file in files %}
  <li class="file {{ row_class.next() }}">{{ file }}
{% endfor %}
</ul>
```
{% endraw %}

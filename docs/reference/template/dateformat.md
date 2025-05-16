# dateformat

```rust
pub fn dateformat(
    state: &State<'_, '_>,
    value: Value,
    kwargs: Kwargs,
) -> Result<String, Error>
```

Formats a timestamp as date.

The value needs to be a unix timestamp, or a parsable string (ISO 8601) or a
format supported by `chrono` or `time`.  If the string does not include time
information, then timezone adjustments are not performed.

The filter accepts two keyword arguments (`format` and `tz`) to influence the format
and the timezone.  The default format is `"medium"`.  The defaults for these keyword
arguments are taken from two global variables in the template context: `DATE_FORMAT`
and `TIMEZONE`.  If the timezone is set to `"original"` or is not configured, then
the timezone of the value is retained.  Otherwise the timezone is the name of a
timezone [from the database](https://en.wikipedia.org/wiki/List_of_tz_database_time_zones).

{% raw %}
```jinja
{{ value|dateformat }}
```

```jinja
{{ value|dateformat(format="short") }}
```

```jinja
{{ value|dateformat(format="short", tz="Europe/Vienna") }}
```
{% endraw %}

This filter currently uses the `time` crate to format dates and uses the format
string specification of that crate in version 2.  For more information read the
[Format description documentation](https://time-rs.github.io/book/api/format-description.html).
Additionally some special formats are supported:

* `short`: a short date format (`2023-06-24`)
* `medium`: a medium length date format (`Jun 24 2023`)
* `long`: a longer date format (`June 24 2023`)
* `full`: a full date format (`Saturday, June 24 2023`)

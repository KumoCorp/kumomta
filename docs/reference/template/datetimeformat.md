# datetimeformat

```rust
pub fn datetimeformat(
    state: &State<'_, '_>,
    value: Value,
    kwargs: Kwargs,
) -> Result<String, Error>
```

Formats a timestamp as date and time.

The value needs to be a unix timestamp, or a parsable string (ISO 8601) or a
format supported by `chrono` or `time`.

The filter accepts two keyword arguments (`format` and `tz`) to influence the format
and the timezone.  The default format is `"medium"`.  The defaults for these keyword
arguments are taken from two global variables in the template context: `DATETIME_FORMAT`
and `TIMEZONE`.  If the timezone is set to `"original"` or is not configured, then
the timezone of the value is retained.  Otherwise the timezone is the name of a
timezone [from the database](https://en.wikipedia.org/wiki/List_of_tz_database_time_zones).

{% raw %}
```jinja
{{ value|datetimeformat }}
```

```jinja
{{ value|datetimeformat(format="short") }}
```

```jinja
{{ value|datetimeformat(format="short", tz="Europe/Vienna") }}
```
{% endraw %}

This filter currently uses the `time` crate to format dates and uses the format
string specification of that crate in version 2.  For more information read the
[Format description documentation](https://time-rs.github.io/book/api/format-description.html).
Additionally some special formats are supported:

* `short`: a short date and time format (`2023-06-24 16:37`)
* `medium`: a medium length date and time format (`Jun 24 2023 16:37`)
* `long`: a longer date and time format (`June 24 2023 16:37:22`)
* `full`: a full date and time format (`Saturday, June 24 2023 16:37:22`)
* `unix`: a unix timestamp in seconds only (`1687624642`)
* `iso`: date and time in iso format (`2023-06-24T16:37:22+00:00`)

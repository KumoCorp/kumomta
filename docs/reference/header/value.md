# value

```lua
local value = header.value
```

{{since('dev')}}

Depending on the header name, returns one of the structured representations of the header value.

If there is no defined header name and structured representation, the fallback
behavior is to return the same value that you'd see if you accessed
[header.unstructured](unstructured.md).

!!! note
    For the sake of forwards compatibility, it is possible that we will add
    support for names and structured representations in future versions.  In that
    scenario, if you were using `header.value` and relying on it returning
    the same value as `header.unstructured` then your logic may be broken
    by upgrading.

    With that in mind, we recommend that you explicitly use `header.unstructured`
    in your code for headers that are not included in the table below, rather
    than relying on the fallback behavior.

## Supported Header Names and Types

If the `Since` column is blank, it is assumed to be since the inception of `header.value`,
which is the version shown at the top of this page.

|Name|Equivalent field accessor|Since|
|----|----|-----|
|Authentication-Results|authentication_results||
|Bcc|address_list||
|Cc|address_list||
|Comments|unstructured||
|Content-Disposition|mime_params||
|Content-Id|message_id||
|Content-Transfer-Encoding|mime_params||
|Content-Type|mime_params||
|From|mailbox_list||
|Message-ID|message_id||
|Mime-Version|unstructured||
|References|message_id_list||
|Reply-To|address_list||
|Resent-Bcc|address_list||
|Resent-Cc|address_list||
|Resent-From|mailbox_list||
|Resent-Sender|mailbox||
|Resent-To|address_list||
|Sender|mailbox||
|Subject|unstructured||
|To|address_list||


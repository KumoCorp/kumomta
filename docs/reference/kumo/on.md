# `kumo.on(EVENT, FUNCTION)`

Register a handler for a named event.

`EVENT` can be any string, and `FUNCTION` can be any lua function or closure.

Only the most recently registered function for a given event will be used.

The possible events are listed in the [events reference](../events/index.md).

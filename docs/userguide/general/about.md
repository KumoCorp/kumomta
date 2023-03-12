# About This Manual

This is the Reference Manual for the KumoMTA SMTP server, version 1.0, through release 1.0. For license information, see the Legal Notices.

Because this manual serves as a reference, it does not provide general instruction on SMTP or email infrastructure concepts. It also does not teach you how to use your operating system or command-line interpreter.

The KumoMTA Software is under constant development, and this Manual is updated frequently as well.

If you have questions about using KumoMTA, community support is available in the [Forum](https://forum.kumomta.com) and the [Community Discord](https://kumomta.com/discord).

## Typographical and Syntax Conventions

This manual uses certain typographical conventions:

!!! note
    This is a noteworthy section

!!! warning
    This indicates a warning

!!! danger
    This indicates something that can have dangerous consequences

`Text in this style` indicates input that you type in examples.

**`Text in this style`** indicates the names of executable programs and scripts, examples being **`kumod`** (the KumoMTA server executable).

_`Text in this style`_ is used for variable input for which you should substitute a value of your own choosing.

_Text in this style_ is used for emphasis.

**Text in this style** is used in table headings and to convey especially strong emphasis.

`Text in this style` is used to indicate a program option that affects how the program is executed, or that supplies information that is needed for the program to function in a certain way. _Example_: “The `--policy` option tells the **`kumod`** server the path to the initial policy file to execute on startup”.

File names and directory names are written like this: “The `simple-policy.lua` file is located in the `/etc/kumod` directory.”

Character sequences are written like this: “To specify a wildcard, use the `‘%’` character.”

When commands or statements are prefixed by a prompt, we use these:

```text
$> type a command here
#> type a command as root here
kumo> type a mysql statement here
```

Commands are issued in your command interpreter. On Unix, this is typically a program such as sh, csh, or bash.

```admonish
When you enter a command or statement shown in an example, do not type the prompt shown in the example.
```

In syntax descriptions, square brackets (“\[” and “\]”) indicate optional words or clauses. For example, in the following statement, --user is optional:

**`kumod`**_`--policy simple-policy.lua [--user] someuser`_

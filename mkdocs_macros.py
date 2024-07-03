import json
import glob
import subprocess

# https://mkdocs-macros-plugin.readthedocs.io/en/latest/macros/
def define_env(env):
    @env.macro
    # Set indent=True when you want to define a box containing version-specific info.
    #
    # Set inline=True when you want to define a simple inline version indicator,
    # such as when emitting information into a table row.
    def since(vers, indent=False, inline=False):

        scope = "section"
        expanded = ""
        expander = "???"
        rule = ""
        if indent:
            scope = "outlined box"
            expander = "!!!"
            rule = "    <hr/>"

        if vers == "dev":
            first_line = "*Since: Dev Builds Only*"
            if scope != "section":
                expanded = "+"
            blurb = f"""
    *The functionality described in this {scope} requires a dev build of KumoMTA.
    You can obtain a dev build by following the instructions in the
    [Installation](/userguide/installation/linux/) section.*
"""
        else:
            first_line = f"*Since: Version {vers}*"
            blurb = f"""
    *The functionality described in this {scope} requires version {vers} of KumoMTA,
    or a more recent version.*
"""

        if inline:
            return f"({first_line})"

        # If we're not expandable, don't emit the expanded marker
        if expander == "!!!":
            expanded = ""

        return f"""
{expander}{expanded} info "{first_line}"
{blurb}
{rule}
"""

    @env.macro
    def toml_data(caller):
        toml = caller()

        second_line = toml.split('\n')[1]
        indentation = len(second_line) - len(second_line.lstrip())
        indent = " " * indentation
        tab_indent = " " * (indentation + 4)

        def remove_indent(s):
            result = []
            for line in s.split('\n'):
                result.append(line[indentation:])
            return "\n".join(result)

        def apply_indent(s):
            result = []
            for line in s.split('\n'):
                result.append(tab_indent + line)
            return "\n".join(result)

        toml = remove_indent(toml)
        adjusted_toml = apply_indent(toml)

        p = subprocess.Popen(["/util/toml2jsonc"],
                encoding='utf-8',
                stdin=subprocess.PIPE,
                stderr=subprocess.PIPE,
                stdout=subprocess.PIPE)
        json, err = p.communicate(toml)

        if err:
            err = apply_indent(err)
            err = f"""{indent}!!! error
{tab_indent}```
{err}
{tab_indent}```
"""

        adjusted_json = apply_indent(json)

        result = f"""
{indent}=== \"TOML\"

{tab_indent}```toml
{adjusted_toml}
{tab_indent}```

{indent}=== \"JSON\"
{tab_indent}```json
{adjusted_json}
{tab_indent}```

{err}
"""
        # print(result)
        return result

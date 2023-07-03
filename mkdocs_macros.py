import json

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
    [Installation](/userguide/installation/linux.md) section.*
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

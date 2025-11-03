import json
import glob
import os
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

            # Determine the relative path traversal to the root,
            # so that we can emit the link to the install page

            page_url = env.page.url
            # Annoying dance because we don't know if we are `foo/index.md` or `foo.md`
            # the url for both is `/foo/` but the number of `../` we need to emit for
            # them is different.  Make an educated guess about which we are processing.
            # It would be great if we knew the source path from the page object,
            # but I don't see a public way to access that.
            if page_url.endswith('/'):
                page_url = page_url[:-1]

            index_url = page_url + "/index.md"
            direct_url = page_url + ".md"

            if os.path.exists('docs/' + index_url):
                # Looks like we have the foo/index.md variant
                page_url = index_url
            else:
                # foo.md variant
                page_url = direct_url

            # Compute the appropriate amount of ../ to reach the root.
            # Why not simply use an absolute link? Because mkdocs doesn't
            # support it and will not generate the appropriate link.
            levels = len(page_url.split('/')) - 1
            rel_root = "../" * levels
            blurb = f"""
    *The functionality described in this {scope} requires a dev build of KumoMTA.
    You can obtain a dev build by following the instructions in the
    [Installation]({rel_root}userguide/installation/linux.md) section.*
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

    # This macro is for use by docs/reference/kumo/set_lruttl_cache_capacity.md
    # It expects that the json file referenced below be updated in docs/build.sh
    # to reflect the current set of cache definitions
    @env.macro
    def lruttl_defs():
        with open('docs/reference/lruttl-caches.json') as f:
            defs = json.load(f)

            info = "|Name|Capacity|Comment|\n|-|-|-|\n"
            for d in defs:
                name = d['name']
                capacity = d['capacity']
                doc = d.get('doc', '')
                if doc is None:
                    doc = ''
                info = info + f"|{name}|{capacity}|{doc}|\n"

            return info

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

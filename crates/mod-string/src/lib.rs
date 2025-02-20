use config::get_or_create_sub_module;
use mlua::Lua;

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let string_mod = get_or_create_sub_module(lua, "string")?;

    string_mod.set(
        "replace",
        lua.create_function(move |_, (s, from, to): (String, String, String)| {
            Ok(s.replace(&from, &to))
        })?,
    )?;

    string_mod.set(
        "replacen",
        lua.create_function(
            move |_, (s, from, to, count): (String, String, String, usize)| {
                Ok(s.replacen(&from, &to, count))
            },
        )?,
    )?;

    string_mod.set(
        "rsplit",
        lua.create_function(move |_, (s, pattern): (String, String)| {
            Ok(s.rsplit(&pattern)
                .map(|s| s.to_string())
                .collect::<Vec<String>>())
        })?,
    )?;

    string_mod.set(
        "rsplitn",
        lua.create_function(move |_, (s, limit, pattern): (String, usize, String)| {
            Ok(s.rsplitn(limit, &pattern)
                .map(|s| s.to_string())
                .collect::<Vec<String>>())
        })?,
    )?;

    string_mod.set(
        "split",
        lua.create_function(move |_, (s, pattern): (String, String)| {
            Ok(s.split(&pattern)
                .map(|s| s.to_string())
                .collect::<Vec<String>>())
        })?,
    )?;

    string_mod.set(
        "splitn",
        lua.create_function(move |_, (s, limit, pattern): (String, usize, String)| {
            Ok(s.splitn(limit, &pattern)
                .map(|s| s.to_string())
                .collect::<Vec<String>>())
        })?,
    )?;

    string_mod.set(
        "split_whitespace",
        lua.create_function(move |_, s: String| {
            Ok(s.split_whitespace()
                .map(|s| s.to_string())
                .collect::<Vec<String>>())
        })?,
    )?;

    string_mod.set(
        "split_ascii_whitespace",
        lua.create_function(move |_, s: String| {
            Ok(s.split_ascii_whitespace()
                .map(|s| s.to_string())
                .collect::<Vec<String>>())
        })?,
    )?;

    string_mod.set(
        "trim",
        lua.create_function(move |_, s: String| Ok(s.trim().to_string()))?,
    )?;
    string_mod.set(
        "trim_end",
        lua.create_function(move |_, s: String| Ok(s.trim_end().to_string()))?,
    )?;
    string_mod.set(
        "trim_start",
        lua.create_function(move |_, s: String| Ok(s.trim_start().to_string()))?,
    )?;

    string_mod.set(
        "psl_domain",
        lua.create_function(move |_, s: String| Ok(psl::domain_str(&s).map(|s| s.to_string())))?,
    )?;

    string_mod.set(
        "psl_suffix",
        lua.create_function(move |_, s: String| Ok(psl::suffix_str(&s).map(|s| s.to_string())))?,
    )?;

    string_mod.set(
        "eval_template",
        lua.create_function(
            move |_, (name, template, context): (String, String, mlua::Value)| {
                let engine = kumo_template::TemplateEngine::new();
                engine
                    .render(&name, &template, context)
                    .map_err(config::any_err)
            },
        )?,
    )?;

    Ok(())
}

use config::{get_or_create_sub_module, serialize_options};
use file_type::FileType;
use mlua::{IntoLua, Lua, LuaSerdeExt};
use serde::Serialize;

#[derive(Serialize)]
struct FileTypeResult {
    pub name: String,
    pub extensions: Vec<String>,
    pub media_types: Vec<String>,
}

impl Into<FileTypeResult> for &FileType {
    fn into(self) -> FileTypeResult {
        FileTypeResult {
            name: self.name().to_string(),
            extensions: self
                .extensions()
                .into_iter()
                .map(|s| s.to_string())
                .collect(),
            media_types: self
                .media_types()
                .into_iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }
}

impl IntoLua for FileTypeResult {
    fn into_lua(self, lua: &Lua) -> mlua::Result<mlua::Value> {
        lua.to_value_with(&self, serialize_options())
    }
}

fn ft_to_lua(ft: &FileType, lua: &Lua) -> mlua::Result<mlua::Value> {
    let res: FileTypeResult = ft.into();
    res.into_lua(lua)
}

fn fts_to_lua(ft: &[&FileType], lua: &Lua) -> mlua::Result<mlua::Value> {
    let fts: Vec<FileTypeResult> = ft.into_iter().map(|&ft| ft.into()).collect();
    lua.to_value_with(&fts, serialize_options())
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let ft_mod = get_or_create_sub_module(lua, "file_type")?;

    ft_mod.set(
        "from_bytes",
        lua.create_function(move |lua, bytes: mlua::String| {
            let ft = file_type::FileType::from_bytes(bytes.as_bytes());
            ft_to_lua(&ft, lua)
        })?,
    )?;

    ft_mod.set(
        "from_extension",
        lua.create_function(move |lua, extension: String| {
            fts_to_lua(FileType::from_extension(&extension), lua)
        })?,
    )?;

    ft_mod.set(
        "from_media_type",
        lua.create_function(move |lua, mt: String| {
            fts_to_lua(FileType::from_media_type(&mt), lua)
        })?,
    )?;
    Ok(())
}

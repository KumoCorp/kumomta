use anyhow::Context;
use config::{any_err, from_lua_value, get_or_create_module};
use mlua::{Lua, LuaSerdeExt, MultiValue, UserData, UserDataMethods, Value};
use serde_json::{Map, Value as JsonValue};
use sqlite::{Connection, ConnectionWithFullMutex, ParameterIndex, State, Statement, Type};
use std::sync::Arc;

fn bind_param<I: ParameterIndex>(
    stmt: &mut Statement,
    index: I,
    value: &JsonValue,
) -> anyhow::Result<()> {
    Ok(match value {
        JsonValue::Null => stmt.bind((index, ()))?,
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                stmt.bind((index, i))?
            } else if let Some(f) = n.as_f64() {
                stmt.bind((index, f))?
            } else {
                anyhow::bail!("numeric value {n} is out of range for sqlite");
            }
        }
        JsonValue::String(s) => stmt.bind((index, s.as_str()))?,
        _ => {
            anyhow::bail!("only numbers, strings and nil can be passed as parameter values");
        }
    })
}

fn params_to_json<'lua>(lua: &'lua Lua, mut params: MultiValue) -> mlua::Result<JsonValue> {
    match params.len() {
        0 => Ok(JsonValue::Null),
        1 => {
            let param = params
                .pop_front()
                .expect("we checked and we have at least one");
            let param: JsonValue = from_lua_value(lua, param)?;
            Ok(param)
        }
        _ => {
            let mut result = vec![];
            for p in params {
                let p: JsonValue = from_lua_value(lua, p)?;
                result.push(p);
            }
            Ok(JsonValue::Array(result))
        }
    }
}

fn bind_params(stmt: &mut Statement, params: &JsonValue) -> anyhow::Result<()> {
    match params {
        JsonValue::Object(obj) => {
            for (name, value) in obj.iter() {
                bind_param(stmt, format!(":{name}").as_str(), &value)
                    .with_context(|| format!("binding parameter :{name} with value {value:?}"))?;
            }
            Ok(())
        }
        JsonValue::Array(arr) => {
            for (i, value) in arr.iter().enumerate() {
                // Parameter indices are 1-based
                let i = i + 1;
                bind_param(stmt, i, &value)
                    .with_context(|| format!("binding parameter {i} with value {value:?}"))?;
            }
            Ok(())
        }
        JsonValue::Null => Ok(()),
        p => bind_param(stmt, 1, &p)
            .with_context(|| format!("binding sole parameter with value {p:?}")),
    }
}

fn get_column(stmt: &Statement, index: usize) -> anyhow::Result<JsonValue> {
    match stmt.column_type(index)? {
        Type::Binary | Type::String => {
            let s: String = stmt.read(index).map_err(any_err)?;
            Ok(s.into())
        }
        Type::Integer => {
            let i: i64 = stmt.read(index)?;
            Ok(i.into())
        }
        Type::Float => {
            let f: f64 = stmt.read(index)?;
            Ok(f.into())
        }
        Type::Null => Ok(JsonValue::Null),
    }
}

#[derive(Clone)]
struct Conn(Arc<ConnectionWithFullMutex>);

impl Conn {
    // Sqlite queries are blocking and we cannot safely block an async
    // function, so we push the work over to this blocking function
    // via spawn_blocking.
    fn execute(&self, sql: String, params: JsonValue) -> anyhow::Result<JsonValue> {
        let mut stmt = self.0.prepare(&sql)?;
        bind_params(&mut stmt, &params)
            .with_context(|| format!("bind parameters {params:?} in query `{sql}'"))?;

        let state = stmt.next()?;
        if state == State::Done && stmt.column_count() == 0 {
            // Query cannot return any rows, so we'll return
            // the affected row count
            return Ok(self.0.change_count().into());
        }

        let mut table = vec![];
        // Query has rows. Decide whether we are returning a simple
        // array of single column results, or an array of objects
        let col_count = stmt.column_count();
        if col_count == 1 {
            loop {
                let value = get_column(&mut stmt, 0)?;
                table.push(value);

                if stmt.next()? == State::Done {
                    break;
                }
            }
        } else {
            loop {
                let mut obj = Map::new();
                let col_names = stmt.column_names();
                for i in 0..col_count {
                    let value = get_column(&stmt, i)?;
                    obj.insert(col_names[i].to_string(), value);
                }
                table.push(JsonValue::Object(obj));

                if stmt.next()? == State::Done {
                    break;
                }
            }
        }

        Ok(JsonValue::Array(table))
    }
}

impl UserData for Conn {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_async_method(
            "execute",
            |lua, this, (sql, params): (String, MultiValue)| async move {
                let json_params = params_to_json(lua, params)?;
                let result: JsonValue = tokio::task::Builder::new()
                    .name(&format!("sqlite {sql}"))
                    .spawn_blocking(move || -> anyhow::Result<JsonValue> {
                        this.execute(sql, json_params)
                    })
                    .map_err(any_err)?
                    .await
                    .map_err(any_err)?
                    .map_err(any_err)?;

                let result: Value = lua
                    .to_value_with(&result, config::serialize_options())
                    .map_err(any_err)?;
                Ok(result)
            },
        );
    }
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let sqlite_mod = get_or_create_module(lua, "sqlite")?;

    sqlite_mod.set(
        "open",
        lua.create_function(move |_, (path, busy_timeout): (String, Option<usize>)| {
            let mut db = Connection::open_with_full_mutex(path).map_err(any_err)?;
            db.set_busy_timeout(busy_timeout.unwrap_or(500))
                .map_err(any_err)?;
            Ok(Conn(Arc::new(db)))
        })?,
    )?;

    Ok(())
}

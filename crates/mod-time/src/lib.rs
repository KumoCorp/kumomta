use chrono::format::StrftimeItems;
use chrono::{DateTime, Datelike, LocalResult, TimeZone, Timelike, Utc};
use config::{any_err, get_or_create_module, get_or_create_sub_module};
use humantime::format_duration;
use mlua::{
    FromLua, IntoLua, Lua, MetaMethod, UserData, UserDataFields, UserDataMethods, UserDataRef,
};
use prometheus::{HistogramTimer, HistogramVec};
use std::sync::LazyLock;
use tokio::time::Duration;

static LATENCY_HIST: LazyLock<HistogramVec> = LazyLock::new(|| {
    prometheus::register_histogram_vec!(
        "user_lua_latency",
        "how long something user-defined took to run in your lua policy",
        &["label"]
    )
    .unwrap()
});

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let kumo_mod = get_or_create_module(lua, "kumo")?;
    let time_mod = get_or_create_sub_module(lua, "time")?;

    let sleep_fn = lua.create_async_function(sleep)?;
    kumo_mod.set("sleep", sleep_fn.clone())?;
    time_mod.set("sleep", sleep_fn)?;

    time_mod.set("start_timer", lua.create_function(Timer::start)?)?;
    time_mod.set("now", lua.create_function(Time::now)?)?;
    time_mod.set(
        "from_unix_timestamp",
        lua.create_function(Time::from_unix_timestamp)?,
    )?;
    time_mod.set("with_ymd_hms", lua.create_function(Time::with_ymd_hms)?)?;
    time_mod.set("parse_rfc3339", lua.create_function(Time::parse_rfc3339)?)?;
    time_mod.set("parse_rfc2822", lua.create_function(Time::parse_rfc2822)?)?;
    time_mod.set(
        "parse_duration",
        lua.create_function(TimeDelta::parse_duration)?,
    )?;

    Ok(())
}

/// A Timer keeps track of the time since it was started,
/// and will record the duration until its done method is
/// called, or the __close metamethod is invoked.
struct Timer {
    timer: Option<HistogramTimer>,
}

impl Drop for Timer {
    fn drop(&mut self) {
        // We might be called some time after the code is done due
        // to gc delays and pooling. We don't want the default
        // Drop impl for HistogramTimer to record in that case:
        // we will only report when our done method is explicitly
        // called in lua
        if let Some(timer) = self.timer.take() {
            timer.stop_and_discard();
        }
    }
}

impl Timer {
    fn start(_lua: &Lua, name: String) -> mlua::Result<Self> {
        let timer = LATENCY_HIST
            .get_metric_with_label_values(&[&name])
            .expect("to get histo")
            .start_timer();
        Ok(Self { timer: Some(timer) })
    }

    fn done(_lua: &Lua, this: &mut Self, _: ()) -> mlua::Result<Option<f64>> {
        Ok(this.timer.take().map(|timer| timer.stop_and_record()))
    }
}

impl UserData for Timer {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method_mut("done", Self::done);
        methods.add_meta_method_mut(MetaMethod::Close, Self::done);
    }
}

async fn sleep(_lua: Lua, seconds: f64) -> mlua::Result<()> {
    tokio::time::sleep(Duration::from_secs_f64(seconds)).await;
    Ok(())
}

pub struct Time {
    t: DateTime<Utc>,
}

impl From<DateTime<Utc>> for Time {
    fn from(t: DateTime<Utc>) -> Self {
        Self { t }
    }
}

#[derive(Clone, Copy)]
pub struct TimeDelta(chrono::TimeDelta);

impl From<chrono::TimeDelta> for TimeDelta {
    fn from(delta: chrono::TimeDelta) -> Self {
        Self(delta)
    }
}

impl From<TimeDelta> for chrono::TimeDelta {
    fn from(delta: TimeDelta) -> Self {
        delta.0
    }
}

impl Time {
    fn now(_lua: &Lua, _: ()) -> mlua::Result<Self> {
        Ok(Self { t: Utc::now() })
    }

    fn from_unix_timestamp(_lua: &Lua, seconds: mlua::Value) -> mlua::Result<Self> {
        let dt = match seconds {
            mlua::Value::Integer(i) => DateTime::from_timestamp_secs(i),
            mlua::Value::Number(n) => {
                let seconds = n.trunc() as i64;
                let nanos = (n.fract() * 1e9) as u32;
                DateTime::from_timestamp(seconds, nanos)
            }
            _ => {
                return Err(mlua::Error::external(
                    "timestamp must be either an integer or floating point number of seconds",
                ));
            }
        };
        Ok(Self {
            t: dt
                .ok_or_else(|| mlua::Error::external("invalid timestamp"))?
                .to_utc(),
        })
    }

    fn parse_rfc3339(_lua: &Lua, spec: String) -> mlua::Result<Self> {
        Ok(Self {
            t: DateTime::parse_from_rfc3339(&spec)
                .map_err(any_err)?
                .to_utc(),
        })
    }

    fn parse_rfc2822(_lua: &Lua, spec: String) -> mlua::Result<Self> {
        Ok(Self {
            t: DateTime::parse_from_rfc2822(&spec)
                .map_err(any_err)?
                .to_utc(),
        })
    }

    fn with_ymd_hms(
        _lua: &Lua,
        (year, month, day, h, m, s): (i32, u32, u32, u32, u32, u32),
    ) -> mlua::Result<Self> {
        match Utc.with_ymd_and_hms(year, month, day, h, m, s) {
            LocalResult::Single(t) => Ok(Self { t }),
            // I'm not sure that these LocalResult variants are possible
            // with UTC, but I'm filling them in with a reasonable error case.
            LocalResult::Ambiguous(_, _) => Err(mlua::Error::external(
                "time cannot be represented unambiguously due to a fold in local time",
            )),
            LocalResult::None => Err(mlua::Error::external(
                "time cannot be represented due to a gap in local time",
            )),
        }
    }
}

impl UserData for Time {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::ToString, |_, this, _: ()| {
            Ok(this.t.to_string())
        });
        methods.add_meta_method(MetaMethod::Eq, |_, this, other: UserDataRef<Time>| {
            Ok(this.t.eq(&other.t))
        });
        methods.add_method("format", |_, this, fmt: String| {
            let items = StrftimeItems::new_lenient(&fmt).parse().map_err(any_err)?;
            Ok(this
                .t
                .format_with_items(items.as_slice().iter())
                .to_string())
        });
        methods.add_meta_method(
            MetaMethod::Sub,
            |lua, this, value: mlua::Value| match UserDataRef::<Time>::from_lua(value.clone(), lua)
            {
                Ok(time) => TimeDelta(this.t.signed_duration_since(time.t)).into_lua(lua),
                Err(err1) => match UserDataRef::<TimeDelta>::from_lua(value, lua) {
                    Ok(delta) => Time {
                        t: this
                            .t
                            .checked_sub_signed(delta.0)
                            .ok_or_else(|| mlua::Error::external("time would overflow"))?,
                    }
                    .into_lua(lua),
                    Err(err2) => Err(mlua::Error::external(format!(
                        "could not represent argument as \
                         either Time ({err1:#}) or TimeDelta ({err2:#}"
                    ))),
                },
            },
        );
        methods.add_meta_method(MetaMethod::Add, |_, this, delta: UserDataRef<TimeDelta>| {
            Ok(Time {
                t: this
                    .t
                    .checked_add_signed(delta.0)
                    .ok_or_else(|| mlua::Error::external("time would overflow"))?,
            })
        });
    }

    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("year", |_, this| Ok(this.t.year()));
        fields.add_field_method_get("month", |_, this| Ok(this.t.month()));
        fields.add_field_method_get("day", |_, this| Ok(this.t.day()));
        fields.add_field_method_get("hour", |_, this| Ok(this.t.hour()));
        fields.add_field_method_get("minute", |_, this| Ok(this.t.minute()));
        fields.add_field_method_get("second", |_, this| Ok(this.t.second()));
        fields.add_field_method_get("unix_timestamp", |_, this| Ok(this.t.timestamp()));
        fields.add_field_method_get("unix_timestamp_millis", |_, this| {
            Ok(this.t.timestamp_millis())
        });
        fields.add_field_method_get("rfc2822", |_, this| Ok(this.t.to_rfc2822()));
        fields.add_field_method_get("rfc3339", |_, this| Ok(this.t.to_rfc3339()));
        fields.add_field_method_get("elapsed", |_, this| {
            let now = Utc::now();
            Ok(TimeDelta(now.signed_duration_since(this.t)))
        });
    }
}

impl TimeDelta {
    fn parse_duration(_lua: &Lua, value: mlua::Value) -> mlua::Result<Self> {
        let delta = match value {
            mlua::Value::Integer(seconds) => chrono::TimeDelta::try_seconds(seconds)
                .ok_or_else(|| mlua::Error::external("seconds out of range"))?,
            mlua::Value::Number(n) => {
                let d = Duration::from_secs_f64(n);
                chrono::TimeDelta::from_std(d).map_err(any_err)?
            }
            mlua::Value::String(s) => {
                let s = s.to_str()?;
                let d = humantime::parse_duration(&s).map_err(any_err)?;
                chrono::TimeDelta::from_std(d).map_err(any_err)?
            }
            _ => return Err(mlua::Error::external("invalid duration value")),
        };

        Ok(TimeDelta(delta))
    }
}

impl UserData for TimeDelta {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(MetaMethod::ToString, |_, this, _: ()| {
            Ok(format_duration(this.0.to_std().map_err(any_err)?).to_string())
        });
        methods.add_meta_method(MetaMethod::Eq, |_, this, other: UserDataRef<TimeDelta>| {
            Ok(this.0.eq(&other.0))
        });
        methods.add_meta_method(MetaMethod::Sub, |_, this, other: UserDataRef<TimeDelta>| {
            Ok(TimeDelta(this.0.checked_sub(&other.0).ok_or_else(
                || mlua::Error::external("time would overflow"),
            )?))
        });
        methods.add_meta_method(MetaMethod::Add, |_, this, other: UserDataRef<TimeDelta>| {
            Ok(TimeDelta(this.0.checked_add(&other.0).ok_or_else(
                || mlua::Error::external("time would overflow"),
            )?))
        });
    }

    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("seconds", |_, this| Ok(this.0.as_seconds_f64()));
        fields.add_field_method_get("nanoseconds", |_, this| Ok(this.0.num_nanoseconds()));
        fields.add_field_method_get("milliseconds", |_, this| Ok(this.0.num_milliseconds()));
        fields.add_field_method_get("microseconds", |_, this| Ok(this.0.num_microseconds()));
        fields.add_field_method_get("human", |_, this| {
            Ok(format_duration(this.0.to_std().map_err(any_err)?).to_string())
        });
    }
}

use config::{get_or_create_module, get_or_create_sub_module};
use mlua::{Lua, MetaMethod, UserData, UserDataMethods};
use prometheus::{HistogramTimer, HistogramVec};
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};
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
    time_mod.set("epoch_millis", lua.create_function(epoch_millis)?)?;
    time_mod.set("since_millis", lua.create_function(since_millis)?)?;

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

fn epoch_millis(_lua: &Lua, _: ()) -> mlua::Result<u128> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| mlua::Error::RuntimeError(format!("Failed to get epoch millis: {}", e)))?;
    Ok(now.as_millis())
}

fn since_millis(_lua: &Lua, epoch_millis: u128) -> mlua::Result<i128> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| mlua::Error::RuntimeError(format!("Failed to get epoch millis: {}", e)))?
        .as_millis() as i128;
    Ok(now - epoch_millis as i128)
}

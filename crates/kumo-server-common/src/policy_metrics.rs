use config::{any_err, get_or_create_sub_module, SerdeWrappedValue};
use mlua::{FromLua, Lua, UserData, UserDataMethods, Value};
use parking_lot::{Mutex, RwLock};
use prometheus::core::Desc;
use prometheus::{register_int_counter, IntCounter, Opts};
use serde::Deserialize;
use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, LazyLock, Once};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CounterParams {
    name: String,
    help: Option<String>,
    #[serde(default, deserialize_with = "deserialize_label_names")]
    labels: Vec<String>,
}

fn deserialize_label_names<'de, D: serde::Deserializer<'de>>(
    de: D,
) -> Result<Vec<String>, D::Error> {
    use serde::de::Error;

    // Deserializing a Lua table as Vec would silently ignore its non-sequence keys.
    let expected = "labels must be a list of strings indexed from 1 without gaps";
    let entries =
        BTreeMap::<usize, String>::deserialize(de).map_err(|_| D::Error::custom(expected))?;
    if entries.keys().copied().ne(1..=entries.len()) {
        return Err(D::Error::custom(expected));
    }
    Ok(entries.into_values().collect())
}

type Labels = BTreeMap<String, String>;

struct Family {
    desc: Desc,
    series: RwLock<HashMap<Labels, IntCounter>>,
    warn_help: Once,
    warn_labels: Once,
}

enum LuaCounter {
    Active(Arc<Family>),
    Disabled,
}

impl UserData for LuaCounter {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("inc", |lua, this, (labels, by): (Option<Labels>, Value)| {
            let labels = labels.unwrap_or_default();
            let by = match by {
                Value::Nil => 1,
                Value::Integer(n) if n >= 0 => n as u64,
                Value::Number(n) if n >= 0.0 && n.fract() == 0.0 => u64::from_lua(by, lua)?,
                _ => return Err(any_err("counter increment must be a nonnegative integer")),
            };
            let Self::Active(family) = this else {
                return Ok(());
            };
            let Family { desc, series, .. } = family.as_ref();
            if let Some(counter) = series.read().get(&labels) {
                counter.inc_by(by);
                return Ok(());
            }
            if desc.variable_labels.iter().ne(labels.keys()) {
                return Err(any_err(format!(
                    "{} expects labels {:?}, got {:?}",
                    desc.fq_name,
                    desc.variable_labels,
                    labels.keys()
                )));
            }
            // Use the full label map as the series key. prometheus 0.14's
            // IntCounterVec hashes concatenated values: (a, bc) aliases (ab, c).
            let mut series = series.write();
            let counter = match series.entry(labels) {
                Entry::Occupied(entry) => entry.into_mut(),
                Entry::Vacant(entry) => {
                    let opts = Opts::new(&desc.fq_name, &desc.help)
                        .const_labels(entry.key().clone().into_iter().collect());
                    entry.insert(register_int_counter!(opts).map_err(any_err)?)
                }
            };
            counter.inc_by(by);
            Ok(())
        });
    }
}

fn define_counter(
    _: &Lua,
    SerdeWrappedValue(mut params): SerdeWrappedValue<CounterParams>,
) -> mlua::Result<LuaCounter> {
    static FAMILIES: LazyLock<Mutex<HashMap<String, Arc<Family>>>> =
        LazyLock::new(Default::default);
    let name = format!("policy_{}", params.name);
    let help = params.help.filter(|s| !s.is_empty()).unwrap_or(params.name);
    if let Some(label) = params.labels.iter().find(|label| label.starts_with("__")) {
        return Err(any_err(format!(
            "label {label:?} uses Prometheus's reserved __ prefix"
        )));
    }
    params.labels.sort_unstable();
    // Validate the declaration before treating a difference as a reload conflict.
    let desc = Desc::new(name.clone(), help, params.labels, HashMap::new()).map_err(any_err)?;
    let family = match FAMILIES.lock().entry(name) {
        Entry::Vacant(entry) => {
            let family = Arc::new(Family {
                desc,
                series: RwLock::default(),
                warn_help: Once::new(),
                warn_labels: Once::new(),
            });
            return Ok(LuaCounter::Active(entry.insert(family).clone()));
        }
        Entry::Occupied(entry) => entry.get().clone(),
    };

    if family.desc.variable_labels != desc.variable_labels {
        family.warn_labels.call_once(|| tracing::warn!(
            "{} already uses labels {:?}, not {:?}; ignoring increments through this handle. Restore the original labels, use a new metric name, or restart the process",
            desc.fq_name, family.desc.variable_labels, desc.variable_labels
        ));
        return Ok(LuaCounter::Disabled);
    }
    if family.desc.help != desc.help {
        family.warn_help.call_once(|| {
            tracing::warn!(
                "{}: keeping the original help text until restart; increments still count",
                desc.fq_name
            )
        });
    }
    Ok(LuaCounter::Active(family))
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    get_or_create_sub_module(lua, "metrics")?
        .set("counter", lua.create_function(define_counter)?)?;
    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn series_appears_only_after_a_valid_increment() -> anyhow::Result<()> {
        let lua = Lua::new();
        register(&lua)?;
        lua.load(
            r#"
            counter = require('kumo').metrics.counter { name = 'increment_test_total' }
            for _, amount in ipairs { 1.5, -0.5, -1 } do
                assert(not pcall(function() counter:inc({}, amount) end))
            end
        "#,
        )
        .exec()?;
        let sample = || {
            prometheus::gather()
                .iter()
                .find(|f| f.name() == "policy_increment_test_total")
                .map(|f| f.get_metric()[0].get_counter().value())
        };
        assert_eq!(sample(), None);
        lua.load("counter:inc({}, 0)").exec()?;
        assert_eq!(sample(), Some(0.0));
        lua.load("counter:inc({}, 2.0)").exec()?;
        assert_eq!(sample(), Some(2.0));
        Ok(())
    }

    #[test]
    fn invalid_labels_do_not_reserve_a_counter_name() -> anyhow::Result<()> {
        let lua = Lua::new();
        register(&lua)?;
        lua.load(
            r#"
            local define = require('kumo').metrics.counter
            local name = 'label_validation_test_total'
            for _, labels in ipairs {
                { rule = true }, { 'rule', extra = 'result' }, { [1] = 'rule', [3] = 'result' },
                { '__name__' }, { '__internal' },
            } do
                assert(not pcall(define, { name = name, labels = labels }))
            end
            define { name = name, help = '', labels = { '_rule' } }:inc { _rule = 'pass' }
        "#,
        )
        .exec()?;
        let output = prometheus::TextEncoder::new().encode_to_string(&prometheus::gather())?;
        assert!(output
            .lines()
            .any(|line| line
                == "# HELP policy_label_validation_test_total label_validation_test_total"));
        assert!(output
            .lines()
            .any(|line| line == r#"policy_label_validation_test_total{_rule="pass"} 1"#));
        Ok(())
    }

    #[test]
    fn reload_conflicts_preserve_the_counter() -> anyhow::Result<()> {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tracing_subscriber::layer::{Context, SubscriberExt};
        use tracing_subscriber::{Layer, Registry};

        #[derive(Clone, Default)]
        struct Warnings(Arc<AtomicUsize>);
        impl Layer<Registry> for Warnings {
            fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, Registry>) {
                if *event.metadata().level() == tracing::Level::WARN {
                    self.0.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        let warnings = Warnings::default();
        let _guard = tracing::subscriber::set_default(Registry::default().with(warnings.clone()));

        let original = Lua::new();
        register(&original)?;
        original.load(r#"
            checks = require('kumo').metrics.counter { name = 'reload_test_total', help = 'original', labels = { 'rule' } }
            checks:inc { rule = 'gate' }
            assert(not pcall(require('kumo').metrics.counter, { name = 'reload_test_total', labels = { 'bad-label' } }))
        "#).exec()?;
        for (label, expected_warnings) in [("rule", 1), ("rule", 1), ("result", 2), ("result", 2)] {
            let lua = Lua::new();
            register(&lua)?;
            let callback = lua.load(r#"
                local label = ...
                local checks = require('kumo').metrics.counter { name = 'reload_test_total', help = 'new help', labels = { label } }
                return function() checks:inc { [label] = 'gate' } end
            "#).call::<mlua::Function>(label)?;
            assert_eq!(warnings.0.load(Ordering::Relaxed), expected_warnings);
            callback.call::<()>(())?;
        }
        original.load("checks:inc { rule = 'gate' }").exec()?;

        let family = prometheus::gather()
            .into_iter()
            .find(|f| f.name() == "policy_reload_test_total")
            .unwrap();
        assert_eq!(family.help(), "original");
        let [sample] = family.get_metric() else {
            panic!("expected only the original series")
        };
        assert_eq!(sample.get_counter().value(), 4.0);
        Ok(())
    }

    #[test]
    fn policy_metrics_share_counters_across_lua_contexts() -> anyhow::Result<()> {
        for _ in 0..2 {
            let lua = Lua::new();
            register(&lua)?;
            lua.load(r#"
                local metrics = require('kumo').metrics
                local counter = metrics.counter { name = 'series_test_total', labels = { 'route', 'group' } }
                counter:inc { route = 'bc', group = 'a' }
                counter:inc({ group = 'ab', route = 'c' }, 3)
                assert(not pcall(function() counter:inc { route = 'bc' } end))
                metrics.counter { name = 'unlabelled_test_total' }:inc()
            "#).exec()?;
        }
        let output = prometheus::TextEncoder::new().encode_to_string(&prometheus::gather())?;
        for sample in [
            r#"policy_series_test_total{group="a",route="bc"} 2"#,
            r#"policy_series_test_total{group="ab",route="c"} 6"#,
            "policy_unlabelled_test_total 2",
        ] {
            assert!(
                output.lines().any(|line| line == sample),
                "missing {sample}"
            );
        }
        Ok(())
    }
}

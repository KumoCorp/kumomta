#![cfg(feature = "lua")]

use mlua::Lua;
use std::io::Write;
use tempfile::TempDir;

/// Helper: create a zstd-compressed JSONL file with the given records.
fn write_zstd_log(dir: &std::path::Path, filename: &str, records: &[&str], mark_done: bool) {
    let path = dir.join(filename);
    let mut encoder = zstd::Encoder::new(std::fs::File::create(&path).unwrap(), 3).unwrap();
    for record in records {
        writeln!(encoder, "{record}").unwrap();
    }
    encoder.finish().unwrap();

    if mark_done {
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(true);
        std::fs::set_permissions(&path, perms).unwrap();
    }
}

/// Run a lua script in a fresh Lua state with the tailer module registered.
async fn run_lua(script: &str) -> mlua::Result<()> {
    let lua = Lua::new();
    kumo_log_tailer::lua::register(&lua).unwrap();
    // Make the kumo module accessible as a global (normally done by
    // the kumomta config loader).
    let package: mlua::Table = lua.globals().get("package")?;
    let loaded: mlua::Table = package.get("loaded")?;
    let kumo: mlua::Table = loaded.get("kumo")?;
    lua.globals().set("kumo", kumo)?;
    lua.load(script).exec_async().await
}

/// Test the single-consumer lua interface (kumo.tailer.new).
#[tokio::test]
async fn test_lua_single_consumer() {
    let dir = TempDir::new().unwrap();
    let log_dir = dir.path().to_str().unwrap().to_string();

    write_zstd_log(
        dir.path(),
        "seg-001.zst",
        &[
            r#"{"type":"Delivery","id":1}"#,
            r#"{"type":"Bounce","id":2}"#,
        ],
        true,
    );
    write_zstd_log(
        dir.path(),
        "seg-002.zst",
        &[r#"{"type":"Delivery","id":3}"#],
        true,
    );

    let script = format!(
        r#"
        local tailer = kumo.tailer.new {{
            directory = '{log_dir}',
            pattern = '*.zst',
            max_batch_size = 100,
            max_batch_latency = '100ms',
        }}

        local all_ids = {{}}

        for batch in tailer:batches() do
            for record in batch:iter_records() do
                table.insert(all_ids, record.id)
            end
            batch:commit()
            -- After consuming all done files, break
            if #all_ids >= 3 then
                break
            end
        end

        tailer:close()

        assert(#all_ids == 3, 'expected 3 records, got ' .. #all_ids)
        assert(all_ids[1] == 1, 'first record id should be 1')
        assert(all_ids[2] == 2, 'second record id should be 2')
        assert(all_ids[3] == 3, 'third record id should be 3')
        "#
    );

    run_lua(&script).await.unwrap();
}

/// Test the single-consumer lua interface with a filter.
#[tokio::test]
async fn test_lua_single_consumer_with_filter() {
    let dir = TempDir::new().unwrap();
    let log_dir = dir.path().to_str().unwrap().to_string();

    write_zstd_log(
        dir.path(),
        "seg-001.zst",
        &[
            r#"{"type":"Delivery","id":1}"#,
            r#"{"type":"Bounce","id":2}"#,
            r#"{"type":"Delivery","id":3}"#,
        ],
        true,
    );

    let script = format!(
        r#"
        local tailer = kumo.tailer.new(
            {{
                directory = '{log_dir}',
                pattern = '*.zst',
                max_batch_size = 100,
                max_batch_latency = '100ms',
            }},
            function(record)
                return record.type == 'Delivery'
            end
        )

        local all_ids = {{}}

        for batch in tailer:batches() do
            for record in batch:iter_records() do
                table.insert(all_ids, record.id)
            end
            batch:commit()
            if #all_ids >= 2 then
                break
            end
        end

        tailer:close()

        assert(#all_ids == 2, 'expected 2 records, got ' .. #all_ids)
        assert(all_ids[1] == 1, 'first should be id 1')
        assert(all_ids[2] == 3, 'second should be id 3')
        "#
    );

    run_lua(&script).await.unwrap();
}

/// Test the multi-consumer lua interface (kumo.tailer.new_multi).
#[tokio::test]
async fn test_lua_multi_consumer() {
    let dir = TempDir::new().unwrap();
    let log_dir = dir.path().to_str().unwrap().to_string();

    write_zstd_log(
        dir.path(),
        "seg-001.zst",
        &[
            r#"{"type":"Delivery","id":1}"#,
            r#"{"type":"Bounce","id":2}"#,
            r#"{"type":"Delivery","id":3}"#,
            r#"{"type":"Bounce","id":4}"#,
        ],
        true,
    );

    let script = format!(
        r#"
        local tailer = kumo.tailer.new_multi {{
            directory = '{log_dir}',
            pattern = '*.zst',
            consumers = {{
                {{
                    name = 'deliveries',
                    max_batch_size = 100,
                    max_batch_latency = '100ms',
                    filter = function(record)
                        return record.type == 'Delivery'
                    end,
                }},
                {{
                    name = 'bounces',
                    max_batch_size = 100,
                    max_batch_latency = '100ms',
                    filter = function(record)
                        return record.type == 'Bounce'
                    end,
                }},
            }},
        }}

        local delivery_ids = {{}}
        local bounce_ids = {{}}
        local iterations = 0

        for batches in tailer:batches() do
            iterations = iterations + 1
            for _, batch in ipairs(batches) do
                local name = batch:consumer_name()
                local records = batch:records()
                for _, record in ipairs(records) do
                    if name == 'deliveries' then
                        table.insert(delivery_ids, record.id)
                    elseif name == 'bounces' then
                        table.insert(bounce_ids, record.id)
                    end
                end
                batch:commit()
            end
            if #delivery_ids >= 2 and #bounce_ids >= 2 then
                break
            end
        end

        tailer:close()

        assert(#delivery_ids == 2,
            'expected 2 deliveries, got ' .. #delivery_ids)
        assert(delivery_ids[1] == 1,
            'first delivery should be id 1, got ' .. tostring(delivery_ids[1]))
        assert(delivery_ids[2] == 3,
            'second delivery should be id 3, got ' .. tostring(delivery_ids[2]))

        assert(#bounce_ids == 2,
            'expected 2 bounces, got ' .. #bounce_ids)
        assert(bounce_ids[1] == 2,
            'first bounce should be id 2, got ' .. tostring(bounce_ids[1]))
        assert(bounce_ids[2] == 4,
            'second bounce should be id 4, got ' .. tostring(bounce_ids[2]))
        "#
    );

    run_lua(&script).await.unwrap();
}

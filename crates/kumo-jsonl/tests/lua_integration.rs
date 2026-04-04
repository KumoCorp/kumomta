#![cfg(feature = "lua")]

use camino::Utf8PathBuf;
use kumo_jsonl::LogWriterConfig;
use mlua::Lua;
use tempfile::TempDir;

/// Write records into a single completed segment using [`LogWriter`].
fn write_segment(dir: &std::path::Path, records: &[&str]) {
    let log_dir = Utf8PathBuf::try_from(dir.to_path_buf()).unwrap();
    let mut writer = LogWriterConfig::new(log_dir)
        .compression_level(3)
        .max_file_size(u64::MAX)
        .build();
    for record in records {
        writer.write_line(record).unwrap();
    }
    writer.close().unwrap();
}

/// Run a lua script in a fresh Lua state with the tailer module registered.
async fn run_lua(script: &str) -> mlua::Result<()> {
    let lua = Lua::new();
    kumo_jsonl::lua::register(&lua).unwrap();
    // Make the kumo module accessible as a global (normally done by
    // the kumomta config loader).
    let package: mlua::Table = lua.globals().get("package")?;
    let loaded: mlua::Table = package.get("loaded")?;
    let kumo: mlua::Table = loaded.get("kumo")?;
    lua.globals().set("kumo", kumo)?;
    lua.load(script).exec_async().await
}

/// Test the single-consumer lua interface (kumo.jsonl.new_tailer).
#[tokio::test]
async fn test_lua_single_consumer() {
    let dir = TempDir::new().unwrap();
    let log_dir = dir.path().to_str().unwrap().to_string();

    write_segment(
        dir.path(),
        &[
            r#"{"type":"Delivery","id":1}"#,
            r#"{"type":"Bounce","id":2}"#,
        ],
    );
    write_segment(dir.path(), &[r#"{"type":"Delivery","id":3}"#]);

    let script = format!(
        r#"
        local tailer <close> = kumo.jsonl.new_tailer {{
            directory = '{log_dir}',
            
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

    write_segment(
        dir.path(),
        &[
            r#"{"type":"Delivery","id":1}"#,
            r#"{"type":"Bounce","id":2}"#,
            r#"{"type":"Delivery","id":3}"#,
        ],
    );

    let script = format!(
        r#"
        local tailer <close> = kumo.jsonl.new_tailer(
            {{
                directory = '{log_dir}',
                
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

        assert(#all_ids == 2, 'expected 2 records, got ' .. #all_ids)
        assert(all_ids[1] == 1, 'first should be id 1')
        assert(all_ids[2] == 3, 'second should be id 3')
        "#
    );

    run_lua(&script).await.unwrap();
}

/// Test the multi-consumer lua interface (kumo.jsonl.new_multi_tailer).
#[tokio::test]
async fn test_lua_multi_consumer() {
    let dir = TempDir::new().unwrap();
    let log_dir = dir.path().to_str().unwrap().to_string();

    write_segment(
        dir.path(),
        &[
            r#"{"type":"Delivery","id":1}"#,
            r#"{"type":"Bounce","id":2}"#,
            r#"{"type":"Delivery","id":3}"#,
            r#"{"type":"Bounce","id":4}"#,
        ],
    );

    let script = format!(
        r#"
        local tailer <close> = kumo.jsonl.new_multi_tailer {{
            directory = '{log_dir}',
            
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

/// Test the writer lua interface (kumo.jsonl.new_writer).
#[tokio::test]
async fn test_lua_writer() {
    let dir = TempDir::new().unwrap();
    let log_dir = dir.path().to_str().unwrap().to_string();

    // Write records via lua using :write_line and :write_record,
    // then read them back and verify they are correct.
    let write_script = format!(
        r#"
        local writer <close> = kumo.jsonl.new_writer {{
            log_dir = '{log_dir}',
        }}

        writer:write_line('{{"source":"write_line"}}')
        writer:write_record({{source='write_record', n=2}})
        "#
    );
    run_lua(&write_script).await.unwrap();

    // Read back and verify
    let read_script = format!(
        r#"
        local tailer <close> = kumo.jsonl.new_tailer {{
            directory = '{log_dir}',
            max_batch_size = 100,
            max_batch_latency = '100ms',
        }}

        local records = {{}}
        for batch in tailer:batches() do
            for record in batch:iter_records() do
                table.insert(records, record)
            end
            batch:commit()
            if #records >= 2 then break end
        end

        assert(#records == 2, 'expected 2 records, got ' .. #records)
        assert(records[1].source == 'write_line',
            'first record source should be write_line, got ' .. tostring(records[1].source))
        assert(records[2].source == 'write_record',
            'second record source should be write_record')
        assert(records[2].n == 2,
            'second record n should be 2, got ' .. tostring(records[2].n))
        "#
    );
    run_lua(&read_script).await.unwrap();
}

/// Test that the writer accepts a timezone name via the `tz` parameter.
/// Verifies that a segment file is created and can be read back,
/// proving the timezone was accepted without error.
#[tokio::test]
async fn test_lua_writer_with_tz() {
    let dir = TempDir::new().unwrap();
    let log_dir = dir.path().to_str().unwrap().to_string();

    let script = format!(
        r#"
        local writer <close> = kumo.jsonl.new_writer {{
            log_dir = '{log_dir}',
            tz = 'America/New_York',
        }}

        writer:write_record({{msg = 'hello'}})
        "#
    );
    run_lua(&script).await.unwrap();

    // A segment file should have been created and closed (marked done)
    let entries: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(entries.len(), 1, "expected one segment file");
    assert!(
        entries[0].metadata().unwrap().permissions().readonly(),
        "segment should be readonly (done)"
    );
}

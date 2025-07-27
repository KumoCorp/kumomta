use config::any_err;
use mlua::prelude::{LuaUserData, *};
use mlua::{Lua, UserDataMethods};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, BufStream, SeekFrom};
use tokio::sync::Mutex;

pub struct AsyncFile {
    file: Mutex<Option<BufStream<File>>>,
}

impl AsyncFile {
    async fn do_close(&self) -> anyhow::Result<()> {
        if let Some(mut file) = self.file.lock().await.take() {
            file.flush().await?;
        }
        Ok(())
    }

    async fn do_flush(&self) -> anyhow::Result<()> {
        let mut file = self.file.lock().await;
        match file.as_mut() {
            Some(file) => Ok(file.flush().await?),
            None => {
                anyhow::bail!("attempt to flush a closed file handle");
            }
        }
    }

    async fn do_read(&self, n: Option<usize>) -> anyhow::Result<Vec<u8>> {
        let mut file = self.file.lock().await;
        match file.as_mut() {
            Some(file) => match n {
                Some(n) => {
                    let mut buf = vec![0u8; n];
                    let n_read = file.read(&mut buf).await?;
                    buf.truncate(n_read);
                    Ok(buf)
                }
                None => {
                    let mut buf = vec![];
                    file.read_to_end(&mut buf).await?;
                    Ok(buf)
                }
            },
            None => {
                anyhow::bail!("attempt to flush a closed file handle");
            }
        }
    }

    async fn do_write(&self, buf: &[u8]) -> anyhow::Result<()> {
        let mut file = self.file.lock().await;
        match file.as_mut() {
            Some(file) => {
                file.write_all(buf).await?;
                Ok(())
            }
            None => {
                anyhow::bail!("attempt to flush a closed file handle");
            }
        }
    }

    async fn do_seek(&self, pos: SeekFrom) -> anyhow::Result<u64> {
        let mut file = self.file.lock().await;
        match file.as_mut() {
            Some(file) => Ok(file.seek(pos).await?),
            None => {
                anyhow::bail!("attempt to seek a closed file handle");
            }
        }
    }
}

impl LuaUserData for AsyncFile {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_async_method(
            "close",
            move |_lua, this: LuaUserDataRef<AsyncFile>, ()| async move {
                this.do_close().await.map_err(any_err)
            },
        );
        methods.add_async_method(
            "flush",
            move |_lua, this: LuaUserDataRef<AsyncFile>, ()| async move {
                this.do_flush().await.map_err(any_err)
            },
        );
        methods.add_async_method(
            "read",
            move |lua, this: LuaUserDataRef<AsyncFile>, n: Option<usize>| async move {
                let buf = this.do_read(n).await.map_err(any_err)?;
                lua.create_string(buf)
            },
        );
        methods.add_async_method(
            "write",
            move |_lua, this: LuaUserDataRef<AsyncFile>, buf: mlua::String| async move {
                this.do_write(&buf.as_bytes()).await.map_err(any_err)
            },
        );
        methods.add_async_method(
            "seek",
            move |_lua, this: LuaUserDataRef<AsyncFile>, mut params: LuaMultiValue| async move {
                let mut offset = None;
                let mut whence = None;

                while let Some(p) = params.pop_front() {
                    match p {
                        LuaValue::String(s) => {
                            if whence.is_some() {
                                return Err(mlua::Error::external("whence already set"));
                            }
                            match s.as_bytes().as_ref() {
                                b"set" => {
                                    whence.replace('s');
                                }
                                b"cur" => {
                                    whence.replace('c');
                                }
                                b"end" => {
                                    whence.replace('e');
                                }
                                _ => {
                                    return Err(mlua::Error::external(
                                        "invalid whence/offset parameters to seek",
                                    ));
                                }
                            }
                        }
                        LuaValue::Integer(n) => {
                            if offset.is_some() {
                                return Err(mlua::Error::external("offset already set"));
                            }
                            offset.replace(n);
                        }
                        _ => {
                            return Err(mlua::Error::external(
                                "invalid whence/offset parameters to seek",
                            ));
                        }
                    }
                }

                let offset = offset.unwrap_or(0);
                let pos = match whence.unwrap_or('c') {
                    'c' => SeekFrom::Current(offset),
                    's' => SeekFrom::Start(offset.try_into().map_err(|_| {
                        mlua::Error::external(format!(
                            "start-relative offset {offset} is out of range"
                        ))
                    })?),
                    'e' => SeekFrom::End(offset),
                    _ => unreachable!(),
                };

                this.do_seek(pos).await.map_err(any_err)
            },
        );
    }
}

impl AsyncFile {
    pub async fn open(_: Lua, (filename, mode): (String, Option<String>)) -> mlua::Result<Self> {
        let mut options = OpenOptions::new();

        let mode = mode.as_deref().unwrap_or("r");
        match mode {
            "r" | "rb" => options.read(true),
            "w" | "wb" => options.write(true).create(true),
            "a" | "ab" => options.write(true).append(true).create(true),
            "r+" | "r+b" => options.read(true).write(true).create(true),
            "w+" | "w+b" => options.read(true).write(true).truncate(true).create(true),
            "a+" | "a+b" => options.read(true).write(true).append(true).create(true),
            unsup => {
                return Err(mlua::Error::external(format!(
                    "unsupported file mode `{unsup}`"
                )));
            }
        };

        let file = options.open(&filename).await.map_err(|err| {
            mlua::Error::external(format!(
                "failed to open {filename} with mode {mode}: {err:#}"
            ))
        })?;

        let file = BufStream::new(file);

        Ok(Self {
            file: Mutex::new(Some(file)),
        })
    }
}

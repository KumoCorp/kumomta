// The contents of this file are derived from
// <https://github.com/saiko-tech/mmproxy-rs/blob/9fdd5ed9d532dee9b62dafb592acecc6da33dc5f/src/listener/tcp.rs#L129>
// which is provided under the MIT License and is
// Copyright (c) 2022 Saiko Technology Ltd.
use anyhow::Context;
use std::io::{Error as IoError, ErrorKind as IoErrorKind, Result as IoResult};
use std::os::fd::AsRawFd;
use tokio::io::Interest;
use tokio::net::tcp::{ReadHalf, WriteHalf};

/// 1MB pipe buffer
const PIPE_BUF_SIZE: usize = 1024 * 1024;

/// This linux specific function uses the splice(2) syscall to perform
/// data transfer from src -> dst via a kernel pipe buffer, eliminating
/// the need to copy the data between userspace and the kernel that
/// would otherwise need to occur.
pub async fn splice_copy(src: &mut ReadHalf<'_>, dst: &mut WriteHalf<'_>) -> anyhow::Result<()> {
    let pipe = Pipe::new().context("failed to create pipe")?;
    // number of bytes that the pipe buffer is currently holding
    let mut size = 0;
    let mut done = false;

    let src = src.as_ref();
    let dst = dst.as_ref();
    let src_fd = src.as_raw_fd();
    let dst_fd = dst.as_raw_fd();

    while !done {
        if size == 0 {
            // Wait for data to arrive
            src.readable()
                .await
                .context("awaiting on readable failed")?;
        }
        // (Speculatively) attempt to fill the pipe buffer
        let ret = src.try_io(Interest::READABLE, || {
            while size < PIPE_BUF_SIZE {
                let r = splice(src_fd, pipe.w, PIPE_BUF_SIZE - size)?;
                if r == 0 {
                    done = true;
                    break;
                }
                size += r;
            }
            Ok(())
        });
        if let Err(err) = ret {
            if err.kind() != IoErrorKind::WouldBlock {
                return if done {
                    Ok(())
                } else {
                    Err(err).context("splicing src -> pipe")
                };
            }
        }

        if size == 0 {
            // No data yet; continue the loop to wait for more
            continue;
        }

        dst.writable()
            .await
            .context("awaiting on writable failed")?;
        let ret = dst.try_io(Interest::WRITABLE, || {
            while size > 0 {
                let r = splice(pipe.r, dst_fd, size)?;
                size -= r;
            }
            Ok(())
        });
        if let Err(err) = ret {
            if err.kind() != IoErrorKind::WouldBlock {
                return if done {
                    Ok(())
                } else {
                    Err(err).context("splicing pipe -> dest")
                };
            }
        }
    }

    Ok(())
}

#[derive(Debug)]
struct Pipe {
    pub r: i32,
    pub w: i32,
}

impl Pipe {
    pub fn new() -> std::io::Result<Self> {
        let pipes = unsafe {
            let mut pipes = std::mem::MaybeUninit::<[libc::c_int; 2]>::uninit();
            if libc::pipe2(
                pipes.as_mut_ptr().cast(),
                libc::O_NONBLOCK | libc::O_CLOEXEC,
            ) < 0
            {
                return Err(IoError::last_os_error());
            }
            pipes.assume_init()
        };

        unsafe {
            if libc::fcntl(pipes[0], libc::F_SETPIPE_SZ, PIPE_BUF_SIZE) < 0 {
                libc::close(pipes[0]);
                libc::close(pipes[1]);

                return Err(IoError::last_os_error());
            }
        }

        Ok(Self {
            r: pipes[0],
            w: pipes[1],
        })
    }
}

impl Drop for Pipe {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.r);
            libc::close(self.w);
        }
    }
}

fn splice(r: i32, w: i32, n: usize) -> IoResult<usize> {
    let result = unsafe {
        libc::splice(
            r,
            std::ptr::null_mut(),
            w,
            std::ptr::null_mut(),
            n,
            libc::SPLICE_F_MOVE | libc::SPLICE_F_NONBLOCK,
        )
    };

    if result >= 0 {
        return Ok(result as usize);
    }

    let err = IoError::last_os_error();

    // Normalize EAGAIN to WouldBlock
    let errno = err.raw_os_error().unwrap_or(0);
    if (errno == libc::EWOULDBLOCK || errno == libc::EAGAIN)
        && err.kind() != IoErrorKind::WouldBlock
    {
        Err(IoError::new(IoErrorKind::WouldBlock, "EWOULDBLOCK"))
    } else {
        Err(err)
    }
}

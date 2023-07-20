use nix::sys::signal::{kill, SIGQUIT};
use nix::unistd::Pid;

pub fn register_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info.payload();
        let payload = payload.downcast_ref::<&str>().unwrap_or(&"!?");
        let bt = backtrace::Backtrace::new();
        if let Some(loc) = info.location() {
            tracing::error!(
                "panic at {}:{}:{} - {}\n{:?}",
                loc.file(),
                loc.line(),
                loc.column(),
                payload,
                bt
            );
        } else {
            tracing::error!("panic - {}\n{:?}", payload, bt);
        }

        default_hook(info);

        // Request a core dump
        kill(Pid::this(), SIGQUIT).ok();
    }));
}

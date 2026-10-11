//! Reading a 3D mouse (feature `native`): a background thread finds a SpaceMouse among the HID
//! devices, reads its input reports, decodes them and hands the [`Event`]s over. It looks again
//! every couple of seconds while none is connected, and after one is unplugged or its connection
//! drops, so plugging one in, or waking a wireless one, just works.
//!
//! On Linux a device that can't be opened is skipped like a missing one: its `/dev/hidraw*` node
//! is root's alone until a udev rule grants access
//! (`packaging/linux/71-solvecraft-3dconnexion.rules`).
//!
//! Nothing here can bring the program down: a failure ends that connection (the view comes to
//! rest and the buttons are released) and the search starts again, and a panic in the HID
//! library is caught the same way.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::task::{Poll, Waker};
use std::time::Duration;

use async_hid::{AsyncHidRead, DeviceReader, HidBackend};
use futures_lite::StreamExt;
use futures_lite::future::{block_on, or, poll_fn};

use crate::{Buttons, Decoder, Event, Motion, is_space_mouse, model_name};

/// How long to wait between searches while no 3D mouse is connected.
const RESCAN: Duration = Duration::from_secs(2);
/// The most events handed over at once (a stalled UI doesn't let them pile up without end).
const MAX_EVENTS: usize = 4096;
/// Input reports are at most this long (3D mice send 7 to 13 bytes).
const REPORT_BYTES: usize = 64;

/// A running reader. Dropping it stops it: the search ends, and a read waiting on a quiet device
/// ends at once, closing the device.
pub struct Reader {
    events: mpsc::Receiver<Event>,
    stop: Arc<AtomicBool>,
    /// Wakes a read waiting on the device, so a stop ends it.
    waiting: Arc<Mutex<Option<Waker>>>,
    device: Arc<Mutex<Option<String>>>,
}

impl Reader {
    /// Start reading. `wake` is called after each event (from the reader's thread), so the UI
    /// can draw a frame even when nothing else happens.
    pub fn start(wake: impl Fn() + Send + 'static) -> Reader {
        let (tx, events) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let waiting = Arc::new(Mutex::new(None));
        let device = Arc::new(Mutex::new(None));
        let (s, w, d) = (stop.clone(), waiting.clone(), device.clone());
        let spawned = std::thread::Builder::new().name("spacemouse".into()).spawn(move || run(&tx, &s, &w, &d, &wake));
        if spawned.is_err() {
            // No thread, no 3D mouse; the program goes on without it.
            stop.store(true, Ordering::Relaxed);
        }
        Reader { events, stop, waiting, device }
    }

    /// The events since the last call.
    pub fn events(&self) -> Vec<Event> {
        self.events.try_iter().take(MAX_EVENTS).collect()
    }

    /// The connected 3D mouse's model, if one is connected.
    pub fn device(&self) -> Option<String> {
        self.device.lock().ok().and_then(|d| d.clone())
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(w) = self.waiting.lock().ok().and_then(|mut w| w.take()) {
            w.wake();
        }
    }
}

fn run(tx: &mpsc::Sender<Event>, stop: &AtomicBool, waiting: &Mutex<Option<Waker>>, device: &Mutex<Option<String>>, wake: &dyn Fn()) {
    let backend = HidBackend::default();
    while !stop.load(Ordering::Relaxed) {
        // One connection: find, read until it ends. A panic inside ends it too.
        let alive = catch_unwind(AssertUnwindSafe(|| {
            let Some((name, mut reader)) = find(&backend) else { return true };
            set(device, Some(name));
            let alive = read(&mut reader, tx, stop, waiting, wake);
            set(device, None);
            alive
        }))
        .unwrap_or(true);
        set(device, None);
        // Whatever was held when the connection ended is let go.
        let released = tx.send(Event::Motion(Motion::default())).is_ok() && tx.send(Event::Buttons(Buttons(0))).is_ok();
        wake();
        if !alive || !released {
            // The receiving side is gone: the program is closing.
            return;
        }
        for _ in 0..20 {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(RESCAN / 20);
        }
    }
}

fn set(device: &Mutex<Option<String>>, name: Option<String>) {
    if let Ok(mut d) = device.lock() {
        *d = name;
    }
}

/// The first 3D mouse that opens, with its model name.
fn find(backend: &HidBackend) -> Option<(String, DeviceReader)> {
    block_on(async {
        let devices: Vec<_> = backend.enumerate().await.ok()?.collect().await;
        for d in devices {
            if !is_space_mouse(d.vendor_id, d.product_id, d.usage_page, d.usage_id) {
                continue;
            }
            if let Ok(reader) = d.open_readable().await {
                let name = model_name(d.vendor_id, d.product_id).map(str::to_string).unwrap_or_else(|| {
                    let n = d.name.trim();
                    if n.is_empty() { "3D mouse".to_string() } else { n.chars().take(80).collect() }
                });
                return Some((name, reader));
            }
        }
        None
    })
}

/// Read reports until the device goes away. Returns false when the reader is stopped or the
/// receiving side is gone.
fn read(reader: &mut DeviceReader, tx: &mpsc::Sender<Event>, stop: &AtomicBool, waiting: &Mutex<Option<Waker>>, wake: &dyn Fn()) -> bool {
    let mut decoder = Decoder::new();
    let mut buf = [0u8; REPORT_BYTES];
    while !stop.load(Ordering::Relaxed) {
        // The next report, or the stop, whichever comes first.
        let report = block_on(or(async { Some(reader.read_input_report(&mut buf).await) }, stopped(stop, waiting)));
        let Some(n) = report else { return false };
        let Ok(n) = n else { return true };
        let report = buf.get(..n.min(REPORT_BYTES)).unwrap_or_default();
        if let Some(e) = decoder.decode(report) {
            if tx.send(e).is_err() {
                return false;
            }
            wake();
        }
    }
    true
}

/// Ready once the reader is stopped: [`Reader`]'s drop sets the flag, then wakes the waker left
/// here.
async fn stopped(stop: &AtomicBool, waiting: &Mutex<Option<Waker>>) -> Option<async_hid::HidResult<usize>> {
    poll_fn(|cx| {
        if let Ok(mut w) = waiting.lock() {
            *w = Some(cx.waker().clone());
        }
        if stop.load(Ordering::Relaxed) { Poll::Ready(None) } else { Poll::Pending }
    })
    .await
}

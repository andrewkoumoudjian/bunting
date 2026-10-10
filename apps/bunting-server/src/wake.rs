//! One event channel per FIX connection, so its session thread wakes
//! exactly when there is work: bytes from the socket (stamped by the reader
//! the instant they arrive), a released job's reply, or a committed batch.
//! Held messages and probes are timed with the channel's receive deadline.

use std::io::Read;
use std::net::TcpStream;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

/// Socket reads waiting for the session; full means the session is busy,
/// and the reader then waits, which is ordinary TCP backpressure.
const EVENT_CAPACITY: usize = 1_024;

pub(crate) enum SessionEvent {
    /// Bytes read from the socket with the venue time they arrived.
    Bytes { received_us: u64, bytes: Vec<u8> },
    /// The socket closed (`None`) or failed (the reason).
    Closed(Option<String>),
    /// A reply or committed batch is ready for this session.
    Wake,
}

/// Lets another thread wake a session without blocking.
#[derive(Clone)]
pub(crate) struct Waker(SyncSender<SessionEvent>);

impl Waker {
    /// A full channel already guarantees the session will wake.
    pub(crate) fn wake(&self) {
        let _ = self.0.try_send(SessionEvent::Wake);
    }
}

/// Starts the reader thread for `stream` and returns the session's event
/// receiver and waker. `now_us` must be the venue clock.
pub(crate) fn connect(
    stream: &TcpStream,
    max_read_bytes: usize,
    now_us: impl Fn() -> u64 + Send + 'static,
) -> Result<(Receiver<SessionEvent>, Waker), String> {
    let mut reader = stream
        .try_clone()
        .map_err(|error| format!("cannot clone FIX socket for reading: {error}"))?;
    reader
        .set_read_timeout(None)
        .map_err(|error| format!("cannot configure FIX reader: {error}"))?;
    let (sender, events) = sync_channel(EVENT_CAPACITY);
    let waker = Waker(sender.clone());
    std::thread::Builder::new()
        .name("bunting-fix-reader".to_owned())
        .spawn(move || {
            let mut buffer = vec![0; max_read_bytes.max(1)];
            loop {
                let event = match reader.read(&mut buffer) {
                    Ok(0) => SessionEvent::Closed(None),
                    Ok(count) => SessionEvent::Bytes {
                        received_us: now_us(),
                        bytes: buffer[..count].to_vec(),
                    },
                    Err(error) => {
                        SessionEvent::Closed(Some(format!("FIX socket read failed: {error}")))
                    }
                };
                let closed = matches!(event, SessionEvent::Closed(_));
                if sender.send(event).is_err() || closed {
                    return;
                }
            }
        })
        .map_err(|error| format!("cannot spawn FIX reader: {error}"))?;
    Ok((events, waker))
}

/// Shuts the socket down when the session ends, so its reader thread's
/// blocking read returns and the thread exits.
pub(crate) struct ShutdownOnDrop(TcpStream);

impl ShutdownOnDrop {
    pub(crate) fn new(stream: &TcpStream) -> Result<Self, String> {
        stream
            .try_clone()
            .map(Self)
            .map_err(|error| format!("cannot clone FIX socket: {error}"))
    }
}

impl Drop for ShutdownOnDrop {
    fn drop(&mut self) {
        let _ = self.0.shutdown(std::net::Shutdown::Both);
    }
}

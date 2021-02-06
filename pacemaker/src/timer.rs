//! Timer
use super::pacemaker::TchanS;
use futures_timer::Delay;
use hotstuff_rs::data::ViewNumber;
use log::error;
use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc,
};
use std::time::Duration;

/// Cancellable timer.
pub(crate) struct DefaultTimer {
    // default 300_000 ms
    max_timeout: u64,
    rtt: u64,
    stop_ch: tokio::sync::broadcast::Sender<()>,
    // bcast_recvr: tokio::sync::broadcast::Receiver<()>,
    notifier: TchanS<ViewNumber>,
    cnt: Arc<AtomicU32>,
}

impl DefaultTimer {
    pub(crate) fn timeout_by_delay(&self) -> Duration {
        Duration::from_millis(u64::min(self.max_timeout, self.rtt.wrapping_shl(1)))
    }

    pub(crate) fn timout_on_gap(&self, alpha: u64, beta: u64, gap: usize) -> Duration {
        Duration::from_millis(u64::min(
            self.max_timeout,
            alpha.saturating_add(beta.wrapping_shl(gap as u32)),
        ))
    }

    pub(crate) fn new(notifier: TchanS<ViewNumber>, max_timeout: u64, rtt: u64) -> Self {
        let (stop_ch, _) = tokio::sync::broadcast::channel(1);

        Self {
            max_timeout,
            rtt,
            stop_ch,
            notifier,
            cnt: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Start new timer.
    pub(crate) fn start(&self, view: ViewNumber, dur: Duration) {
        let mut end_ch = self.stop_ch.subscribe();
        let s = self.notifier.clone();
        let cnt = self.cnt.clone();
        tokio::spawn(async move {
            cnt.fetch_add(1, Ordering::SeqCst);
            tokio::select! {
                () = Delay::new(dur) => {
                    s.send(view).await.unwrap();
                },
                _ = end_ch.recv() => {},
            };
            cnt.fetch_sub(1, Ordering::SeqCst);
        });
    }

    pub(crate) fn stop_all_timer(&self) {
        if let Err(e) = self.stop_ch.send(()) {
            error!("{}", e.to_string());
        }
    }

    fn num_running_timer(&self) -> u32 {
        self.cnt.load(Ordering::SeqCst)
    }
}

impl Drop for DefaultTimer {
    fn drop(&mut self) {
        self.stop_all_timer();
    }
}

#[test]
#[ignore = "tested"]
fn test_close_all_tiemr() {
    let (notifier, _) = tokio::sync::mpsc::channel(1);
    let max_timeout = 30_000;
    let rtt = 5_000;
    let t = DefaultTimer::new(notifier, max_timeout, rtt);

    let num = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(async move {
            t.start(0, t.timeout_by_delay());
            t.start(1, t.timeout_by_delay());

            tokio::time::sleep(Duration::from_secs(2)).await;
            t.stop_all_timer();
            tokio::time::sleep(Duration::from_secs(2)).await;
            t.num_running_timer()
        });

    assert_eq!(num, 0);
}

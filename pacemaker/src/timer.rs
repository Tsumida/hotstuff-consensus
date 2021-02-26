//! Timer
use crate::pacemaker::{CtlRecvr, CtlSender};

use super::pacemaker::TchanS;
use futures_timer::Delay;
use hs_data::ViewNumber;
use log::error;
use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc,
};
use std::time::Duration;

#[derive(Debug, Clone)]
pub(crate) enum TimeoutEvent {
    ViewTimeout(ViewNumber),
    // Branch synchronizing with internal [a, b]
    // BranchSyncTimeout(ViewNumber, ViewNumber),
}

/// Cancellable timer.
pub(crate) struct DefaultTimer {
    // default 300_000 ms
    max_timeout: u64,
    rtt: u64,
    stop_ch: CtlSender,
    // bcast_recvr: tokio::sync::broadcast::Receiver<()>,
    notifier: TchanS<TimeoutEvent>,
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

    pub(crate) fn new(notifier: TchanS<TimeoutEvent>, max_timeout: u64, rtt: u64) -> Self {
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
    pub(crate) fn start(&self, dur: Duration, te: TimeoutEvent) {
        let mut end_ch = self.stop_ch.subscribe();
        let s = self.notifier.clone();
        let cnt = self.cnt.clone();
        tokio::spawn(async move {
            cnt.fetch_add(1, Ordering::SeqCst);
            tokio::select! {
                () = Delay::new(dur) => {
                    s.send(te).await.unwrap();
                },
                _ = end_ch.recv() => {},
            };
            cnt.fetch_sub(1, Ordering::SeqCst);
        });
    }

    fn stop_view_timer(&self) {
        if let Err(e) = self.stop_ch.send(()) {
            error!("{}", e.to_string());
        }
    }

    /// Timeout right now.
    pub(crate) fn view_timeout(&mut self, this_view: ViewNumber) {
        self.stop_view_timer();
        self.start(
            Duration::from_millis(1),
            TimeoutEvent::ViewTimeout(this_view),
        );
    }
}

impl Drop for DefaultTimer {
    fn drop(&mut self) {
        self.stop_view_timer();
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
            t.start(t.timeout_by_delay(), TimeoutEvent::ViewTimeout(0));
            t.start(t.timeout_by_delay(), TimeoutEvent::ViewTimeout(1));

            tokio::time::sleep(Duration::from_secs(1)).await;
            t.stop_view_timer();
            assert_eq!(t.cnt.load(Ordering::SeqCst), 0);

            t.start(
                t.timeout_by_delay().mul_f64(2.0), // 10 sec
                TimeoutEvent::ViewTimeout(1),
            );

            tokio::time::sleep(Duration::from_secs(1)).await;
            t.cnt.load(Ordering::SeqCst)
        });

    assert_eq!(num, 0);
}

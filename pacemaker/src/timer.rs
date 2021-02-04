//! Timer
use super::pacemaker::TchanS;
use futures_timer::Delay;
use hotstuff_rs::data::ViewNumber;
use log::error;
use std::time::Duration;

/// Cancellable timer.
pub(crate) struct DefaultTimer {
    // default 300_000 ms
    max_timeout: u64,
    rtt: u64,
    stop_ch: tokio::sync::broadcast::Sender<()>,
    // bcast_recvr: tokio::sync::broadcast::Receiver<()>,
    notifier: TchanS<ViewNumber>,
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
        }
    }

    /// Cancel all timers and start new one.
    pub(crate) fn start(&self, view: ViewNumber, dur: Duration) {
        let mut end_ch = self.stop_ch.subscribe();
        let s = self.notifier.clone();
        tokio::spawn(async move {
            tokio::select! {
                () = Delay::new(dur) => {
                    s.send(view).await.unwrap();
                },
                _ = end_ch.recv() => {},
            };
        });
    }

    pub(crate) fn stop_all_prev_timer(&self) {
        if let Err(e) = self.stop_ch.send(()) {
            error!("{}", e.to_string());
        }
    }
}

impl Drop for DefaultTimer {
    fn drop(&mut self) {
        self.stop_all_prev_timer();
    }
}

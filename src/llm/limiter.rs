use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use tokio::time::{Instant, sleep_until};

const WINDOW: Duration = Duration::from_secs(60);

/// Sliding-window limiter: at most `rpm` acquisitions in any 60 s span.
#[derive(Debug)]
pub struct RateLimiter {
    rpm: usize,
    stamps: Mutex<VecDeque<Instant>>,
}

impl RateLimiter {
    pub fn new(rpm: u32) -> Self {
        let rpm = rpm.max(1) as usize;
        Self {
            rpm,
            stamps: Mutex::new(VecDeque::with_capacity(rpm)),
        }
    }

    #[cfg(test)]
    pub(super) fn in_window(&self) -> usize {
        self.stamps.lock().unwrap().len()
    }

    /// Returns the time spent waiting for a slot.
    pub async fn acquire(&self) -> Duration {
        let start = Instant::now();
        let mut slept = false;
        loop {
            let wake = {
                let mut stamps = self.stamps.lock().unwrap_or_else(|e| e.into_inner());
                let now = Instant::now();
                while stamps
                    .front()
                    .is_some_and(|t| now.duration_since(*t) >= WINDOW)
                {
                    stamps.pop_front();
                }
                if stamps.len() < self.rpm {
                    stamps.push_back(now);
                    return if slept {
                        now.duration_since(start)
                    } else {
                        Duration::ZERO
                    };
                }
                stamps[0] + WINDOW
            };
            sleep_until(wake).await;
            slept = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn third_acquire_waits_a_minute() {
        let limiter = RateLimiter::new(2);
        assert_eq!(limiter.acquire().await, Duration::ZERO);
        assert_eq!(limiter.acquire().await, Duration::ZERO);
        let waited = limiter.acquire().await;
        assert!(waited >= Duration::from_secs(59) && waited <= Duration::from_secs(61));
    }

    #[tokio::test(start_paused = true)]
    async fn window_slides() {
        let limiter = RateLimiter::new(1);
        limiter.acquire().await;
        tokio::time::advance(Duration::from_secs(61)).await;
        assert_eq!(limiter.acquire().await, Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn concurrent_acquires_respect_the_cap() {
        let limiter = Arc::new(RateLimiter::new(2));
        let start = Instant::now();
        let tasks: Vec<_> = (0..4)
            .map(|_| {
                let limiter = limiter.clone();
                tokio::spawn(async move {
                    limiter.acquire().await;
                    start.elapsed()
                })
            })
            .collect();
        let mut at = Vec::new();
        for task in tasks {
            at.push(task.await.unwrap());
        }
        at.sort();
        assert!(at[1] < Duration::from_secs(1));
        assert!(at[2] >= Duration::from_secs(59));
        assert!(at[3] >= Duration::from_secs(59));
    }
}

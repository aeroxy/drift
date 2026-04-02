use std::time::Duration;

#[allow(dead_code)]
pub struct ReconnectPolicy {
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub multiplier: f64,
    current_delay: Duration,
}

#[allow(dead_code)]
impl ReconnectPolicy {
    pub fn new() -> Self {
        Self {
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
            multiplier: 2.0,
            current_delay: Duration::from_secs(1),
        }
    }

    pub fn next_delay(&mut self) -> Duration {
        let delay = self.current_delay;
        self.current_delay = Duration::from_secs_f64(
            (self.current_delay.as_secs_f64() * self.multiplier).min(self.max_delay.as_secs_f64()),
        );
        delay
    }

    pub fn reset(&mut self) {
        self.current_delay = self.initial_delay;
    }
}

use tokio::time::{Duration, Instant};

pub struct CounterSeriesConfig {
    /// How many buckets should be maintained.
    pub num_buckets: u8,
    /// How long a time interval each bucket represents,
    /// expressed in seconds.
    pub bucket_size: u64,
}

/// CounterSeries implements a time series stored in a
/// sequence of a fixed number of buckets each with a fixed
/// (and equal) duration.
///
/// The buckets are implemented as a ring buffer held in memory.
/// The counter can be incremented or updated to a new value,
/// but only for the bucket representing the current point
/// in time.
///
/// As time elapses, the current bucket changes based on
/// the bucket duration, with older buckets being zeroed
/// out.  No background maintenance tasks are required
/// to manage this rotation, as the counter series maintains
/// book keeping to fixup the structure prior to accessing
/// the buckets.
///
/// The value tracked in each bucket is a u64, meaning
/// that we cannot track negative numbers.  If you try
/// to delta outside the valid range, the resulting
/// value is saturated to the bounds of a u64; it will
/// never be less than zero and never wrap around due
/// to overflow.
pub struct CounterSeries {
    /// The time series data itself
    buckets: Vec<u64>,
    /// How long a time interval each bucket represents,
    /// expressed in seconds
    bucket_size: u64,
    /// Which slot corresponds to the current time interval
    curr_bucket: u8,
    /// When we last changed curr_bucket
    updated: Instant,
}

impl CounterSeries {
    /// Create a new instance. All buckets will be initialized
    /// to zero.
    pub fn with_config(config: CounterSeriesConfig) -> Self {
        Self::with_initial_value(config, 0)
    }

    /// Create a new instance with a pre-set initial value.
    /// Useful when setting up the initial state for observation
    /// based tracking
    pub fn with_initial_value(config: CounterSeriesConfig, value: u64) -> Self {
        let mut buckets = vec![0u64; config.num_buckets as usize];

        buckets[0] = value;

        Self {
            buckets,
            bucket_size: config.bucket_size,
            curr_bucket: 0,
            updated: Instant::now(),
        }
    }

    /// Manage aging out of older bucket values as time elapses.
    /// The strategy here is: figure out how many bucket slots
    /// we need to advance since the prior operation and zero them
    /// out.  We clip that count to the number of buckets so that
    /// we don't do excess iterations if it has been a very long time
    /// since we last touched this structure.
    fn rotate_and_get_current_bucket(&mut self) -> usize {
        let num_buckets = self.buckets.len() as u64;
        let elapsed_seconds = self.updated.elapsed().as_secs();
        let elapsed_slots = elapsed_seconds / (self.bucket_size as u64);

        if elapsed_slots > 0 {
            let num_prune = elapsed_slots.min(num_buckets) as isize;
            self.curr_bucket = ((elapsed_slots + self.curr_bucket as u64) % num_buckets) as u8;
            // we updated curr_bucket, so revise the updated time
            self.updated = Instant::now();

            for prune in 0..num_prune {
                let mut idx = (self.curr_bucket as isize) - prune;
                if idx < 0 {
                    idx = num_buckets as isize + idx;
                }
                self.buckets[idx as usize] = 0;
            }
        }

        self.curr_bucket as usize
    }

    /// Increment the counter for the current time window by
    /// the specified value.
    pub fn increment(&mut self, to_add: u64) {
        let idx = self.rotate_and_get_current_bucket();
        self.buckets[idx] = self.buckets[idx].saturating_add(to_add);
    }

    /// Adjust the counter for the current time window by the specified value
    pub fn delta(&mut self, delta: i64) {
        let idx = self.rotate_and_get_current_bucket();
        if delta > 0 {
            self.buckets[idx] = self.buckets[idx].saturating_add(delta as u64);
        } else {
            self.buckets[idx] = self.buckets[idx].saturating_sub((-delta) as u64);
        }
    }

    /// Record an observation; assigns current_value to the current bucket
    pub fn observe(&mut self, current_value: u64) {
        let idx = self.rotate_and_get_current_bucket();
        self.buckets[idx] = current_value;
    }

    /// Returns the total tracked over the entire series duration
    pub fn sum(&mut self) -> u64 {
        let _idx = self.rotate_and_get_current_bucket();
        self.buckets.iter().sum()
    }

    /// Returns the total tracked over a specific time duration.
    /// Rounds up to the next bucket for spans smaller than
    /// the bucket size.
    pub fn sum_over(&mut self, duration: Duration) -> u64 {
        let idx = self.rotate_and_get_current_bucket() as isize;
        let buckets_to_sum = (duration.as_secs().div_ceil(self.bucket_size as u64))
            .min(self.buckets.len() as u64)
            .max(1) as isize;

        let mut result = 0;
        for i in 0..buckets_to_sum {
            let mut i = idx - i;
            if i < 0 {
                i = self.buckets.len() as isize + i;
            }
            result += self.buckets[i as usize];
        }

        result
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[derive(Debug, PartialEq)]
    #[allow(dead_code)] // we inspect via Debug, so it is not dead!
    struct Delta<'a> {
        buckets: &'a [u64],
        curr: u8,
        elapsed: Duration,
    }

    fn delta(series: &CounterSeries) -> Delta<'_> {
        Delta {
            buckets: &series.buckets,
            curr: series.curr_bucket,
            elapsed: series.updated.elapsed(),
        }
    }

    #[tokio::test]
    async fn test_delta_observe() {
        let mut series = CounterSeries::with_config(CounterSeriesConfig {
            num_buckets: 5,
            bucket_size: 2,
        });

        series.delta(3);
        series.delta(-2);
        k9::assert_equal!(series.sum(), 1);
        series.observe(42);
        k9::assert_equal!(series.sum(), 42);
    }

    #[tokio::test]
    async fn test_rotation() {
        tokio::time::pause();

        let mut series = CounterSeries::with_config(CounterSeriesConfig {
            num_buckets: 5,
            bucket_size: 2,
        });

        k9::assert_equal!(
            delta(&series),
            Delta {
                buckets: &[0, 0, 0, 0, 0],
                curr: 0,
                elapsed: Duration::ZERO
            }
        );

        k9::assert_equal!(series.sum(), 0);

        series.increment(1);

        k9::assert_equal!(
            delta(&series),
            Delta {
                buckets: &[1, 0, 0, 0, 0],
                curr: 0,
                elapsed: Duration::ZERO
            }
        );
        k9::assert_equal!(series.sum(), 1);

        tokio::time::advance(Duration::from_secs(1)).await;

        series.increment(1);

        k9::assert_equal!(
            delta(&series),
            Delta {
                buckets: &[2, 0, 0, 0, 0],
                curr: 0,
                elapsed: Duration::from_secs(1)
            }
        );
        k9::assert_equal!(series.sum(), 2);

        tokio::time::advance(Duration::from_secs(1)).await;
        series.increment(1);

        k9::assert_equal!(
            delta(&series),
            Delta {
                buckets: &[2, 1, 0, 0, 0],
                curr: 1,
                elapsed: Duration::ZERO
            }
        );
        k9::assert_equal!(series.sum(), 3);

        tokio::time::advance(Duration::from_secs(2)).await;
        series.increment(3);

        k9::assert_equal!(
            delta(&series),
            Delta {
                buckets: &[2, 1, 3, 0, 0],
                curr: 2,
                elapsed: Duration::ZERO
            }
        );
        k9::assert_equal!(series.sum(), 6);

        tokio::time::advance(Duration::from_secs(2)).await;
        series.increment(4);

        k9::assert_equal!(
            delta(&series),
            Delta {
                buckets: &[2, 1, 3, 4, 0],
                curr: 3,
                elapsed: Duration::ZERO
            }
        );
        k9::assert_equal!(series.sum(), 10);

        tokio::time::advance(Duration::from_secs(2)).await;
        series.increment(5);

        k9::assert_equal!(
            delta(&series),
            Delta {
                buckets: &[2, 1, 3, 4, 5],
                curr: 4,
                elapsed: Duration::ZERO
            }
        );
        k9::assert_equal!(series.sum(), 15);

        tokio::time::advance(Duration::from_secs(2)).await;
        series.increment(6);

        k9::assert_equal!(
            delta(&series),
            Delta {
                buckets: &[6, 1, 3, 4, 5],
                curr: 0,
                elapsed: Duration::ZERO
            }
        );
        k9::assert_equal!(series.sum(), 19);

        // Now skip a slot
        tokio::time::advance(Duration::from_secs(4)).await;
        series.increment(7);

        k9::assert_equal!(
            delta(&series),
            Delta {
                buckets: &[6, 0, 7, 4, 5],
                curr: 2,
                elapsed: Duration::ZERO
            }
        );
        k9::assert_equal!(series.sum(), 22);
        k9::assert_equal!(series.sum_over(Duration::ZERO), 7);
        k9::assert_equal!(series.sum_over(Duration::from_secs(1)), 7);
        k9::assert_equal!(series.sum_over(Duration::from_secs(2)), 7);
        k9::assert_equal!(series.sum_over(Duration::from_secs(3)), 7);
        k9::assert_equal!(series.sum_over(Duration::from_secs(4)), 7);
        k9::assert_equal!(series.sum_over(Duration::from_secs(5)), 13);
        k9::assert_equal!(series.sum_over(Duration::from_secs(6)), 13);
        k9::assert_equal!(series.sum_over(Duration::from_secs(7)), 18);
        k9::assert_equal!(series.sum_over(Duration::from_secs(8)), 18);
        k9::assert_equal!(series.sum_over(Duration::from_secs(9)), 22);
        k9::assert_equal!(series.sum_over(Duration::from_secs(10)), 22);
        k9::assert_equal!(series.sum_over(Duration::from_secs(60)), 22);

        // Now skip 6 slots
        tokio::time::advance(Duration::from_secs(12)).await;
        series.increment(8);

        k9::assert_equal!(
            delta(&series),
            Delta {
                buckets: &[0, 0, 0, 8, 0],
                curr: 3,
                elapsed: Duration::ZERO
            }
        );
        k9::assert_equal!(series.sum(), 8);

        for i in 1..=4 {
            tokio::time::advance(Duration::from_secs(2)).await;
            series.increment(i);
        }

        k9::assert_equal!(
            delta(&series),
            Delta {
                buckets: &[2, 3, 4, 8, 1],
                curr: 2,
                elapsed: Duration::ZERO
            }
        );
        k9::assert_equal!(series.sum(), 18);

        tokio::time::advance(Duration::from_secs(60)).await;
        series.observe(0);

        k9::assert_equal!(
            delta(&series),
            Delta {
                buckets: &[0, 0, 0, 0, 0],
                curr: 2,
                elapsed: Duration::ZERO
            }
        );
        k9::assert_equal!(series.sum(), 0);
    }
}

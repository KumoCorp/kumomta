#![cfg(test)]
use super::*;

/// Returns the list of delays up until the max_age would be reached
fn compute_schedule(config: &QueueConfig) -> Vec<i64> {
    let mut schedule = vec![];
    let mut age = 0;
    for attempt in 0.. {
        let delay = config.delay_for_attempt(attempt).num_seconds();
        age += delay;
        if age >= config.max_age.as_secs() as i64 {
            return schedule;
        }
        schedule.push(delay);
    }
    unreachable!()
}

#[test]
fn calc_due() {
    let config = QueueConfig {
        retry_interval: Duration::from_secs(2),
        max_retry_interval: None,
        max_age: Duration::from_secs(1024),
        ..Default::default()
    };

    assert_eq!(
        compute_schedule(&config),
        vec![2, 4, 8, 16, 32, 64, 128, 256, 512]
    );
}

#[test]
fn calc_due_capped() {
    let config = QueueConfig {
        retry_interval: Duration::from_secs(2),
        max_retry_interval: Some(Duration::from_secs(8)),
        max_age: Duration::from_secs(128),
        ..Default::default()
    };

    assert_eq!(
        compute_schedule(&config),
        vec![2, 4, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8]
    );
}

#[test]
fn calc_due_defaults() {
    let config = QueueConfig {
        retry_interval: Duration::from_secs(60 * 20),
        max_retry_interval: None,
        max_age: Duration::from_secs(86400),
        ..Default::default()
    };

    assert_eq!(
        compute_schedule(&config),
        vec![1200, 2400, 4800, 9600, 19200, 38400],
    );
}

#[test]
fn spool_in_delay() {
    let config = QueueConfig {
        retry_interval: Duration::from_secs(2),
        max_retry_interval: None,
        max_age: Duration::from_secs(256),
        ..Default::default()
    };

    let mut schedule = vec![];
    let mut age = 2;
    loop {
        let age_chrono = chrono::Duration::try_seconds(age).expect("age to be in range");
        let num_attempts = config.infer_num_attempts(age_chrono);
        match config.compute_delay_based_on_age(num_attempts, age_chrono) {
            Some(delay) => schedule.push((age, num_attempts, delay.num_seconds())),
            None => break,
        }
        age += 4;
    }

    assert_eq!(
        schedule,
        vec![
            (2, 1, 0),
            (6, 2, 0),
            (10, 2, 0),
            (14, 3, 0),
            (18, 3, 0),
            (22, 3, 0),
            (26, 3, 0),
            (30, 4, 0),
            (34, 4, 0),
            (38, 4, 0),
            (42, 4, 0),
            (46, 4, 0),
            (50, 4, 0),
            (54, 4, 0),
            (58, 4, 0),
            (62, 5, 0),
            (66, 5, 0),
            (70, 5, 0),
            (74, 5, 0),
            (78, 5, 0),
            (82, 5, 0),
            (86, 5, 0),
            (90, 5, 0),
            (94, 5, 0),
            (98, 5, 0),
            (102, 5, 0),
            (106, 5, 0),
            (110, 5, 0),
            (114, 5, 0),
            (118, 5, 0),
            (122, 5, 0),
            (126, 6, 0),
            (130, 6, 0),
            (134, 6, 0),
            (138, 6, 0),
            (142, 6, 0),
            (146, 6, 0),
            (150, 6, 0),
            (154, 6, 0),
            (158, 6, 0),
            (162, 6, 0),
            (166, 6, 0),
            (170, 6, 0),
            (174, 6, 0),
            (178, 6, 0),
            (182, 6, 0),
            (186, 6, 0),
            (190, 6, 0),
            (194, 6, 0),
            (198, 6, 0),
            (202, 6, 0),
            (206, 6, 0),
            (210, 6, 0),
            (214, 6, 0),
            (218, 6, 0),
            (222, 6, 0),
            (226, 6, 0),
            (230, 6, 0),
            (234, 6, 0),
            (238, 6, 0),
            (242, 6, 0),
            (246, 6, 0),
            (250, 6, 0),
            (254, 7, 0)
        ]
    );
}

#[test]
fn bigger_delay() {
    let config = QueueConfig {
        retry_interval: Duration::from_secs(1200),
        max_retry_interval: None,
        max_age: Duration::from_secs(3 * 3600),
        ..Default::default()
    };

    let mut schedule = vec![];
    let mut age = 1200;
    loop {
        let age_chrono = chrono::Duration::try_seconds(age).expect("age to be in range");
        let num_attempts = config.infer_num_attempts(age_chrono);
        match config.compute_delay_based_on_age(num_attempts, age_chrono) {
            Some(delay) => schedule.push((age, num_attempts, delay.num_seconds())),
            None => break,
        }
        age += 1200;
    }

    assert_eq!(
        schedule,
        vec![
            (1200, 1, 0),
            (2400, 1, 0),
            (3600, 2, 0),
            (4800, 2, 0),
            (6000, 2, 0),
            (7200, 2, 0),
            (8400, 3, 0),
            (9600, 3, 0)
        ]
    );
}

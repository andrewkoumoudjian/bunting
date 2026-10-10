#![allow(clippy::unwrap_used)]

use super::*;

const MS: u64 = 1_000;

fn policy(mode: AdmissionMode) -> LatencyPolicy {
    LatencyPolicy {
        mode,
        max_one_way_delay_us: 150 * MS,
        jitter_seed: 7,
        default_path: PathLatency::default(),
        paths: Vec::new(),
    }
}

fn estimator(rtt_us: u64) -> RttEstimator {
    let mut rtt = RttEstimator::new(8);
    rtt.record(rtt_us);
    rtt
}

fn participant(id: u128) -> ParticipantId {
    ParticipantId::new(id)
}

#[test]
fn estimator_uses_the_window_minimum_and_clamps_to_d_max() {
    let mut rtt = RttEstimator::new(4);
    assert_eq!(rtt.one_way_delay_us(150 * MS), 0, "no samples: no credit");
    for sample in [40 * MS, 900 * MS, 30 * MS, 35 * MS] {
        rtt.record(sample);
    }
    assert_eq!(rtt.one_way_delay_us(150 * MS), 15 * MS);
    // The minimum leaves the window after `window` newer samples.
    for _ in 0..4 {
        rtt.record(60 * MS);
    }
    assert_eq!(rtt.sample_count(), 4);
    assert_eq!(rtt.one_way_delay_us(150 * MS), 30 * MS);
    rtt.record(10_000 * MS);
    for _ in 0..3 {
        rtt.record(10_000 * MS);
    }
    assert_eq!(rtt.one_way_delay_us(150 * MS), 150 * MS);
}

#[test]
fn equalized_mode_removes_physical_distance_and_physical_mode_keeps_it() {
    // Two clients send at the same true time; the far one's command
    // arrives 57.5 ms later (120 ms vs 5 ms RTT).
    let near = estimator(5 * MS);
    let far = estimator(120 * MS);
    let send = 1_000 * MS;
    let near_rx = send + 2_500;
    let far_rx = send + 60 * MS;

    let mut equalized = LatencyModel::new(policy(AdmissionMode::Equalized)).unwrap();
    let a = equalized
        .decide(near_rx, &near, participant(1), None)
        .unwrap();
    let b = equalized
        .decide(far_rx, &far, participant(2), None)
        .unwrap();
    assert_eq!(a.release_us, b.release_us);
    assert_eq!(a.release_us, send + 150 * MS);

    // Equal releases are broken by arrival order in the sequencer.
    let mut queue = Sequencer::new(8);
    let a = queue.admit(near_rx, a, "near").unwrap();
    let b = queue.admit(far_rx, b, "far").unwrap();
    assert!(a.arrival_sequence < b.arrival_sequence);
    assert!(queue.pop_due(a.release_us - 1).is_none());
    assert_eq!(queue.pop_due(a.release_us).unwrap().1, "near");
    assert_eq!(queue.pop_due(a.release_us).unwrap().1, "far");

    let mut physical = LatencyModel::new(policy(AdmissionMode::Physical)).unwrap();
    let a = physical
        .decide(near_rx, &near, participant(1), None)
        .unwrap();
    let b = physical.decide(far_rx, &far, participant(2), None).unwrap();
    assert_eq!((a.release_us, b.release_us), (near_rx, far_rx));
    assert_eq!(a.one_way_delay_us + a.max_one_way_delay_us, 0);
}

#[test]
fn geographic_mode_adds_path_latency_and_seeded_bounded_jitter() {
    let mut configured = policy(AdmissionMode::Geographic);
    configured.default_path = PathLatency {
        latency_us: 9 * MS,
        jitter_us: 0,
    };
    configured.paths = vec![
        PathEntry {
            participant_id: participant(1),
            venue_id: Some(VenueId::new(1)),
            path: PathLatency {
                latency_us: 50,
                jitter_us: 0,
            },
        },
        PathEntry {
            participant_id: participant(1),
            venue_id: None,
            path: PathLatency {
                latency_us: 5 * MS,
                jitter_us: 200,
            },
        },
    ];
    let rtt = estimator(20 * MS);
    let mut model = LatencyModel::new(configured.clone()).unwrap();
    let colocated = model
        .decide(0, &rtt, participant(1), Some(VenueId::new(1)))
        .unwrap();
    assert_eq!(colocated.path_latency_us, 50);
    assert_eq!(colocated.jitter_position, None);
    assert_eq!(colocated.release_us, 150 * MS - 10 * MS + 50);
    let unlisted = model.decide(0, &rtt, participant(2), None).unwrap();
    assert_eq!(unlisted.path_latency_us, 9 * MS);

    let draws = |model: &mut LatencyModel| {
        (0..32)
            .map(|_| {
                model
                    .decide(0, &rtt, participant(1), Some(VenueId::new(2)))
                    .unwrap()
            })
            .collect::<Vec<_>>()
    };
    let first = draws(&mut model);
    let replayed = draws(&mut LatencyModel::new(configured.clone()).unwrap());
    assert_eq!(first, replayed, "same seed, same jitter stream");
    assert!(
        first
            .iter()
            .all(|record| (5 * MS..=5 * MS + 200).contains(&record.path_latency_us))
    );
    assert_eq!(
        first
            .iter()
            .map(|record| record.jitter_position)
            .collect::<Vec<_>>(),
        (0..32).map(Some).collect::<Vec<_>>()
    );
    assert!(
        first
            .windows(2)
            .any(|pair| pair[0].path_latency_us != pair[1].path_latency_us)
    );
    let mut reseeded = configured;
    reseeded.jitter_seed = 8;
    assert_ne!(draws(&mut LatencyModel::new(reseeded).unwrap()), first);
}

#[test]
fn the_sequencer_never_releases_early_or_out_of_order_and_is_bounded() {
    let mut model = LatencyModel::new(policy(AdmissionMode::Equalized)).unwrap();
    let mut queue = Sequencer::new(3);
    let slow = model
        .decide(0, &estimator(0), participant(1), None)
        .unwrap();
    let fast = model
        .decide(10, &estimator(300 * MS), participant(2), None)
        .unwrap();
    let slow = queue.admit(10, slow, 1).unwrap();
    let fast = queue.admit(10, fast, 2).unwrap();
    // The far client's command was sent earlier, so it releases first.
    assert!(fast.release_us < slow.release_us);
    assert!(fast.release_us >= fast.received_us);
    assert_eq!(queue.next_release_us(), Some(fast.release_us));
    assert_eq!(queue.pop_due(fast.release_us).unwrap().1, 2);
    assert!(queue.pop_due(slow.release_us - 1).is_none());
    assert_eq!(queue.pop_due(slow.release_us).unwrap().1, 1);

    // A command whose receive stamp predates a release is raised to the
    // insertion time instead of jumping ahead of what already ran.
    let late = model
        .decide(0, &estimator(0), participant(3), None)
        .unwrap();
    let late = queue.admit(slow.release_us + 5, late, 3).unwrap();
    assert_eq!(late.release_us, slow.release_us + 5);
    queue.admit(slow.release_us + 5, late, 4).unwrap();
    queue.admit(slow.release_us + 5, late, 5).unwrap();
    assert_eq!(
        queue.admit(slow.release_us + 5, late, 6),
        Err(AdmissionError::QueueFull { limit: 3 })
    );
    let released =
        std::iter::from_fn(|| queue.pop_due(u64::MAX).map(|(_, item)| item)).collect::<Vec<_>>();
    assert_eq!(released, vec![3, 4, 5]);
    assert!(queue.is_empty());
}

#[test]
fn policies_are_validated_and_round_trip_as_published_json() {
    let mut invalid = policy(AdmissionMode::Equalized);
    invalid.max_one_way_delay_us = MAX_CONFIGURED_DELAY_US + 1;
    assert!(LatencyModel::new(invalid).is_err());
    let mut duplicate = policy(AdmissionMode::Geographic);
    let entry = PathEntry {
        participant_id: participant(1),
        venue_id: None,
        path: PathLatency::default(),
    };
    duplicate.paths = vec![entry, entry];
    assert!(duplicate.validate().is_err());

    let json = r#"{"mode":"geographic","max_one_way_delay_us":25000,"jitter_seed":3,
        "paths":[{"participant_id":1,"venue_id":2,"latency_us":800,"jitter_us":20}]}"#;
    let parsed: LatencyPolicy = serde_json::from_str(json).unwrap();
    parsed.validate().unwrap();
    assert_eq!(parsed.paths[0].path.latency_us, 800);
    assert_eq!(
        serde_json::from_str::<LatencyPolicy>(&serde_json::to_string(&parsed).unwrap()).unwrap(),
        parsed
    );
    let record = AdmissionRecord {
        mode: AdmissionMode::Geographic,
        received_us: 1,
        one_way_delay_us: 2,
        max_one_way_delay_us: 3,
        path_latency_us: 4,
        jitter_position: Some(5),
        release_us: 6,
        arrival_sequence: 7,
    };
    assert_eq!(
        serde_json::from_str::<AdmissionRecord>(&serde_json::to_string(&record).unwrap()).unwrap(),
        record
    );
}

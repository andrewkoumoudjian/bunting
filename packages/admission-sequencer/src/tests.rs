#![allow(clippy::unwrap_used)]

use super::*;

const MS: u64 = 1_000;
const D: u64 = 150 * MS;

fn policy(mode: AdmissionMode) -> LatencyPolicy {
    LatencyPolicy {
        mode,
        max_one_way_delay_us: D,
        jitter_seed: 7,
        default_path: PathLatency::default(),
        paths: Vec::new(),
    }
}

/// A connection measured honestly by both sources.
fn measured(rtt_us: u64) -> DelayEstimator {
    let mut estimator = DelayEstimator::new();
    estimator.record_kernel_min(rtt_us);
    estimator.record_probe(rtt_us);
    estimator
}

fn participant(id: u128) -> ParticipantId {
    ParticipantId::new(id)
}

#[test]
fn the_estimate_is_the_lifetime_minimum_across_sources_and_never_rises() {
    let mut estimator = DelayEstimator::new();
    assert_eq!(estimator.one_way_delay_us(D), 0, "no samples: no credit");
    assert_eq!(estimator.source(), RttSource::None);
    estimator.record_kernel_min(0);
    assert_eq!(estimator.source(), RttSource::None, "kernel 0 = no sample");
    estimator.record_probe(40 * MS);
    assert_eq!(estimator.one_way_delay_us(D), 20 * MS);
    estimator.record_kernel_min(30 * MS);
    assert_eq!(estimator.source(), RttSource::Both);
    assert_eq!(estimator.one_way_delay_us(D), 15 * MS);
    // Later, larger samples (queueing, or deliberate inflation) change
    // nothing: the estimate can only fall.
    let mut previous = estimator.one_way_delay_us(D);
    for sample in [900 * MS, 35 * MS, 400 * MS, 28 * MS, 5_000 * MS] {
        estimator.record_probe(sample);
        estimator.record_kernel_min(sample);
        let now = estimator.one_way_delay_us(D);
        assert!(now <= previous);
        previous = now;
    }
    assert_eq!(previous, 14 * MS);
    assert_eq!(estimator.probe_samples(), 6);
    assert_eq!(measured(10_000 * MS).one_way_delay_us(D), D, "clamped to D");
}

#[test]
fn slow_heartbeat_handling_earns_no_credit_while_the_kernel_is_honest() {
    // True RTT 20 ms. A badly written (or deliberately slow) client answers
    // probes 200 ms late; its OS still ACKs promptly.
    let mut sloppy = DelayEstimator::new();
    sloppy.record_kernel_min(20 * MS);
    for _ in 0..10 {
        sloppy.record_probe(220 * MS);
    }
    let honest = measured(20 * MS);
    assert_eq!(sloppy.one_way_delay_us(D), honest.one_way_delay_us(D));

    // Without kernel RTT the inflation is capped at D − true one-way delay.
    let mut probe_only = DelayEstimator::new();
    probe_only.record_probe(10_000 * MS);
    let gain = probe_only.one_way_delay_us(D) - 10 * MS;
    assert_eq!(gain, D - 10 * MS);
}

/// ADR 0034 validation 2: with distance equalized both ways, the client
/// that processes faster wins, whichever one is far away.
#[test]
fn the_faster_client_wins_regardless_of_distance() {
    let near_one_way = 2_500;
    let far_one_way = 60 * MS;
    let near = measured(2 * near_one_way);
    let far = measured(2 * far_one_way);
    let commit = 1_000 * MS;

    // Returns (near release, far release) for given reaction times.
    let race = |near_reaction: u64, far_reaction: u64| {
        let mut model = LatencyModel::new(policy(AdmissionMode::Equalized)).unwrap();
        let mut leg = |rtt: &DelayEstimator, id: u128, one_way: u64, reaction: u64| {
            let hold = model.outbound_hold_us(rtt, participant(id), None).unwrap();
            let seen = commit + hold + one_way;
            assert_eq!(seen, commit + D, "everyone sees the commit together");
            let received = seen + reaction + one_way;
            model
                .decide(received, rtt, participant(id), None)
                .unwrap()
                .release_us
        };
        let near_release = leg(&near, 1, near_one_way, near_reaction);
        let far_release = leg(&far, 2, far_one_way, far_reaction);
        (near_release, far_release)
    };
    let (near_release, far_release) = race(5 * MS, 4 * MS);
    assert!(far_release < near_release, "far but 1 ms faster wins");
    assert_eq!(near_release - far_release, MS);
    let (near_release, far_release) = race(4 * MS, 5 * MS);
    assert!(near_release < far_release, "near and 1 ms faster wins");
    let (near_release, far_release) = race(4 * MS, 4 * MS);
    assert_eq!(
        near_release, far_release,
        "equal skill ties; arrival breaks it"
    );
    assert_eq!(near_release, commit + 2 * D + 4 * MS);

    // In physical mode distance decides instead.
    let mut physical = LatencyModel::new(policy(AdmissionMode::Physical)).unwrap();
    assert_eq!(
        physical
            .outbound_hold_us(&far, participant(2), None)
            .unwrap(),
        0
    );
    let near_release = physical
        .decide(
            commit + near_one_way + 5 * MS + near_one_way,
            &near,
            participant(1),
            None,
        )
        .unwrap();
    let far_release = physical
        .decide(
            commit + far_one_way + 4 * MS + far_one_way,
            &far,
            participant(2),
            None,
        )
        .unwrap();
    assert!(near_release.release_us < far_release.release_us);
    assert_eq!(
        near_release.one_way_delay_us + near_release.max_one_way_delay_us,
        0
    );
}

#[test]
fn equal_true_send_times_tie_and_arrival_order_breaks_the_tie() {
    let near = measured(5 * MS);
    let far = measured(120 * MS);
    let send = 1_000 * MS;
    let near_rx = send + 2_500;
    let far_rx = send + 60 * MS;
    let mut model = LatencyModel::new(policy(AdmissionMode::Equalized)).unwrap();
    let a = model.decide(near_rx, &near, participant(1), None).unwrap();
    let b = model.decide(far_rx, &far, participant(2), None).unwrap();
    assert_eq!((a.release_us, a.rtt_source), (send + D, RttSource::Both));
    assert_eq!(a.release_us, b.release_us);
    let mut queue = Sequencer::new(8);
    let a = queue.admit(near_rx, a, "near").unwrap();
    let b = queue.admit(far_rx, b, "far").unwrap();
    assert!(a.arrival_sequence < b.arrival_sequence);
    assert!(queue.pop_due(a.release_us - 1).is_none());
    assert_eq!(queue.pop_due(a.release_us).unwrap().1, "near");
    assert_eq!(queue.pop_due(a.release_us).unwrap().1, "far");
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
    let rtt = measured(20 * MS);
    let mut model = LatencyModel::new(configured.clone()).unwrap();
    let colocated = model
        .decide(0, &rtt, participant(1), Some(VenueId::new(1)))
        .unwrap();
    assert_eq!(colocated.path_latency_us, 50);
    assert_eq!(colocated.jitter_position, None);
    assert_eq!(colocated.release_us, D - 10 * MS + 50);
    let unlisted = model.decide(0, &rtt, participant(2), None).unwrap();
    assert_eq!(unlisted.path_latency_us, 9 * MS);

    let draws = |model: &mut LatencyModel, outbound_between: bool| {
        (0..32)
            .map(|_| {
                if outbound_between {
                    model
                        .outbound_hold_us(&rtt, participant(1), Some(VenueId::new(2)))
                        .unwrap();
                }
                model
                    .decide(0, &rtt, participant(1), Some(VenueId::new(2)))
                    .unwrap()
            })
            .collect::<Vec<_>>()
    };
    let first = draws(&mut model, false);
    let replayed = draws(&mut LatencyModel::new(configured.clone()).unwrap(), true);
    assert_eq!(
        first, replayed,
        "same seed, same inbound stream, whatever the outbound traffic"
    );
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
    assert_ne!(
        draws(&mut LatencyModel::new(reseeded).unwrap(), false),
        first
    );
}

#[test]
fn the_sequencer_never_releases_early_or_out_of_order_and_is_bounded() {
    let mut model = LatencyModel::new(policy(AdmissionMode::Equalized)).unwrap();
    let mut queue = Sequencer::new(3);
    let slow = model
        .decide(0, &DelayEstimator::new(), participant(1), None)
        .unwrap();
    let fast = model
        .decide(10, &measured(300 * MS), participant(2), None)
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
        .decide(0, &DelayEstimator::new(), participant(3), None)
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
        rtt_source: RttSource::Kernel,
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

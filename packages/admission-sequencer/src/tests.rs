#![allow(clippy::unwrap_used)]

use super::*;

const MS: u64 = 1_000;

fn participant(id: u128) -> ParticipantId {
    ParticipantId::new(id)
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "commands address an optional venue; the helper keeps call sites short"
)]
fn venue(id: u128) -> Option<VenueId> {
    Some(VenueId::new(id))
}

fn path(team: u128, at: u128, latency_us: u64, jitter_us: u64) -> PathEntry {
    PathEntry {
        participant_id: participant(team),
        venue_id: venue(at),
        path: PathLatency {
            latency_us,
            jitter_us,
        },
    }
}

/// Teams in New York trading Toronto (venue 1) and New Jersey (venue 2):
/// team 1 has a fast desk, team 2 the same desk location.
fn new_york_desks() -> LatencyPolicy {
    LatencyPolicy {
        jitter_seed: 7,
        default_path: PathLatency::default(),
        paths: vec![
            path(1, 1, 4 * MS, 0),
            path(1, 2, 200, 0),
            path(2, 1, 4 * MS, 0),
            path(2, 2, 200, 0),
        ],
    }
}

#[test]
fn the_measured_delay_is_published_but_never_changes_ordering() {
    let mut measured = DelayEstimator::new();
    assert_eq!(measured.one_way_us(), None);
    assert_eq!(measured.source(), RttSource::None);
    measured.record_kernel_min(0);
    assert_eq!(measured.source(), RttSource::None, "kernel 0 = no sample");
    measured.record_probe(40 * MS);
    measured.record_kernel_min(30 * MS);
    measured.record_probe(900 * MS);
    assert_eq!(
        (measured.one_way_us(), measured.source()),
        (Some(15 * MS), RttSource::Both)
    );

    // Two commands received at the same instant release together, whatever
    // the teams' measured access latency: real delay is already inside the
    // receive time, so inflating the measurement buys nothing.
    let mut model = LatencyModel::new(new_york_desks()).unwrap();
    let mut slow_looking = DelayEstimator::new();
    slow_looking.record_probe(500 * MS);
    let fast_looking = DelayEstimator::new();
    let a = model
        .decide(10 * MS, &slow_looking, participant(1), venue(2))
        .unwrap();
    let b = model
        .decide(10 * MS, &fast_looking, participant(2), venue(2))
        .unwrap();
    assert_eq!(a.release_us, b.release_us);
    assert_eq!(a.measured_one_way_us, Some(250 * MS));
    assert_eq!(b.measured_one_way_us, None);
}

#[test]
fn real_delay_and_venue_distance_both_count() {
    let mut model = LatencyModel::new(new_york_desks()).unwrap();
    let measured = DelayEstimator::new();
    // Same true send time; team 2's connection is 3 ms slower (Wi-Fi, a
    // proxy hop, a slow stack): it arrives 3 ms later and stays 3 ms behind.
    let send = 1_000 * MS;
    let team1 = model
        .decide(send + 500, &measured, participant(1), venue(2))
        .unwrap();
    let team2 = model
        .decide(send + 3_500, &measured, participant(2), venue(2))
        .unwrap();
    assert_eq!(team2.release_us - team1.release_us, 3 * MS);
    // The virtual path to the venue adds on top: Toronto is 4 ms away,
    // New Jersey 200 µs.
    assert_eq!(team1.release_us, send + 500 + 200);
    let toronto = model
        .decide(send + 500, &measured, participant(1), venue(1))
        .unwrap();
    assert_eq!(toronto.release_us, send + 500 + 4 * MS);
    // Data comes back over the same virtual path.
    assert_eq!(
        model.outbound_delay_us(participant(1), venue(1)).unwrap(),
        4 * MS
    );
    assert_eq!(
        model.outbound_delay_us(participant(1), venue(2)).unwrap(),
        200
    );
}

#[test]
fn a_nearer_team_wins_at_its_venue_even_when_it_sends_later() {
    let mut policy = new_york_desks();
    // Team 2 sits in Toronto instead.
    policy.paths = vec![
        path(1, 1, 4 * MS, 0),
        path(1, 2, 200, 0),
        path(2, 1, 200, 0),
        path(2, 2, 4 * MS, 0),
    ];
    let mut model = LatencyModel::new(policy).unwrap();
    let measured = DelayEstimator::new();
    let new_york = model
        .decide(0, &measured, participant(1), venue(1))
        .unwrap();
    let toronto = model
        .decide(3 * MS, &measured, participant(2), venue(1))
        .unwrap();
    assert!(toronto.release_us < new_york.release_us);
}

#[test]
fn defaults_apply_per_team_then_globally_and_jitter_is_seeded_and_bounded() {
    let mut configured = new_york_desks();
    configured.default_path = PathLatency {
        latency_us: 9 * MS,
        jitter_us: 0,
    };
    configured.paths.push(PathEntry {
        participant_id: participant(3),
        venue_id: None,
        path: PathLatency {
            latency_us: 5 * MS,
            jitter_us: 200,
        },
    });
    let measured = DelayEstimator::new();
    let mut model = LatencyModel::new(configured.clone()).unwrap();
    assert_eq!(
        model
            .decide(0, &measured, participant(9), venue(1))
            .unwrap()
            .path_latency_us,
        9 * MS
    );
    let draws = |model: &mut LatencyModel, outbound_between: bool| {
        (0..32)
            .map(|_| {
                if outbound_between {
                    model.outbound_delay_us(participant(3), venue(2)).unwrap();
                }
                model
                    .decide(0, &measured, participant(3), venue(2))
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
    let mut model = LatencyModel::new(new_york_desks()).unwrap();
    let measured = DelayEstimator::new();
    let mut queue = Sequencer::new(3);
    let toronto = model
        .decide(0, &measured, participant(1), venue(1))
        .unwrap();
    let jersey = model
        .decide(10, &measured, participant(2), venue(2))
        .unwrap();
    let toronto = queue.admit(10, toronto, 1).unwrap();
    let jersey = queue.admit(10, jersey, 2).unwrap();
    assert!(jersey.release_us < toronto.release_us);
    assert_eq!(queue.next_release_us(), Some(jersey.release_us));
    assert!(queue.pop_due(jersey.release_us - 1).is_none());
    assert_eq!(queue.pop_due(jersey.release_us).unwrap().1, 2);
    assert_eq!(queue.pop_due(toronto.release_us).unwrap().1, 1);

    // A command whose receive stamp predates a release is raised to the
    // insertion time instead of jumping ahead of what already ran.
    let late = model.decide(0, &measured, participant(3), None).unwrap();
    let late = queue.admit(toronto.release_us + 5, late, 3).unwrap();
    assert_eq!(late.release_us, toronto.release_us + 5);
    queue.admit(toronto.release_us + 5, late, 4).unwrap();
    queue.admit(toronto.release_us + 5, late, 5).unwrap();
    assert_eq!(
        queue.admit(toronto.release_us + 5, late, 6),
        Err(AdmissionError::QueueFull { limit: 3 })
    );
    let released =
        std::iter::from_fn(|| queue.pop_due(u64::MAX).map(|(_, item)| item)).collect::<Vec<_>>();
    assert_eq!(released, vec![3, 4, 5]);
    assert!(queue.is_empty());
}

#[test]
fn policies_are_validated_and_round_trip_as_published_json() {
    let mut invalid = new_york_desks();
    invalid.default_path.latency_us = MAX_CONFIGURED_DELAY_US + 1;
    assert!(LatencyModel::new(invalid).is_err());
    let mut duplicate = new_york_desks();
    duplicate.paths.push(path(1, 1, 0, 0));
    assert!(duplicate.validate().is_err());

    let json = r#"{"jitter_seed":3,
        "paths":[{"participant_id":1,"venue_id":2,"latency_us":800,"jitter_us":20}]}"#;
    let parsed: LatencyPolicy = serde_json::from_str(json).unwrap();
    parsed.validate().unwrap();
    assert_eq!(parsed.paths[0].path.latency_us, 800);
    assert_eq!(
        serde_json::from_str::<LatencyPolicy>(&serde_json::to_string(&parsed).unwrap()).unwrap(),
        parsed
    );
    let record = AdmissionRecord {
        received_us: 1,
        measured_one_way_us: Some(2),
        rtt_source: RttSource::Kernel,
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

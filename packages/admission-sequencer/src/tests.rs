#![allow(clippy::unwrap_used)]

use super::*;

const MS: u64 = 1_000;

fn team(id: u128) -> Endpoint {
    Endpoint::Participant(ParticipantId::new(id))
}

fn venue(id: u128) -> Endpoint {
    Endpoint::Venue(VenueId::new(id))
}

fn link(first: &str, second: &str, latency_us: u64, jitter_us: u64) -> Link {
    Link {
        between: [first.to_owned(), second.to_owned()],
        latency: PathLatency {
            latency_us,
            jitter_us,
        },
    }
}

/// Venue 1 in Toronto, venue 2 in Secaucus (New Jersey). Teams 1 and 2 in
/// New York, team 3 in Toronto; the hub in New York.
fn north_america() -> LatencyMap {
    LatencyMap {
        jitter_seed: 7,
        default_location: "new_york".to_owned(),
        participants: vec![ParticipantPlacement {
            participant_id: ParticipantId::new(3),
            location: "toronto".to_owned(),
        }],
        venues: vec![
            VenuePlacement {
                venue_id: VenueId::new(1),
                location: "toronto".to_owned(),
            },
            VenuePlacement {
                venue_id: VenueId::new(2),
                location: "secaucus".to_owned(),
            },
        ],
        local: PathLatency {
            latency_us: 50,
            jitter_us: 0,
        },
        links: vec![
            link("new_york", "toronto", 4 * MS, 0),
            link("new_york", "secaucus", 200, 0),
            link("secaucus", "toronto", 4 * MS, 0),
        ],
    }
}

fn participant(id: u128) -> ParticipantId {
    ParticipantId::new(id)
}

#[test]
fn every_path_comes_from_the_locations_and_links_are_symmetric() {
    let map = north_america();
    map.validate().unwrap();
    assert_eq!(map.location(team(1)), "new_york");
    assert_eq!(map.location(team(3)), "toronto");
    assert_eq!(map.location(Endpoint::Hub), "new_york");
    let path = |from, to| map.path(from, to).latency_us;
    // Team to venue and back.
    assert_eq!(path(team(1), venue(1)), 4 * MS);
    assert_eq!(path(venue(1), team(1)), 4 * MS);
    assert_eq!(path(team(1), venue(2)), 200);
    // Colocated: Toronto team at the Toronto venue.
    assert_eq!(path(team(3), venue(1)), 50);
    // Team to team: same city, and across the border.
    assert_eq!(path(team(1), team(2)), 50);
    assert_eq!(path(team(1), team(3)), 4 * MS);
    assert_eq!(path(team(3), team(1)), 4 * MS);
}

#[test]
fn maps_with_unlinked_locations_or_duplicates_are_refused() {
    let mut unlinked = north_america();
    unlinked.links.pop();
    assert!(unlinked.validate().is_err(), "secaucus-toronto missing");
    let mut duplicate = north_america();
    duplicate.links.push(link("toronto", "new_york", 1, 0));
    assert!(duplicate.validate().is_err());
    let mut self_link = north_america();
    self_link.links.push(link("toronto", "toronto", 1, 0));
    assert!(self_link.validate().is_err());
    let mut too_slow = north_america();
    too_slow.local.latency_us = MAX_CONFIGURED_DELAY_US + 1;
    assert!(LatencyModel::new(too_slow).is_err());
    // An empty map puts everyone at the hub: no virtual distance at all.
    let mut empty = LatencyModel::new(LatencyMap::default()).unwrap();
    assert_eq!(empty.delay_us(team(1), venue(9)).unwrap(), 0);
}

#[test]
fn the_measured_delay_is_published_but_never_changes_ordering() {
    let mut measured = DelayEstimator::new();
    assert_eq!(measured.one_way_us(), None);
    measured.record_kernel_min(0);
    assert_eq!(measured.source(), RttSource::None, "kernel 0 = no sample");
    measured.record_probe(40 * MS);
    measured.record_kernel_min(30 * MS);
    measured.record_probe(900 * MS);
    assert_eq!(
        (measured.one_way_us(), measured.source()),
        (Some(15 * MS), RttSource::Both)
    );
    let mut model = LatencyModel::new(north_america()).unwrap();
    let mut slow_looking = DelayEstimator::new();
    slow_looking.record_probe(500 * MS);
    let a = model
        .decide(10 * MS, &slow_looking, participant(1), venue(2))
        .unwrap();
    let b = model
        .decide(10 * MS, &DelayEstimator::new(), participant(2), venue(2))
        .unwrap();
    assert_eq!(a.release_us, b.release_us);
    assert_eq!(a.measured_one_way_us, Some(250 * MS));
    assert_eq!(a.destination, venue(2));
}

#[test]
fn real_delay_and_virtual_distance_add_up() {
    let mut model = LatencyModel::new(north_america()).unwrap();
    let measured = DelayEstimator::new();
    let send = 1_000 * MS;
    // Same true send time; team 2's connection is 3 ms slower.
    let team1 = model
        .decide(send + 500, &measured, participant(1), venue(2))
        .unwrap();
    let team2 = model
        .decide(send + 3_500, &measured, participant(2), venue(2))
        .unwrap();
    assert_eq!(team2.release_us - team1.release_us, 3 * MS);
    assert_eq!(team1.release_us, send + 500 + 200);
    // The Toronto team beats the New York team at the Toronto venue even
    // when it sends 3 ms later.
    let new_york = model
        .decide(0, &measured, participant(1), venue(1))
        .unwrap();
    let toronto = model
        .decide(3 * MS, &measured, participant(3), venue(1))
        .unwrap();
    assert!(toronto.release_us < new_york.release_us);
    // Requests for the organizer go to the hub.
    let report = model
        .decide(0, &measured, participant(3), Endpoint::Hub)
        .unwrap();
    assert_eq!(report.path_latency_us, 4 * MS);
}

#[test]
fn jitter_is_seeded_bounded_and_separate_per_direction() {
    let mut map = north_america();
    map.links[0].latency.jitter_us = 200;
    let measured = DelayEstimator::new();
    let draws = |model: &mut LatencyModel, reverse_traffic: bool| {
        (0..32)
            .map(|_| {
                if reverse_traffic {
                    model.delay_us(venue(1), team(1)).unwrap();
                    model.delay_us(team(3), team(1)).unwrap();
                }
                model
                    .decide(0, &measured, participant(1), venue(1))
                    .unwrap()
            })
            .collect::<Vec<_>>()
    };
    let first = draws(&mut LatencyModel::new(map.clone()).unwrap(), false);
    let replayed = draws(&mut LatencyModel::new(map.clone()).unwrap(), true);
    assert_eq!(first, replayed, "other directions never shift these draws");
    assert!(
        first
            .iter()
            .all(|record| (4 * MS..=4 * MS + 200).contains(&record.path_latency_us))
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
    let mut reseeded = map;
    reseeded.jitter_seed = 8;
    assert_ne!(
        draws(&mut LatencyModel::new(reseeded).unwrap(), false),
        first
    );
}

#[test]
fn the_sequencer_never_releases_early_or_out_of_order_and_is_bounded() {
    let mut model = LatencyModel::new(north_america()).unwrap();
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
    let late = model
        .decide(0, &measured, participant(2), Endpoint::Hub)
        .unwrap();
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
fn maps_and_records_round_trip_as_published_json() {
    let json = r#"{"default_location":"new_york",
        "participants":[{"participant_id":3,"location":"toronto"}],
        "venues":[{"venue_id":1,"location":"toronto"}],
        "local":{"latency_us":50},
        "links":[{"between":["new_york","toronto"],"latency_us":4000,"jitter_us":20}]}"#;
    let parsed: LatencyMap = serde_json::from_str(json).unwrap();
    parsed.validate().unwrap();
    assert_eq!(parsed.path(team(1), venue(1)).latency_us, 4_000);
    assert_eq!(
        serde_json::from_str::<LatencyMap>(&serde_json::to_string(&parsed).unwrap()).unwrap(),
        parsed
    );
    let record = AdmissionRecord {
        received_us: 1,
        measured_one_way_us: Some(2),
        rtt_source: RttSource::Kernel,
        destination: venue(3),
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

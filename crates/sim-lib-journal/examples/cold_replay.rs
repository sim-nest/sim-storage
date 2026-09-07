use std::{collections::BTreeMap, process::ExitCode, time::Instant};

use sim_kernel::Symbol;
use sim_lib_journal::{JournalEntry, JournalHead, JournalObject, StoredState, replay};

fn main() -> ExitCode {
    let records = std::env::args()
        .nth(1)
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(100_000);
    let mut state = StoredState::default();
    let mut previous = None;
    for sequence in 0..records {
        let object = JournalObject::from_bytes(sequence.to_be_bytes());
        let entry = JournalEntry::new(
            sequence,
            previous.clone(),
            Symbol::qualified("benchmark", "cold-replay-record"),
            vec![object.id.clone()],
        );
        let datum = object.datum().clone();
        state.objects.insert(object.id.clone(), object.bytes);
        state.datums.insert(object.id, datum);
        previous = Some(entry.id.clone());
        state.entries.insert(sequence, entry);
    }
    state.head = state
        .entries
        .last_key_value()
        .map(|(_, entry)| JournalHead {
            sequence: entry.sequence,
            entry: entry.id.clone(),
        });

    // Rebuild the maps to discard construction locality before timing the
    // complete verification and reducer-facing replay materialization.
    state.objects = BTreeMap::from_iter(state.objects);
    state.datums = BTreeMap::from_iter(state.datums);
    state.entries = BTreeMap::from_iter(state.entries);
    let started = Instant::now();
    let replayed = match replay(state) {
        Ok(replay) => replay.count(),
        Err(error) => {
            eprintln!("cold replay failed: {error}");
            return ExitCode::FAILURE;
        }
    };
    let elapsed = started.elapsed();
    if replayed != records as usize {
        eprintln!("cold replay count mismatch: expected {records}, observed {replayed}");
        return ExitCode::FAILURE;
    }
    println!(
        "records={records} elapsed_ns={} elapsed_ms={:.3}",
        elapsed.as_nanos(),
        elapsed.as_secs_f64() * 1_000.0
    );
    ExitCode::SUCCESS
}

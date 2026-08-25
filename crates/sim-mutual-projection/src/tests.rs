use super::*;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Clone)]
struct TestClock(Arc<AtomicU64>);
impl Clock for TestClock {
    fn now(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}
impl TestClock {
    fn set(&self, value: u64) {
        self.0.store(value, Ordering::SeqCst);
    }
}

struct SyntheticKeys(BTreeMap<KeyId, Vec<u8>>);
impl SyntheticKeys {
    fn proof(&self, key: &str, message: &[u8]) -> Vec<u8> {
        let mut hash = Sha256::new();
        hash.update(self.0.get(key).unwrap());
        hash.update(message);
        hash.finalize().to_vec()
    }
}
impl KeyVerifier for SyntheticKeys {
    fn verify(&self, key: &KeyId, message: &[u8], proof: &[u8]) -> bool {
        self.0.get(key).is_some_and(|secret| {
            let mut hash = Sha256::new();
            hash.update(secret);
            hash.update(message);
            hash.finalize().as_slice() == proof
        })
    }
}

fn fixture() -> (
    MutualPorch<SyntheticKeys, TestClock>,
    TestClock,
    Offer,
    Fact,
) {
    let clock = TestClock(Arc::new(AtomicU64::new(10)));
    let keys = SyntheticKeys(BTreeMap::from([
        ("mia-key".into(), b"mia-only".to_vec()),
        ("bo-key".into(), b"bo-only".to_vec()),
    ]));
    let provenance = vec![ProvenanceLink {
        relation: "observed-in".into(),
        content: sim_kernel::ContentId::from_bytes(sim_kernel::Symbol::new("synthetic"), [7; 32]),
    }];
    let fact = Fact::synthetic(
        BTreeMap::from([
            ("chosen".into(), b"porch".to_vec()),
            ("sibling".into(), b"private".to_vec()),
            ("adjacent-claim".into(), b"private".to_vec()),
        ]),
        provenance,
    );
    let offer = Offer::new(
        "Mia".into(),
        "Bo".into(),
        "mia-key".into(),
        "bo-key".into(),
        fact.id.clone(),
        FieldShape::new(["chosen".into()]).unwrap(),
        20,
    );
    (MutualPorch::new(keys, clock.clone()), clock, offer, fact)
}
fn acceptance(
    porch: &MutualPorch<SyntheticKeys, TestClock>,
    offer: &Offer,
    party: &str,
    key: &str,
) -> Acceptance {
    let _ = porch;
    let keys = SyntheticKeys(BTreeMap::from([
        ("mia-key".into(), b"mia-only".to_vec()),
        ("bo-key".into(), b"bo-only".to_vec()),
    ]));
    Acceptance {
        offer: offer.id.clone(),
        party: party.into(),
        key: key.into(),
        proof: keys.proof(key, &offer.acceptance_message()),
    }
}

#[test]
fn two_independent_keys_admit_only_the_fixed_projection() {
    let (porch, _, offer, fact) = fixture();
    porch.invite(offer.clone()).unwrap();
    assert_eq!(
        porch.deliver(&offer.id, &fact),
        Err(PorchError::MissingAcceptance)
    );
    porch
        .accept(acceptance(&porch, &offer, "Mia", "mia-key"))
        .unwrap();
    assert_eq!(
        porch.deliver(&offer.id, &fact),
        Err(PorchError::MissingAcceptance)
    );
    let keys = SyntheticKeys(BTreeMap::from([("mia-key".into(), b"mia-only".to_vec())]));
    let forged = Acceptance {
        offer: offer.id.clone(),
        party: "Bo".into(),
        key: "bo-key".into(),
        proof: keys.proof("mia-key", &offer.acceptance_message()),
    };
    assert_eq!(porch.accept(forged), Err(PorchError::InvalidProof));
    porch
        .accept(acceptance(&porch, &offer, "Bo", "bo-key"))
        .unwrap();
    let receipt = porch.deliver(&offer.id, &fact).unwrap();
    assert_eq!(porch.deliver(&offer.id, &fact).unwrap(), receipt);
    let copy = porch.read(&offer.id).unwrap();
    assert_eq!(
        copy.fields,
        BTreeMap::from([("chosen".into(), b"porch".to_vec())])
    );
    assert_eq!(copy.provenance, fact.provenance);
}

#[test]
fn receiver_cannot_widen_offer_or_substitute_fact() {
    let (porch, _, offer, fact) = fixture();
    porch.invite(offer.clone()).unwrap();
    porch
        .accept(acceptance(&porch, &offer, "Mia", "mia-key"))
        .unwrap();
    porch
        .accept(acceptance(&porch, &offer, "Bo", "bo-key"))
        .unwrap();
    let widened = Offer::new(
        offer.inviter.clone(),
        offer.recipient.clone(),
        offer.inviter_key.clone(),
        offer.recipient_key.clone(),
        fact.id.clone(),
        FieldShape::new(["chosen".into(), "sibling".into()]).unwrap(),
        offer.expires_at,
    );
    assert_eq!(
        porch.deliver(&widened.id, &fact),
        Err(PorchError::UnknownOffer)
    );
    let other = Fact::synthetic(
        BTreeMap::from([("chosen".into(), b"other".to_vec())]),
        vec![],
    );
    assert_eq!(porch.deliver(&offer.id, &other), Err(PorchError::WrongFact));
}

#[test]
fn refusal_silence_expiry_and_revocation_reveal_no_payload() {
    let (porch, clock, offer, _fact) = fixture();
    porch.invite(offer.clone()).unwrap();
    assert_eq!(porch.read(&offer.id), Err(PorchError::MissingAcceptance));
    porch.refuse(&offer.id, "Bo").unwrap();
    assert_eq!(porch.read(&offer.id), Err(PorchError::Inactive));
    let (porch, clock2, offer, fact) = fixture();
    porch.invite(offer.clone()).unwrap();
    porch
        .accept(acceptance(&porch, &offer, "Mia", "mia-key"))
        .unwrap();
    porch
        .accept(acceptance(&porch, &offer, "Bo", "bo-key"))
        .unwrap();
    porch.deliver(&offer.id, &fact).unwrap();
    clock2.set(20);
    assert_eq!(porch.read(&offer.id), Err(PorchError::Inactive));
    assert!(
        matches!(porch.audit().last(), Some(AuditEvent::Ended(Tombstone { reason, .. })) if reason == "expired")
    );
    clock.set(30);
    drop(_fact);
}

#[test]
fn concurrent_revoke_is_idempotent_and_removes_the_copy() {
    let (porch, _, offer, fact) = fixture();
    let porch = Arc::new(porch);
    porch.invite(offer.clone()).unwrap();
    porch
        .accept(acceptance(&porch, &offer, "Mia", "mia-key"))
        .unwrap();
    porch
        .accept(acceptance(&porch, &offer, "Bo", "bo-key"))
        .unwrap();
    porch.deliver(&offer.id, &fact).unwrap();
    let a = Arc::clone(&porch);
    let id = offer.id.clone();
    let left = std::thread::spawn(move || a.revoke(&id, "Mia"));
    let b = Arc::clone(&porch);
    let id = offer.id.clone();
    let right = std::thread::spawn(move || b.revoke(&id, "Bo"));
    assert_eq!(
        left.join().unwrap().unwrap(),
        right.join().unwrap().unwrap()
    );
    assert_eq!(porch.read(&offer.id), Err(PorchError::Inactive));
    assert_eq!(
        porch
            .audit()
            .iter()
            .filter(|event| matches!(event, AuditEvent::Ended(_)))
            .count(),
        1
    );
}

#[test]
fn partial_failure_publishes_nothing_and_independent_restore_redelivers() {
    let (porch, _, offer, fact) = fixture();
    porch.invite(offer.clone()).unwrap();
    porch
        .accept(acceptance(&porch, &offer, "Mia", "mia-key"))
        .unwrap();
    porch
        .accept(acceptance(&porch, &offer, "Bo", "bo-key"))
        .unwrap();
    assert_eq!(
        porch.deliver_with_failure_for_test(&offer.id, &fact),
        Err(PorchError::DeliveryFailed)
    );
    assert_eq!(porch.read(&offer.id), Err(PorchError::MissingAcceptance));
    let receipt = porch.deliver(&offer.id, &fact).unwrap();
    let restored = porch.read(&offer.id).unwrap();
    assert_eq!(receipt.copied, super::model::copied_id(&restored));
}

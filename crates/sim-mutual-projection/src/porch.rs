use crate::model::{copied_id, receipt_id};
use crate::{
    Acceptance, AuditEvent, CopiedFact, Fact, KeyId, Offer, OfferId, PartyId, ProjectionReceipt,
    Tombstone,
};
use std::{
    collections::BTreeMap,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};
use thiserror::Error;

pub trait Clock: Send + Sync {
    fn now(&self) -> u64;
}
pub struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
}

/// Verification boundary for a key held outside the porch and either archive.
pub trait KeyVerifier: Send + Sync {
    fn verify(&self, key: &KeyId, message: &[u8], proof: &[u8]) -> bool;
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PorchError {
    #[error("offer is unknown")]
    UnknownOffer,
    #[error("offer has expired or was revoked")]
    Inactive,
    #[error("acceptance does not match the fixed offer")]
    WrongAcceptance,
    #[error("acceptance proof was not made by its independently held key")]
    InvalidProof,
    #[error("both distinct parties and keys must accept")]
    MissingAcceptance,
    #[error("the source fact does not match the offer")]
    WrongFact,
    #[error("an offered field is absent from the fact: {0}")]
    MissingField(String),
    #[error("delivery failed before publication")]
    DeliveryFailed,
}

#[derive(Default)]
struct State {
    offers: BTreeMap<OfferId, Offer>,
    acceptances: BTreeMap<(OfferId, PartyId), Acceptance>,
    copies: BTreeMap<OfferId, CopiedFact>,
    receipts: BTreeMap<OfferId, ProjectionReceipt>,
    ended: BTreeMap<OfferId, Tombstone>,
    audit: Vec<AuditEvent>,
}
/// A copied-view coordinator. It owns no archive or key provider.
pub struct MutualPorch<V, C> {
    verifier: V,
    clock: C,
    state: Mutex<State>,
}
impl<V: KeyVerifier, C: Clock> MutualPorch<V, C> {
    pub fn new(verifier: V, clock: C) -> Self {
        Self {
            verifier,
            clock,
            state: Mutex::new(State::default()),
        }
    }

    pub fn invite(&self, offer: Offer) -> Result<OfferId, PorchError> {
        if offer.inviter == offer.recipient || offer.inviter_key == offer.recipient_key {
            return Err(PorchError::MissingAcceptance);
        }
        let mut state = self.state.lock().expect("porch lock poisoned");
        state
            .offers
            .entry(offer.id.clone())
            .or_insert_with(|| offer.clone());
        if !state.audit.contains(&AuditEvent::Offered(offer.id.clone())) {
            state.audit.push(AuditEvent::Offered(offer.id.clone()));
        }
        Ok(offer.id)
    }

    pub fn accept(&self, acceptance: Acceptance) -> Result<(), PorchError> {
        let mut state = self.state.lock().expect("porch lock poisoned");
        let offer = state
            .offers
            .get(&acceptance.offer)
            .ok_or(PorchError::UnknownOffer)?
            .clone();
        self.ensure_active(&state, &offer)?;
        let expected_key = if acceptance.party == offer.inviter {
            &offer.inviter_key
        } else if acceptance.party == offer.recipient {
            &offer.recipient_key
        } else {
            return Err(PorchError::WrongAcceptance);
        };
        if &acceptance.key != expected_key {
            return Err(PorchError::WrongAcceptance);
        }
        if !self
            .verifier
            .verify(expected_key, &offer.acceptance_message(), &acceptance.proof)
        {
            return Err(PorchError::InvalidProof);
        }
        state
            .acceptances
            .entry((offer.id.clone(), acceptance.party.clone()))
            .or_insert(acceptance);
        Ok(())
    }

    pub fn refuse(&self, offer: &OfferId, party: &str) -> Result<(), PorchError> {
        let mut state = self.state.lock().expect("porch lock poisoned");
        let invited = state.offers.get(offer).ok_or(PorchError::UnknownOffer)?;
        if party != invited.inviter && party != invited.recipient {
            return Err(PorchError::WrongAcceptance);
        }
        if !state.ended.contains_key(offer) {
            state.audit.push(AuditEvent::Refused(offer.clone()));
        }
        Self::end(&mut state, offer, self.clock.now(), "refused");
        Ok(())
    }

    pub fn deliver(
        &self,
        offer_id: &OfferId,
        fact: &Fact,
    ) -> Result<ProjectionReceipt, PorchError> {
        self.deliver_inner(offer_id, fact, false)
    }
    #[doc(hidden)]
    pub fn deliver_with_failure_for_test(
        &self,
        offer_id: &OfferId,
        fact: &Fact,
    ) -> Result<ProjectionReceipt, PorchError> {
        self.deliver_inner(offer_id, fact, true)
    }
    fn deliver_inner(
        &self,
        offer_id: &OfferId,
        fact: &Fact,
        fail: bool,
    ) -> Result<ProjectionReceipt, PorchError> {
        let mut state = self.state.lock().expect("porch lock poisoned");
        if let Some(receipt) = state.receipts.get(offer_id) {
            return Ok(receipt.clone());
        }
        let offer = state
            .offers
            .get(offer_id)
            .ok_or(PorchError::UnknownOffer)?
            .clone();
        self.ensure_active(&state, &offer)?;
        for party in [&offer.inviter, &offer.recipient] {
            if !state
                .acceptances
                .contains_key(&(offer.id.clone(), party.clone()))
            {
                return Err(PorchError::MissingAcceptance);
            }
        }
        if fact.id != offer.fact {
            return Err(PorchError::WrongFact);
        }
        let mut fields = BTreeMap::new();
        for name in offer.shape.fields() {
            fields.insert(
                name.to_owned(),
                fact.fields
                    .get(name)
                    .cloned()
                    .ok_or_else(|| PorchError::MissingField(name.to_owned()))?,
            );
        }
        let copy = CopiedFact {
            source: fact.id.clone(),
            fields,
            provenance: fact.provenance.clone(),
        };
        if fail {
            return Err(PorchError::DeliveryFailed);
        }
        let at = self.clock.now();
        let copied = copied_id(&copy);
        let receipt = ProjectionReceipt {
            id: receipt_id(&offer.id, &copied, at),
            offer: offer.id.clone(),
            copied,
            admitted_at: at,
        };
        state.copies.insert(offer.id.clone(), copy);
        state.receipts.insert(offer.id.clone(), receipt.clone());
        state.audit.push(AuditEvent::Copied(receipt.clone()));
        Ok(receipt)
    }

    pub fn read(&self, offer: &OfferId) -> Result<CopiedFact, PorchError> {
        let mut state = self.state.lock().expect("porch lock poisoned");
        let invited = state
            .offers
            .get(offer)
            .ok_or(PorchError::UnknownOffer)?
            .clone();
        if self.clock.now() >= invited.expires_at {
            Self::end(&mut state, offer, self.clock.now(), "expired");
            return Err(PorchError::Inactive);
        }
        if state.ended.contains_key(offer) {
            return Err(PorchError::Inactive);
        }
        state
            .copies
            .get(offer)
            .cloned()
            .ok_or(PorchError::MissingAcceptance)
    }

    pub fn revoke(&self, offer: &OfferId, party: &str) -> Result<Tombstone, PorchError> {
        let mut state = self.state.lock().expect("porch lock poisoned");
        let invited = state.offers.get(offer).ok_or(PorchError::UnknownOffer)?;
        if party != invited.inviter && party != invited.recipient {
            return Err(PorchError::WrongAcceptance);
        }
        Self::end(&mut state, offer, self.clock.now(), "revoked");
        Ok(state.ended.get(offer).expect("just ended").clone())
    }
    pub fn audit(&self) -> Vec<AuditEvent> {
        self.state
            .lock()
            .expect("porch lock poisoned")
            .audit
            .clone()
    }
    fn ensure_active(&self, state: &State, offer: &Offer) -> Result<(), PorchError> {
        if self.clock.now() >= offer.expires_at || state.ended.contains_key(&offer.id) {
            Err(PorchError::Inactive)
        } else {
            Ok(())
        }
    }
    fn end(state: &mut State, offer: &OfferId, at: u64, reason: &str) {
        if state.ended.contains_key(offer) {
            return;
        }
        let tombstone = Tombstone {
            offer: offer.clone(),
            receipt: state.receipts.get(offer).map(|r| r.id.clone()),
            ended_at: at,
            reason: reason.into(),
        };
        state.copies.remove(offer);
        state.acceptances.retain(|(id, _), _| id != offer);
        state.ended.insert(offer.clone(), tombstone.clone());
        state.audit.push(AuditEvent::Ended(tombstone));
    }
}

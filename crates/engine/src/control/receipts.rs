//! Retains idempotent control outcomes.
use crate::Session;
use crate::session::wire::{
    ControlError, MAX_RECEIPT_BYTES, MAX_RECEIPTS, RECEIPT_RESERVATION, Receipt, Request, Response,
};
use serde_json::json;
pub(super) fn control_fingerprint(
    request: &Request,
) -> std::result::Result<blake3::Hash, ControlError> {
    Ok(blake3::hash(
        &serde_json::to_vec(&json!({"command":request.command,"params":request.params,
            "base_rev":request.base_rev,"api_version":request.api_version,"registry_hash":request.registry_hash,"session_id":request.session_id}))
        .map_err(|_| ControlError::InvalidParams)?,
    ))
}

impl Session {
    pub(super) fn check_receipt_capacity(&self) -> std::result::Result<(), ControlError> {
        if self.ledger.receipts.len() >= MAX_RECEIPTS
            || self
                .ledger
                .receipt_bytes
                .saturating_add(RECEIPT_RESERVATION)
                > MAX_RECEIPT_BYTES
        {
            Err(ControlError::ReceiptCapacity)
        } else {
            Ok(())
        }
    }
}

impl Session {
    pub(super) fn replay_receipt(
        &self,
        key: &(String, String),
        fingerprint: blake3::Hash,
        request_id: &str,
    ) -> std::result::Result<Option<Response>, ControlError> {
        let Some(receipt) = self.ledger.receipts.get(key) else {
            return Ok(None);
        };
        if receipt.fingerprint != fingerprint {
            return Err(ControlError::IdempotencyConflict);
        }
        let mut response = receipt.response.clone();
        response.request_id = request_id.to_owned();
        Ok(Some(response))
    }
}

impl Session {
    pub(super) fn store_receipt(
        &mut self,
        key: (String, String),
        fingerprint: blake3::Hash,
        response: &Response,
    ) -> std::result::Result<(), ControlError> {
        let bytes = serde_json::to_vec(response)
            .map_err(|_| ControlError::InvalidParams)?
            .len()
            + key.0.len()
            + key.1.len()
            + 32;
        self.ledger.receipt_bytes += bytes;
        self.ledger.receipts.insert(
            key,
            Receipt {
                fingerprint,
                response: response.clone(),
            },
        );
        Ok(())
    }
}

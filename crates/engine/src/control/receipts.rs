//! Retains idempotent control outcomes.
use super::{
    EditError, MAX_RECEIPT_BYTES, MAX_RECEIPTS, RECEIPT_RESERVATION, Receipt, Request, Response,
    Result, Session, json,
};
pub(super) fn control_fingerprint(request: &Request) -> Result<blake3::Hash> {
    Ok(blake3::hash(
        &serde_json::to_vec(&json!({"command":request.command,"params":request.params,
            "base_rev":request.base_rev,"api_version":request.api_version,"registry_hash":request.registry_hash,"session_id":request.session_id}))
        .map_err(|_| EditError::InvalidParams)?,
    ))
}

impl Session {
    pub(super) fn check_receipt_capacity(&self) -> Result<()> {
        if self.control.receipts.len() >= MAX_RECEIPTS
            || self
                .control
                .receipt_bytes
                .saturating_add(RECEIPT_RESERVATION)
                > MAX_RECEIPT_BYTES
        {
            Err(EditError::ReceiptCapacity)
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
    ) -> Result<Option<Response>> {
        let Some(receipt) = self.control.receipts.get(key) else {
            return Ok(None);
        };
        if receipt.fingerprint != fingerprint {
            return Err(EditError::IdempotencyConflict);
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
    ) -> Result<()> {
        let bytes = serde_json::to_vec(response)
            .map_err(|_| EditError::InvalidParams)?
            .len()
            + key.0.len()
            + key.1.len()
            + 32;
        self.control.receipt_bytes += bytes;
        self.control.receipts.insert(
            key,
            Receipt {
                fingerprint,
                response: response.clone(),
            },
        );
        Ok(())
    }
}
